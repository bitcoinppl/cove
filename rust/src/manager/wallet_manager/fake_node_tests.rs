//! Drives a persisted wallet's manager against the fake node, through the same manager, label
//! and coin control calls the wallet screens make: saving a receipt label, locking and unlocking
//! its output, opening transaction details during a refresh, and importing a hardware wallet's
//! public export by paste

use std::{str::FromStr as _, sync::Arc, time::Duration};

use act_zero::call;
use bip39::Mnemonic;
use bitcoin::hashes::{Hash as _, sha256};
use bitcoin::{
    Amount, CompressedPublicKey, Network as BitcoinNetwork, ScriptBuf, Txid,
    bip32::{DerivationPath, Xpriv},
    secp256k1::Secp256k1,
};
use cove_fake_node::{Gate, RuleHandle};
use cove_types::BlockSizeLast;
use rand::RngExt as _;

use super::{
    RustWalletManager, TransactionLockState, WalletLedgerState,
    WalletManagerReconcileMessage as Msg,
};
use crate::{
    database::{Database, test_support::RestoreSignetWallets},
    discovery_scanner::ScannerResponse,
    label_manager::LabelManager,
    manager::{
        deferred_sender::SingleOrMany, import_wallet_manager::import_mnemonic_with_target,
        wallet_manager::WalletScanStatus,
    },
    mnemonic::MnemonicExt as _,
    network::Network,
    node::Node,
    test_support::fake_node::{FakeNode, wait_until},
    transaction::{Transaction, TxId, ffi::BitcoinTransaction},
    wallet::{
        Wallet, WalletAddressType,
        metadata::{WalletId, WalletMode, WalletType},
    },
};

mod fee_bump;

/// A fake node selected for signet, which puts back the node, wallet lists, selections, network
/// and wallet mode it changed when it drops, since later tests share these settings
struct Fixture {
    node: FakeNode,
    network: Network,
    previous_node: Node,
    previous_block_height: Option<BlockSizeLast>,
    wallet_id: Option<WalletId>,

    // dropped after `drop` restores the node
    _wallets: RestoreSignetWallets,
}

impl Fixture {
    /// Starts from empty signet wallet lists in main mode, with the fake node selected
    fn signet() -> Self {
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        let wallets = RestoreSignetWallets::empty();

        let network = Network::Signet;
        let previous_node = Database::global().global_config.selected_node_for_network(network);
        let node = FakeNode::start(network.into());
        node.chain().mine(5);
        node.select_esplora();

        let previous_block_height =
            Database::global().global_cache.get_block_height(network).ok().flatten();

        Self {
            node,
            network,
            previous_node,
            previous_block_height,
            wallet_id: None,
            _wallets: wallets,
        }
    }

    /// Makes the fixture remove `id`'s wallet data when it drops
    fn own(&mut self, id: &WalletId) {
        self.wallet_id = Some(id.clone());
    }

    /// Opens the wallet's manager as the wallet screen does, and owns the wallet's cleanup
    fn open(&mut self, id: WalletId) -> (RustWalletManager, ScreenUpdates) {
        self.own(&id);
        let manager = RustWalletManager::try_new(id).expect("the wallet manager opens");
        let screen = ScreenUpdates::listen(&manager);

        (manager, screen)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(id) = &self.wallet_id {
            crate::wallet::test_support::remove_wallet(id);
        }

        let database = Database::global();
        let _ = database.global_config.set_selected_node(&self.previous_node);

        // confirmations cache the fake chain's height, which later wallets would take as a birthday
        database.global_cache.restore_block_height(self.network, self.previous_block_height);
    }
}

/// A mnemonic nobody has used, so its addresses have no history anywhere
fn fresh_mnemonic() -> Mnemonic {
    Mnemonic::from_entropy(&rand::rng().random::<[u8; 16]>()).expect("16 bytes of entropy")
}

/// Reconcile messages the manager sent the screen that shows it
struct ScreenUpdates {
    receiver: Arc<flume::Receiver<SingleOrMany<Msg>>>,
    seen: Vec<Msg>,
}

impl ScreenUpdates {
    fn listen(manager: &RustWalletManager) -> Self {
        Self { receiver: manager.reconciler.receiver(), seen: Vec::new() }
    }

    /// Waits for a message matching `done` among those seen since the screen was last cleared,
    /// returning its position in `seen`
    async fn until(&mut self, what: &str, done: impl Fn(&Msg) -> bool) -> usize {
        self.until_after(0, what, done).await
    }

    /// Waits for a message matching `done` at or after position `start` in `seen`, returning its
    /// position
    async fn until_after(
        &mut self,
        start: usize,
        what: &str,
        done: impl Fn(&Msg) -> bool,
    ) -> usize {
        let timeout = crate::test_support::loaded_host_wait(Duration::from_secs(20));
        tokio::time::timeout(timeout, async {
            loop {
                if let Some(found) = self.seen.iter().skip(start).position(&done) {
                    return start + found;
                }

                match self.receiver.recv_async().await.expect("the reconciler stays open") {
                    SingleOrMany::Single(message) => self.seen.push(message),
                    SingleOrMany::Many(messages) => self.seen.extend(messages),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {:#?}", self.seen))
    }

    /// Waits for a message matching `done`, then for the scan status to go idle after it, so an
    /// idle status from before that message does not count
    async fn until_then_idle(&mut self, what: &str, done: impl Fn(&Msg) -> bool) {
        let found = self.until(what, done).await;
        self.until_after(found + 1, "the scan status to go idle", |message| {
            matches!(message, Msg::WalletScanStatusChanged(WalletScanStatus::Idle))
        })
        .await;
    }

    /// Waits for a scan to complete and the scan status to settle back to idle after it
    async fn until_scan_settles(&mut self) {
        self.seen.clear();
        self.until_then_idle("the scan to complete", |message| {
            matches!(message, Msg::ScanComplete(_))
        })
        .await;
    }
}

fn tx_id(txid: Txid) -> Arc<TxId> {
    Arc::new(TxId::from(txid))
}

async fn receive_script(manager: &RustWalletManager, index: u32) -> ScriptBuf {
    let info = manager.address_at(index).await.expect("the wallet derives the address");

    bitcoin::Address::from_str(&info.address.to_string())
        .expect("the wallet's address parses")
        .assume_checked()
        .script_pubkey()
}

async fn transaction_list(manager: &RustWalletManager) -> Vec<Transaction> {
    call!(manager.actor.transactions()).await.expect("the wallet actor answers")
}

/// The label the home screen's transaction list shows for `txid`
async fn listed_label(manager: &RustWalletManager, txid: Txid) -> Option<String> {
    transaction_list(manager)
        .await
        .into_iter()
        .find(|transaction| transaction.id().0 == txid)
        .expect("the transaction is listed")
        .label_opt()
}

impl Transaction {
    fn label_opt(&self) -> Option<String> {
        let labels = match self {
            Transaction::Confirmed(confirmed) => &confirmed.labels,
            Transaction::Unconfirmed(pending) => &pending.labels,
        };

        crate::transaction::non_empty_transaction_label(labels)
    }
}

/// What the transaction details screen shows for `txid`
struct ShownDetails {
    amount_sats: u64,
    sent: bool,
    sent_sans_fee_sats: Option<u64>,
    fee_sats: Option<u64>,
    address: Option<String>,
    label: Option<String>,
    confirmations: Option<u32>,
    lock: TransactionLockState,
}

async fn open_details(manager: &RustWalletManager, txid: Txid) -> ShownDetails {
    let presentation = manager.transaction_details(tx_id(txid)).await.expect("details open");
    let details = presentation.details();
    let lock = manager.transaction_lock_state(tx_id(txid)).await.expect("lock state reads");

    ShownDetails {
        amount_sats: details.amount().to_sat(),
        sent: details.is_sent(),
        sent_sans_fee_sats: details.sent_sans_fee().map(|amount| amount.to_sat()),
        fee_sats: details.fee.map(|fee| fee.to_sat()),
        address: details.address().map(|address| address.to_string()),
        label: details.transaction_label(),
        confirmations: presentation.confirmations(),
        lock,
    }
}

/// The BIP329 export's records, as Settings → Export Labels writes them
async fn exported_labels(id: &WalletId) -> Vec<serde_json::Value> {
    let jsonl = LabelManager::try_new(id.clone())
        .expect("the label manager opens")
        .export()
        .await
        .expect("the labels export");

    jsonl
        .lines()
        .map(|line| serde_json::from_str(line).expect("each export line is a json record"))
        .collect()
}

fn records_for<'a>(
    records: &'a [serde_json::Value],
    kind: &str,
    reference: &str,
) -> Vec<&'a serde_json::Value> {
    records.iter().filter(|record| record["type"] == kind && record["ref"] == reference).collect()
}

/// Forces a pull-to-refresh and holds its history request for `script`, returning once that
/// request reached the node, so the refresh is known to be in flight
async fn hold_a_refresh(
    fx: &Fixture,
    manager: &RustWalletManager,
    script: &ScriptBuf,
) -> (Gate, RuleHandle) {
    let digest = sha256::Hash::hash(script.as_bytes());
    let before = fx.node.history_requests_for(script).len();
    let (gate, rule) = fx.node.on(&format!("/scripthash/{digest}/")).hold();
    manager.force_wallet_scan().await.expect("the refresh starts");
    wait_until("the refresh to ask about the held address", || {
        fx.node.history_requests_for(script).len() > before && gate.waiting() > 0
    })
    .await;

    (gate, rule)
}

/// Releases a refresh held by `hold_a_refresh` and waits for it to complete and settle, counting
/// only messages from after the release
async fn release_the_refresh(screen: &mut ScreenUpdates, gate: Gate, rule: RuleHandle) {
    screen.seen.clear();
    drop(rule);
    gate.open();
    wait_for_refresh_to_settle(screen).await;
}

/// Waits for the released refresh to complete and the scan status to settle, counting only
/// messages seen since the screen was last cleared
async fn wait_for_refresh_to_settle(screen: &mut ScreenUpdates) {
    screen
        .until_then_idle("the refresh to complete", |message| {
            matches!(message, Msg::ScanComplete(_))
        })
        .await;
}

/// A funded hot wallet with an older 390-sat receipt that is already labeled and locked, whose new
/// 1,000-sat receipt is labeled, locked from coin control, unlocked from its details, and toggled
#[tokio::test(flavor = "current_thread")]
async fn receipt_label_survives_coin_control_and_transaction_lock_changes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    let wallet =
        import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());
    let old_txid = fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(390));
    let receipt_txid =
        fx.node.chain().pay(&receive_script(&manager, 1).await, Amount::from_sat(1_000));

    fx.node.chain().confirm_together(&[old_txid, receipt_txid]);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    // the older receipt was labeled and locked earlier
    let details = |txid| {
        let manager = &manager;
        async move { manager.transaction_details(tx_id(txid)).await.expect("details open").details() }
    };

    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details(old_txid).await, "Lock test 390".into(), None)
        .expect("the old label saves");

    let lock = manager
        .toggle_transaction_lock_state(tx_id(old_txid))
        .await
        .expect("the old receipt locks");

    assert_eq!(lock, TransactionLockState::Locked);
    let old_output = format!("{old_txid}:0");
    let old_records = exported_labels(&wallet.id).await;
    assert_eq!(
        records_for(&old_records, "output", &old_output).len(),
        1,
        "the locked 390 output has an export record, got {old_records:#?}"
    );

    // the receipt label is saved from the details screen and left to settle
    screen.seen.clear();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details(receipt_txid).await, "Receipt".into(), None)
        .expect("the receipt label saves");

    screen
        .until("the list to show the saved label", |message| match message {
            Msg::TransactionUpdated(transaction) => {
                transaction.id().0 == receipt_txid
                    && transaction.label_opt().as_deref() == Some("Receipt")
            }

            Msg::ScanComplete(transactions)
            | Msg::AvailableTransactions(transactions)
            | Msg::UpdatedTransactions(transactions) => transactions.iter().any(|transaction| {
                transaction.id().0 == receipt_txid
                    && transaction.label_opt().as_deref() == Some("Receipt")
            }),

            _ => false,
        })
        .await;

    let label = Some("Receipt".to_string());
    assert_eq!(listed_label(&manager, receipt_txid).await, label);
    assert_eq!(open_details(&manager, receipt_txid).await.label, label);

    // the export carries the receipt's transaction and address records, and the old lock as it was
    let records = exported_labels(&wallet.id).await;
    let receipt_tx_records = records_for(&records, "tx", &receipt_txid.to_string());
    assert_eq!(receipt_tx_records.len(), 1, "one transaction record, got {records:#?}");
    assert_eq!(receipt_tx_records[0]["label"], "Receipt");

    let receipt_address = manager.address_at(1).await.unwrap().address.to_string();
    assert_eq!(
        records_for(&records, "addr", &receipt_address).len(),
        1,
        "the receiving address gets a record, got {records:#?}"
    );

    assert_eq!(
        records_for(&records, "output", &old_output),
        records_for(&old_records, "output", &old_output),
        "saving the new label must leave the locked 390 output record untouched"
    );

    // lock the receipt's output from coin control
    let coin_control = manager.new_coin_control_manager().await.expect("coin control opens");
    let receipt_utxo = coin_control
        .utxos()
        .into_iter()
        .find(|utxo| utxo.outpoint.txid().0 == receipt_txid)
        .expect("the receipt is an unspent output");

    assert!(receipt_utxo.spendable);
    assert_eq!(receipt_utxo.label, label, "coin control shows the receipt's label");
    coin_control
        .set_utxo_spendability(receipt_utxo.outpoint.clone(), false)
        .await
        .expect("the output locks");

    let utxos = coin_control.utxos();
    let locked_receipt =
        utxos.iter().find(|utxo| utxo.outpoint == receipt_utxo.outpoint).expect("still listed");
    let old_utxo =
        utxos.iter().find(|utxo| utxo.outpoint.txid().0 == old_txid).expect("old output listed");

    assert!(!locked_receipt.spendable, "coin control shows the receipt locked");
    assert_eq!(locked_receipt.label, label, "the locked output keeps its label in coin control");
    assert_eq!(locked_receipt.amount.as_sats(), 1_000);
    assert!(!old_utxo.spendable, "the 390 lock is untouched");
    assert_eq!(open_details(&manager, receipt_txid).await.lock, TransactionLockState::Locked);
    assert_eq!(listed_label(&manager, receipt_txid).await, label, "locking keeps the label");
    assert_eq!(open_details(&manager, receipt_txid).await.label, label);

    // unlock from the transaction details screen's locked badge
    let lock =
        manager.unlock_transaction_outputs(tx_id(receipt_txid)).await.expect("the receipt unlocks");

    assert_eq!(lock, TransactionLockState::Unlocked);

    let unlocked = open_details(&manager, receipt_txid).await;
    assert_eq!(unlocked.amount_sats, 1_000);
    assert_eq!(unlocked.label, label, "unlocking keeps the label");
    assert_eq!(listed_label(&manager, receipt_txid).await, label);

    coin_control.reload_labels().await.expect("coin control reloads");
    let utxos = coin_control.utxos();
    let unlocked_receipt =
        utxos.iter().find(|utxo| utxo.outpoint == receipt_utxo.outpoint).expect("still listed");

    assert!(unlocked_receipt.spendable, "coin control shows the receipt unlocked again");
    assert_eq!(unlocked_receipt.label, label, "the unlocked output keeps its label");

    assert!(
        utxos.iter().any(|utxo| utxo.outpoint.txid().0 == old_txid && !utxo.spendable),
        "the 390 lock is still untouched"
    );

    // the details screen's Lock action toggles back and forth, ending unlocked
    let receipt = tx_id(receipt_txid);
    let toggled = manager.toggle_transaction_lock_state(receipt.clone()).await.unwrap();
    assert_eq!(toggled, TransactionLockState::Locked);
    assert_eq!(open_details(&manager, receipt_txid).await.label, label);

    let toggled = manager.toggle_transaction_lock_state(receipt).await.unwrap();
    assert_eq!(toggled, TransactionLockState::Unlocked);

    let ended = open_details(&manager, receipt_txid).await;
    assert_eq!((ended.label, ended.lock), (label.clone(), TransactionLockState::Unlocked));
    assert_eq!(listed_label(&manager, receipt_txid).await, label);
    assert_eq!(
        open_details(&manager, old_txid).await.lock,
        TransactionLockState::Locked,
        "toggling the receipt never touches the old lock"
    );

    assert_eq!(
        records_for(&exported_labels(&wallet.id).await, "output", &old_output),
        records_for(&old_records, "output", &old_output),
    );
}

#[tokio::test(flavor = "current_thread")]
async fn details_opened_during_a_refresh_belong_to_their_transaction_and_confirm_in_place() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    let wallet =
        import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());

    let first_address = manager.address_at(0).await.unwrap().address.to_string();
    let second_address = manager.address_at(1).await.unwrap().address.to_string();
    let paid = fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(50_000));
    let other = fx.node.chain().pay(&receive_script(&manager, 1).await, Amount::from_sat(20_000));

    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    // the transaction about to be opened has its own label and a lock
    let details = manager.transaction_details(tx_id(paid)).await.unwrap().details();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details, "Payjoin receive".into(), None)
        .expect("the label saves");

    manager.toggle_transaction_lock_state(tx_id(paid)).await.expect("the output locks");

    // hold the watcher's status poll and a pull-to-refresh scan so both are in flight
    let status_path = format!("/tx/{paid}/status");
    let (status_gate, status_rule) = fx.node.on(&status_path).hold();
    let (scan_gate, scan_rule) =
        hold_a_refresh(&fx, &manager, &receive_script(&manager, 0).await).await;

    let opened = open_details(&manager, paid).await;
    assert_eq!(opened.amount_sats, 50_000);
    assert_eq!(opened.address.as_deref(), Some(first_address.as_str()));
    assert_eq!(opened.label.as_deref(), Some("Payjoin receive"));
    assert_eq!(opened.confirmations, None, "the payment is still pending");
    assert_eq!(opened.lock, TransactionLockState::Locked);

    let other_opened = open_details(&manager, other).await;
    assert_eq!(other_opened.amount_sats, 20_000);
    assert_eq!(other_opened.address.as_deref(), Some(second_address.as_str()));
    assert_eq!(other_opened.label, None, "another transaction's label must not leak in");
    assert_eq!(other_opened.lock, TransactionLockState::Unlocked);

    // the node confirms the payment while the details screen and the refresh wait on it, and the
    // watcher reports it while the refresh is still held, so the refresh finishes last
    wait_until("the watcher to poll the node", || status_gate.waiting() > 0).await;
    screen.seen.clear();
    let height = fx.node.chain().confirm(paid);
    drop(status_rule);
    status_gate.open();

    let confirmed = |message: &Msg| {
        matches!(
            message,
            Msg::TransactionDetailsUpdated(details)
                if details.tx_id().0 == paid && details.confirmations().is_some()
        )
    };

    screen.until("the open details to confirm", confirmed).await;

    let updated = screen
        .seen
        .iter()
        .find_map(|message| match message {
            Msg::TransactionDetailsUpdated(details) if confirmed(message) => Some(details.clone()),
            _ => None,
        })
        .expect("confirmed details were reconciled");

    let updated_details = updated.details();
    assert_eq!(updated_details.amount().to_sat(), 50_000);
    assert_eq!(
        updated_details.address().map(|address| address.to_string()),
        Some(first_address.clone())
    );

    assert_eq!(updated_details.transaction_label().as_deref(), Some("Payjoin receive"));
    assert_eq!(updated.confirmations(), Some(fx.node.chain().tip_height() - height + 1));

    // the forced refresh finishes after the watcher, and must not undo the confirmation
    release_the_refresh(&mut screen, scan_gate, scan_rule).await;

    // reopening after the refresh shows the same transaction, now confirmed, with its lock
    let reopened = open_details(&manager, paid).await;
    assert_eq!(reopened.amount_sats, 50_000);
    assert_eq!(reopened.address.as_deref(), Some(first_address.as_str()));
    assert_eq!(reopened.label.as_deref(), Some("Payjoin receive"));
    assert!(reopened.confirmations.is_some());
    assert_eq!(reopened.lock, TransactionLockState::Locked);

    let other_reopened = open_details(&manager, other).await;
    assert_eq!(other_reopened.amount_sats, 20_000);
    assert_eq!(other_reopened.confirmations, None, "the other payment is still pending");
    assert_eq!(other_reopened.label, None);
}

/// A payment this wallet sends keeps its amount, destination, fee and label while a refresh runs and
/// the node confirms it, as the open details screen shows them
#[tokio::test(flavor = "current_thread")]
async fn sent_payment_details_keep_amount_destination_fee_and_label_as_it_confirms() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    let mnemonic = fresh_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());
    let funding_script = receive_script(&manager, 0).await;
    let funding = fx.node.chain().pay(&funding_script, Amount::from_sat(100_000));
    fx.node.chain().confirm(funding);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    // pay someone else's first address, signed with this wallet's own words
    let destination = fresh_mnemonic()
        .into_descriptors(None, fx.network, WalletAddressType::NativeSegwit)
        .external
        .extended_descriptor
        .at_derivation_index(0)
        .unwrap()
        .script_pubkey();

    let destination = bitcoin::Address::from_script(&destination, BitcoinNetwork::Signet).unwrap();
    let pay_to = crate::wallet::Address::new(destination.clone());
    let fee_rate = bitcoin::FeeRate::from_sat_per_vb(2).unwrap();
    let mut psbt = call!(manager.actor.build_tx(Amount::from_sat(10_000), pay_to, fee_rate))
        .await
        .expect("the wallet actor answers")
        .expect("the payment builds");

    let signer = mnemonic
        .into_descriptors(None, fx.network, WalletAddressType::NativeSegwit)
        .into_create_params()
        .network(fx.network.into())
        .create_wallet_no_persist()
        .unwrap();

    assert!(signer.sign(&mut psbt, Default::default()).unwrap(), "the payment is fully signed");
    let fee = psbt.fee().unwrap().to_sat();
    let signed = psbt.extract_tx().unwrap();
    let sent = signed.compute_txid();
    screen.seen.clear();
    manager
        .broadcast_transaction(Arc::new(BitcoinTransaction(signed)))
        .await
        .expect("the payment broadcasts");

    wait_for_refresh_to_settle(&mut screen).await;

    let details = manager.transaction_details(tx_id(sent)).await.unwrap().details();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details, "Rent".into(), None)
        .expect("the label saves");

    // hold the watcher's status poll and a pull-to-refresh scan so both are in flight
    let (status_gate, status_rule) = fx.node.on(&format!("/tx/{sent}/status")).hold();
    let (scan_gate, scan_rule) = hold_a_refresh(&fx, &manager, &funding_script).await;
    let shows_the_payment = |shown: &ShownDetails| {
        assert!(shown.sent, "the payment shows as sent");
        assert_eq!(shown.sent_sans_fee_sats, Some(10_000));
        assert_eq!(shown.fee_sats, Some(fee));
        assert_eq!(shown.address.as_deref(), Some(destination.to_string().as_str()));
        assert_eq!(shown.label.as_deref(), Some("Rent"));
    };

    let pending = open_details(&manager, sent).await;
    shows_the_payment(&pending);
    assert_eq!(pending.confirmations, None, "the payment is still pending");

    // the node confirms the payment while the details screen and the refresh wait on it, and the
    // watcher reports it while the refresh is still held, so the refresh finishes last
    wait_until("the watcher to poll the node", || status_gate.waiting() > 0).await;
    screen.seen.clear();
    fx.node.chain().confirm(sent);
    drop(status_rule);
    status_gate.open();

    let confirmed = |message: &Msg| {
        matches!(
            message,
            Msg::TransactionDetailsUpdated(details)
                if details.tx_id().0 == sent && details.confirmations().is_some()
        )
    };

    screen.until("the open details to confirm", confirmed).await;
    let updated = screen
        .seen
        .iter()
        .find_map(|message| match message {
            Msg::TransactionDetailsUpdated(details) if confirmed(message) => Some(details.clone()),
            _ => None,
        })
        .expect("confirmed details were reconciled");

    let updated = updated.details();
    assert!(updated.is_sent());
    assert_eq!(updated.sent_sans_fee().map(|amount| amount.to_sat()), Some(10_000));
    assert_eq!(updated.fee.map(|amount| amount.to_sat()), Some(fee));
    assert_eq!(updated.address().map(|address| address.to_string()), Some(destination.to_string()));
    assert_eq!(updated.transaction_label().as_deref(), Some("Rent"));

    // the forced refresh finishes after the watcher, and must not undo the confirmation
    release_the_refresh(&mut screen, scan_gate, scan_rule).await;
    let reopened = open_details(&manager, sent).await;
    shows_the_payment(&reopened);
    assert!(reopened.confirmations.is_some(), "the payment now shows as confirmed");
}

/// Pastes a hardware wallet's public export, as Add Wallet → Hardware Wallet → Paste does
fn paste_hardware_export(mnemonic: &Mnemonic, network: Network) -> Wallet {
    let export = crate::wallet::test_support::public_descriptor_export(mnemonic, network);

    Wallet::new_from_xpub(export).expect("the pasted export imports")
}

#[tokio::test(flavor = "current_thread")]
async fn pasted_hardware_export_imports_its_fingerprint_and_existing_funds() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    // the hardware wallet already received twice before it is imported
    let mnemonic = fresh_mnemonic();
    let descriptor =
        mnemonic.clone().into_descriptors(None, fx.network, WalletAddressType::NativeSegwit);

    let script_at = |index| {
        descriptor.external.extended_descriptor.at_derivation_index(index).unwrap().script_pubkey()
    };

    let first = fx.node.chain().pay(&script_at(0), Amount::from_sat(100_000));
    let second = fx.node.chain().pay(&script_at(1), Amount::from_sat(50_000));
    fx.node.chain().confirm_together(&[first, second]);

    let wallet = paste_hardware_export(&mnemonic, fx.network);
    let metadata = wallet.metadata.clone();
    fx.own(&metadata.id);
    drop(wallet);

    let fingerprint = mnemonic.xpub(fx.network.into()).fingerprint().to_string();
    assert_eq!(
        metadata.master_fingerprint.as_ref().map(|fingerprint| fingerprint.as_lowercase()),
        Some(fingerprint),
        "the imported wallet carries the hardware wallet's fingerprint"
    );

    assert_eq!(metadata.wallet_type, WalletType::Cold);
    assert_eq!(metadata.network, fx.network);
    assert_eq!(metadata.address_type, WalletAddressType::NativeSegwit);

    let (manager, mut screen) = fx.open(metadata.id.clone());
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    let balance = manager.balance().await.expect("the balance reads");
    assert_eq!(balance.0.confirmed, Amount::from_sat(150_000));

    let mut listed = transaction_list(&manager)
        .await
        .into_iter()
        .map(|transaction| transaction.id().0)
        .collect::<Vec<_>>();

    listed.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(listed, expected, "both received transactions are listed");
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_hardware_key_receives_first_at_index_zero() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    // a key generated for this test has no history, unlike published test vectors
    let mnemonic = fresh_mnemonic();

    // derive m/84'/1'/0'/0/0 straight from the seed, not through the descriptors the import uses
    let secp = Secp256k1::new();
    let first_key = Xpriv::new_master(BitcoinNetwork::Signet, &mnemonic.to_seed(""))
        .unwrap()
        .derive_priv(&secp, &DerivationPath::from_str("m/84'/1'/0'/0/0").unwrap())
        .unwrap()
        .to_priv();

    let first_key = CompressedPublicKey::from_private_key(&secp, &first_key).unwrap();
    let expected = bitcoin::Address::p2wpkh(&first_key, BitcoinNetwork::Signet).to_string();
    let wallet = paste_hardware_export(&mnemonic, fx.network);
    let id = wallet.metadata.id.clone();
    fx.own(&id);
    drop(wallet);

    let (manager, mut screen) = fx.open(id);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    screen.seen.clear();
    manager.dispatch(super::WalletManagerAction::OpenReceiveAddress).expect("receive opens");
    screen
        .until("the receive address", |message| matches!(message, Msg::ReceiveAddressUpdated(_)))
        .await;

    let shown = screen
        .seen
        .iter()
        .find_map(|message| match message {
            Msg::ReceiveAddressUpdated(state) => Some(state.clone()),
            _ => None,
        })
        .expect("the receive sheet got an address");

    assert_eq!(shown.address.info.address.to_string(), expected);
    assert_eq!(shown.address.info.index, 0);
    let path = shown.address.derivation_path.clone().expect("a hardware wallet shows its path");
    assert!(path.ends_with("/0/0"), "the first receive path is /0/0, got {path}");
}

/// Both apps open Send and coin control through these manager calls, and the iOS Send screen also
/// checks the ledger state the wallet screen was sent, so a funded wallet stays unspendable until
/// its first full scan lands
#[tokio::test(flavor = "current_thread")]
async fn send_opens_only_after_the_initial_scan_completes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    let wallet =
        import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());
    let manager = Arc::new(manager);
    let funded = fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(100_000));
    fx.node.chain().confirm(funded);

    let (scan_gate, scan_rule) = fx.node.on("/scripthash/").hold();
    manager.start_wallet_scan().await.expect("the scan starts");
    wait_until("the initial scan to reach the node", || scan_gate.waiting() > 0).await;

    assert!(matches!(manager.ledger_state(), WalletLedgerState::InitialScanIncomplete(_)));
    let balance = Arc::new(manager.balance().await.expect("the balance reads"));
    assert!(
        matches!(
            manager.clone().new_send_flow_manager(balance.clone()),
            Err(super::Error::InitialScanIncomplete)
        ),
        "Send must be refused while the initial scan runs"
    );

    assert!(
        matches!(
            manager.new_coin_control_manager().await,
            Err(super::Error::InitialScanIncomplete)
        ),
        "coin control must be refused while the initial scan runs"
    );

    drop(scan_rule);
    scan_gate.open();
    screen.until_scan_settles().await;
    screen
        .until("the wallet screen to learn the ledger is complete", |message| {
            matches!(message, Msg::LedgerStateChanged(WalletLedgerState::Complete))
        })
        .await;

    assert_eq!(manager.ledger_state(), WalletLedgerState::Complete);
    let balance = Arc::new(manager.balance().await.expect("the balance reads"));
    assert_eq!(balance.0.confirmed, Amount::from_sat(100_000));
    manager.clone().new_send_flow_manager(balance).expect("Send opens once the scan completes");
}

/// Discovery of imported words finds history on the other address types while the wallet's own
/// initial scan is still running, and choosing one from the Choose Wallet Type sheet loads only
/// that type's history
#[tokio::test(flavor = "current_thread")]
async fn choosing_a_discovered_address_type_during_the_initial_scan_loads_its_history() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    // the same words received on each address type before they were imported
    let mnemonic = fresh_mnemonic();
    let first_script = |address_type| {
        mnemonic
            .clone()
            .into_descriptors(None, fx.network, address_type)
            .external
            .extended_descriptor
            .at_derivation_index(0)
            .unwrap()
            .script_pubkey()
    };

    let native_script = first_script(WalletAddressType::NativeSegwit);
    let native = fx.node.chain().pay(&native_script, Amount::from_sat(50_000));
    let wrapped = fx
        .node
        .chain()
        .pay(&first_script(WalletAddressType::WrappedSegwit), Amount::from_sat(30_000));

    let legacy =
        fx.node.chain().pay(&first_script(WalletAddressType::Legacy), Amount::from_sat(20_000));

    fx.node.chain().confirm_together(&[native, wrapped, legacy]);

    // hold the wallet's own native segwit scan so discovery reports while it is still running
    let native_digest = sha256::Hash::hash(native_script.as_bytes());
    let (scan_gate, scan_rule) = fx.node.on(&format!("/scripthash/{native_digest}/")).hold();

    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());
    manager.start_wallet_scan().await.expect("the scan starts");
    wait_until("the initial scan to reach the node", || scan_gate.waiting() > 0).await;

    let found_types = |message: &Msg| match message {
        Msg::WalletScannerResponse(ScannerResponse::FoundAddresses(found)) => {
            let mut types = found.iter().map(|address| address.type_).collect::<Vec<_>>();
            types.sort_by_key(|address_type| format!("{address_type:?}"));
            Some(types)
        }

        _ => None,
    };

    screen
        .until("discovery to offer the other address types", |message| {
            found_types(message).is_some()
        })
        .await;

    let offered = screen.seen.iter().find_map(found_types).expect("discovery reported");
    assert_eq!(offered, vec![WalletAddressType::Legacy, WalletAddressType::WrappedSegwit]);
    assert!(
        matches!(manager.ledger_state(), WalletLedgerState::InitialScanIncomplete(_)),
        "the choice is offered while the initial scan still runs"
    );

    manager
        .switch_to_different_wallet_address_type(WalletAddressType::WrappedSegwit)
        .await
        .expect("the switch to wrapped segwit succeeds");

    screen.seen.clear();
    drop(scan_rule);
    scan_gate.open();

    // a late result from the interrupted native segwit scan would land before the status settles
    screen
        .until_then_idle("the wrapped segwit history to load", |message| {
            matches!(message, Msg::ScanComplete(transactions)
                if transactions.iter().any(|transaction| transaction.id().0 == wrapped))
        })
        .await;

    let listed = transaction_list(&manager)
        .await
        .into_iter()
        .map(|transaction| transaction.id().0)
        .collect::<Vec<_>>();

    assert_eq!(listed, vec![wrapped], "only the chosen type's history is listed");
    let balance = manager.balance().await.expect("the balance reads");
    assert_eq!(balance.0.confirmed, Amount::from_sat(30_000));
    assert_eq!(manager.metadata.read().address_type, WalletAddressType::WrappedSegwit);
}
