//! Restores wallets with on-chain history through the Cloud Backup actions the restore buttons
//! dispatch, then checks that each restored wallet's first scan is a full scan that recovers the
//! history the original wallet had

use std::collections::BTreeMap;
use std::time::Duration;

use bdk_wallet::KeychainKind;
use bitcoin::{Amount, Network as BitcoinNetwork, OutPoint, ScriptBuf};
use cove_types::psbt::Psbt;

use super::*;
use crate::database::test_support::RestoreWalletTarget;
use crate::manager::cloud_backup_manager::CloudOnlyState;
use crate::manager::cloud_backup_manager::wallets::decode_cloud_labels_jsonl;
use crate::manager::deferred_sender::SingleOrMany;
use crate::manager::wallet_manager::{
    WalletManagerReconcileMessage, WalletScanPhase, WalletScanStatus,
};
use crate::network::Network;
use crate::node::Node;
use crate::test_support::fake_node::FakeNode;
use crate::transaction::Transaction;
use crate::wallet::metadata::WalletId;
use crate::wallet_lifecycle::ShutdownDeadlineTier;

const ABANDON_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

const LEGAL_WINNER_MNEMONIC: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

// past the 30-address gap an incremental sync stops at, so only a full scan reaches it
const FAR_RECEIVE_INDEX: u32 = 40;

const FAR_RECEIVE_LABEL: &str = "far receive";

const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// A fake node selected as the wallets' Bitcoin node, which puts back the network, wallet mode
/// and Bitcoin node it found when it drops, since later tests share these settings
struct SelectedFakeNode {
    node: FakeNode,
    previous_bitcoin_node: Node,

    // dropped after `drop` restores the node, which restores the network and mode
    _previous_target: RestoreWalletTarget,
}

impl SelectedFakeNode {
    fn start() -> Self {
        let config = &Database::global().global_config;
        let previous_target = RestoreWalletTarget::capture();
        let previous_bitcoin_node = config.selected_node_for_network(Network::Bitcoin);
        config.set_selected_network(Network::Bitcoin).expect("select the Bitcoin network");

        let node = FakeNode::start(BitcoinNetwork::Bitcoin);
        node.chain().mine(5);
        node.select_esplora();

        Self { node, previous_bitcoin_node, _previous_target: previous_target }
    }
}

impl Drop for SelectedFakeNode {
    fn drop(&mut self) {
        let _ = Database::global().global_config.set_selected_node(&self.previous_bitcoin_node);
    }
}

/// What a wallet holds once its scan settles: the transaction list and balance the wallet
/// screen shows, the coins it can spend, and the last used address on each chain, which decides
/// the receive and change addresses it hands out next
#[derive(Debug, PartialEq, Eq)]
struct WalletHistory {
    // the transaction list without each pending transaction's last-seen time, which is when this
    // device saw it rather than part of the wallet's history
    transactions: Vec<HistoryEntry>,
    balance: bdk_wallet::Balance,
    unspent: BTreeMap<OutPoint, u64>,
    last_used_indices: BTreeMap<KeychainKind, u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum HistoryEntry {
    Confirmed {
        txid: bitcoin::Txid,
        block_height: u32,
        confirmed_at: jiff::Timestamp,
        sent_and_received: crate::transaction::SentAndReceived,
        labels: bip329::Labels,
    },
    Pending {
        txid: bitcoin::Txid,
        sent_and_received: crate::transaction::SentAndReceived,
        labels: bip329::Labels,
    },
}

impl From<&Transaction> for HistoryEntry {
    fn from(transaction: &Transaction) -> Self {
        match transaction {
            Transaction::Confirmed(confirmed) => Self::Confirmed {
                txid: confirmed.txid.0,
                block_height: confirmed.block_height,
                confirmed_at: confirmed.confirmed_at,
                sent_and_received: confirmed.sent_and_received,
                labels: confirmed.labels.clone(),
            },

            Transaction::Unconfirmed(pending) => Self::Pending {
                txid: pending.txid.0,
                sent_and_received: pending.sent_and_received,
                labels: pending.labels.clone(),
            },
        }
    }
}

impl WalletHistory {
    fn labels_of(&self, wanted: bitcoin::Txid) -> bip329::Labels {
        self.transactions
            .iter()
            .find_map(|entry| match entry {
                HistoryEntry::Confirmed { txid, labels, .. }
                | HistoryEntry::Pending { txid, labels, .. } => {
                    (*txid == wanted).then(|| labels.clone())
                }
            })
            .unwrap_or_default()
    }

    async fn read(manager: &RustWalletManager) -> Self {
        let mut transactions = call!(manager.actor.transactions())
            .await
            .expect("the wallet actor answers")
            .iter()
            .map(HistoryEntry::from)
            .collect::<Vec<_>>();

        transactions.sort_by_key(|entry| match entry {
            HistoryEntry::Confirmed { txid, .. } | HistoryEntry::Pending { txid, .. } => *txid,
        });

        let balance = manager.balance().await.expect("the wallet has a balance").0;

        // the scan persists what it found, which is what the next launch loads
        let metadata = manager.wallet_metadata();
        let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, metadata.network)
            .expect("the wallet's store opens");
        let bdk = bdk_wallet::Wallet::load()
            .load_wallet(&mut store.conn)
            .expect("the wallet's store loads")
            .expect("the wallet's store holds a wallet");

        let unspent = bdk
            .list_unspent()
            .map(|output| (output.outpoint, output.txout.value.to_sat()))
            .collect();
        let last_used_indices = bdk.spk_index().last_used_indices();

        Self { transactions, balance, unspent, last_used_indices }
    }
}

/// Reconcile messages a wallet manager sent the screen that shows it
struct ScreenUpdates {
    receiver: Arc<flume::Receiver<SingleOrMany<WalletManagerReconcileMessage>>>,
    seen: Vec<WalletManagerReconcileMessage>,
}

impl ScreenUpdates {
    fn listen(manager: &RustWalletManager) -> Self {
        let receiver = manager.reconciler.receiver();
        receiver.drain().for_each(drop);

        Self { receiver, seen: Vec::new() }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&WalletManagerReconcileMessage) -> bool) {
        tokio::time::timeout(SCAN_TIMEOUT, async {
            while !self.seen.iter().any(&done) {
                match self.receiver.recv_async().await.expect("the reconciler stays open") {
                    SingleOrMany::Single(message) => self.seen.push(message),
                    SingleOrMany::Many(messages) => self.seen.extend(messages),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// Waits for a scan to finish and the scan status to settle back to idle after it
    async fn until_scan_settles(&mut self) {
        self.seen.clear();
        self.until("the scan to complete", |message| {
            matches!(message, WalletManagerReconcileMessage::ScanComplete(_))
        })
        .await;

        self.until("the scan status to go idle", |message| {
            matches!(
                message,
                WalletManagerReconcileMessage::WalletScanStatusChanged(WalletScanStatus::Idle)
            )
        })
        .await;
    }

    fn reported_full_scan_progress(&self) -> bool {
        self.seen.iter().any(|message| {
            matches!(
                message,
                WalletManagerReconcileMessage::WalletScanStatusChanged(WalletScanStatus::Scanning(
                    progress
                )) if progress.phase == WalletScanPhase::Full
            )
        })
    }
}

/// A hot wallet the user created while Cloud Backup was on, used until it has a history
struct OriginalWallet {
    id: WalletId,
    record_id: String,
    far_receive: ScriptBuf,
    history: WalletHistory,
}

impl OriginalWallet {
    /// Creates the wallet the way importing recovery words does, gives it confirmed and pending
    /// receives, a confirmed send with change and a label, and waits for its backup to carry
    /// every label its history shows
    async fn create_and_use(
        node: &FakeNode,
        globals: &TestGlobals,
        name: &str,
        words: &str,
    ) -> Self {
        let mut metadata = WalletMetadata::preview_new();
        metadata.name = name.into();
        metadata.network = Network::Bitcoin;

        // the created wallet closes its store before the wallet manager opens it again
        let (id, [first_receive, far_receive, pending_receive]) = {
            let wallet = Wallet::try_new_persisted_and_selected(
                metadata,
                Mnemonic::parse(words).expect("valid test mnemonic"),
                None,
            )
            .expect("the wallet is created");

            let receive =
                |index| wallet.bdk.peek_address(KeychainKind::External, index).script_pubkey();

            (wallet.id(), [receive(0), receive(FAR_RECEIVE_INDEX), receive(2)])
        };

        let record_id = wallet_record_id(id.as_ref());

        let first_txid = node.chain().pay(&first_receive, Amount::from_sat(120_000));
        node.chain().confirm(first_txid);
        let far_txid = node.chain().pay(&far_receive, Amount::from_sat(80_000));
        node.chain().confirm(far_txid);
        node.chain().mine(2);

        let manager = RustWalletManager::try_new(id.clone()).expect("the wallet opens");
        let mut screen = ScreenUpdates::listen(&manager);
        manager.start_wallet_scan().await.unwrap();
        screen.until_scan_settles().await;

        // spend from the wallet through the hot wallet send path, leaving change on chain
        let psbt = call!(manager.actor.build_tx(
            Amount::from_sat(50_000),
            crate::wallet::Address::preview_new(),
            bitcoin::FeeRate::from_sat_per_vb(2).expect("fee rate")
        ))
        .await
        .expect("the wallet actor answers")
        .expect("the send builds");

        let send_txid = psbt.unsigned_tx.compute_txid();
        manager.initiate_payment(Arc::new(Psbt(psbt)), None).await.expect("the send broadcasts");
        screen.until_scan_settles().await;
        node.chain().confirm(send_txid);

        let pending_txid = node.chain().pay(&pending_receive, Amount::from_sat(30_000));
        manager.force_wallet_scan().await.unwrap();
        screen.until_scan_settles().await;

        let far_details = manager
            .transaction_details(Arc::new(far_txid.into()))
            .await
            .expect("the far receive has details")
            .details();

        manager
            .label_manager()
            .insert_or_update_labels_for_txn(far_details, FAR_RECEIVE_LABEL.into(), None)
            .expect("the label saves");

        let history = WalletHistory::read(&manager).await;
        let txids = history
            .transactions
            .iter()
            .map(|entry| match entry {
                HistoryEntry::Confirmed { txid, .. } | HistoryEntry::Pending { txid, .. } => *txid,
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            txids,
            [first_txid, far_txid, send_txid, pending_txid].into_iter().collect(),
            "the original wallet must hold every transaction the test made"
        );

        assert!(
            history.last_used_indices.contains_key(&KeychainKind::Internal),
            "the send must have left change on the wallet's change chain"
        );

        assert!(
            history
                .labels_of(far_txid)
                .iter()
                .any(|label| label.label() == Some(FAR_RECEIVE_LABEL)),
            "the original wallet's list must show the far receive's label"
        );

        wait_for_backup_with_labels(globals, &record_id, &history).await;

        Self { id, record_id, far_receive, history }
    }

    /// Deletes the wallet from this device the way the delete button does, leaving its backup
    async fn delete_locally(&self) {
        crate::app::delete_wallet_with_tier(self.id.clone(), ShutdownDeadlineTier::Initial, None)
            .await
            .expect("the wallet deletes");

        assert!(
            Database::global()
                .wallets()
                .get(&self.id, Network::Bitcoin, WalletMode::Main)
                .unwrap()
                .is_none()
        );
    }

    /// Opens the restored wallet, runs the scan opening it starts and checks it against the
    /// original
    async fn assert_restored_history_matches(&self, node: &FakeNode) {
        let restored = Database::global()
            .wallets()
            .get(&self.id, Network::Bitcoin, WalletMode::Main)
            .unwrap()
            .expect("the restore saved the wallet");

        // the source device's scan state never comes back, so nothing can route around a full scan
        assert_eq!(restored.internal.performed_full_scan_at, None);
        assert_eq!(restored.internal.last_scan_finished, None);
        assert_eq!(restored.internal.address_index, None);

        node.clear_requests();
        let manager =
            RustWalletManager::try_new(self.id.clone()).expect("the restored wallet opens");
        let mut screen = ScreenUpdates::listen(&manager);

        // the wallet screen asks for a scan when it appears
        manager.start_wallet_scan().await.unwrap();
        screen.until_scan_settles().await;

        assert!(
            screen.reported_full_scan_progress(),
            "the restored wallet's first scan must be a full scan"
        );

        assert!(
            !node.history_requests_for(&self.far_receive).is_empty(),
            "the restored wallet's scan must have looked up the address at index {FAR_RECEIVE_INDEX}"
        );

        assert!(
            manager.wallet_metadata().internal.performed_full_scan_at.is_some(),
            "finishing the full scan must be recorded"
        );

        assert_eq!(WalletHistory::read(&manager).await, self.history);
    }
}

/// Waits until the wallet's uploaded backup carries every label the history shows
///
/// Labeling a transaction saves the transaction label and its output labels in separate writes,
/// so an upload already running can carry the first without the rest; waiting for the whole
/// history keeps the restore from comparing against a backup that a later upload completes
async fn wait_for_backup_with_labels(
    globals: &TestGlobals,
    record_id: &str,
    history: &WalletHistory,
) {
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();

    let uploaded_labels = || {
        let encrypted = globals.cloud.wallet_backup_bytes(&namespace, record_id)?;
        let uploaded = cove_cspp::wallet_crypto::decrypt_wallet_backup(
            &serde_json::from_slice(&encrypted).ok()?,
            &master_key.critical_data_key(),
        )
        .ok()?;

        let jsonl = decode_cloud_labels_jsonl(&uploaded).ok().flatten()?;
        bip329::Labels::try_from_str(&jsonl).ok()
    };

    let wanted = history
        .transactions
        .iter()
        .flat_map(|entry| match entry {
            HistoryEntry::Confirmed { labels, .. } | HistoryEntry::Pending { labels, .. } => {
                labels.iter()
            }
        })
        .collect::<Vec<_>>();

    wait_for_test_condition(Duration::from_secs(10), "the backup carries every label", || {
        uploaded_labels()
            .is_some_and(|uploaded| wanted.iter().all(|label| uploaded.contains(label)))
    })
    .await;
}

/// Enables Cloud Backup on the manager the app dispatches to, with a passkey that verifies the
/// backup and a cloud that lists every wallet backup this device uploads
fn enable_cloud_backup_with_passkey(manager: &RustCloudBackupManager, globals: &TestGlobals) {
    configure_enabled_cloud_backup(manager, globals, 0);
    seed_verifiable_cloud_master_key(globals);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
}

/// Opens Cloud Backup detail and waits for every given backup to show as not on this device
///
/// Adding a wallet asks for the backup to be verified again, so entering the detail verifies it
/// with the passkey before it lists the inventory
async fn open_detail_with_cloud_only(manager: &RustCloudBackupManager, record_ids: &[&str]) {
    manager.dispatch(CloudBackupManagerAction::EnterDetail);

    wait_for_test_condition(Duration::from_secs(10), "the cloud-only rows load", || {
        let inventory = manager.inventory.read();
        let CloudOnlyState::Loaded { wallets, .. } = inventory.cloud_only() else { return false };

        // read through the held guard, since a second read can wait behind a queued writer forever
        inventory.detail_inventory_is_ready()
            && record_ids.iter().all(|record_id| {
                wallets.iter().any(|wallet| {
                    wallet.record_id == *record_id
                        && wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice
                })
            })
    })
    .await;
}

async fn wait_for_restore_to_finish(manager: &RustCloudBackupManager, wallets: &[&OriginalWallet]) {
    wait_for_test_condition(Duration::from_secs(20), "the restore finishes", || {
        manager.projected_exclusive_operation().is_none()
            && wallets.iter().all(|wallet| {
                Database::global()
                    .wallets()
                    .get(&wallet.id, Network::Bitcoin, WalletMode::Main)
                    .unwrap()
                    .is_some()
            })
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn restored_wallet_full_scan_recovers_the_original_history() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = global_manager();
    enable_cloud_backup_with_passkey(&manager, globals);
    let node = SelectedFakeNode::start();

    let original =
        OriginalWallet::create_and_use(&node.node, globals, "Restored History", ABANDON_MNEMONIC)
            .await;
    original.delete_locally().await;

    open_detail_with_cloud_only(&manager, &[&original.record_id]).await;
    manager
        .dispatch(CloudBackupManagerAction::RestoreCloudWallet(original.record_id.clone().into()));
    wait_for_restore_to_finish(&manager, &[&original]).await;

    original.assert_restored_history_matches(&node.node).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_full_scans_recover_each_original_history() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = global_manager();
    enable_cloud_backup_with_passkey(&manager, globals);
    let node = SelectedFakeNode::start();

    let first =
        OriginalWallet::create_and_use(&node.node, globals, "Restore All One", ABANDON_MNEMONIC)
            .await;
    let second = OriginalWallet::create_and_use(
        &node.node,
        globals,
        "Restore All Two",
        LEGAL_WINNER_MNEMONIC,
    )
    .await;
    first.delete_locally().await;
    second.delete_locally().await;

    open_detail_with_cloud_only(&manager, &[&first.record_id, &second.record_id]).await;
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { .. }
    ));

    manager.dispatch(CloudBackupManagerAction::StartRestoreAll);
    wait_for_restore_to_finish(&manager, &[&first, &second]).await;

    first.assert_restored_history_matches(&node.node).await;
    second.assert_restored_history_matches(&node.node).await;
}
