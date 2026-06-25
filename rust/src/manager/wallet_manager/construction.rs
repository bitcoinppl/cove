use std::sync::Arc;

use act_zero::Addr;
use cove_tokio::task::{self, spawn_actor};
use cove_util::result_ext::ResultExt as _;
use parking_lot::RwLock;
use tracing::warn;

use crate::{
    database::Database,
    discovery_scanner::WalletDiscoveryScanner,
    label_manager::LabelManager,
    tap_card::tap_signer_reader::DeriveInfo,
    transaction::Transaction,
    wallet::{
        Wallet,
        balance::Balance,
        metadata::{DiscoveryState, WalletBirthday, WalletId, WalletMetadata},
    },
};

use super::{
    DeferredSender, Error, Message, MessageSender, RustWalletManager, SingleOrMany,
    WalletActor, WalletBootstrapUnsignedTransactions, WalletLedgerState, WalletScanStatus,
    WalletSnapshot, downgrade_and_notify_if_needed,
};

fn start_discovery_scanner(
    metadata: WalletMetadata,
    sender: flume::Sender<SingleOrMany>,
) -> Option<Addr<WalletDiscoveryScanner>> {
    if !matches!(
        &metadata.discovery_state,
        DiscoveryState::StartedJson(_) | DiscoveryState::StartedMnemonic
    ) {
        return None;
    }

    let id = metadata.id.clone();
    match WalletDiscoveryScanner::try_new(metadata, sender) {
        Ok(scanner) => Some(spawn_actor(scanner)),
        Err(error) => {
            warn!("unable to start wallet discovery scanner for {id}: {error}");
            None
        }
    }
}

#[uniffi::export]
impl RustWalletManager {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(10);

        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let reconciler = MessageSender::new(sender.clone());
        let mut deferred = DeferredSender::new(reconciler.clone());

        let mut wallet = Wallet::try_load_persisted(id.clone())?;
        wallet.metadata = downgrade_and_notify_if_needed(wallet.metadata, &mut deferred)?;

        let metadata = Database::global()
            .wallets
            .get(&id, network, mode)
            .map_err_str(Error::GetSelectedWalletError)?
            .ok_or(Error::WalletDoesNotExist)?;

        // sanity check to make sure the wallet metadata is correct
        if wallet.metadata != metadata {
            return Err(Error::UnknownError(
                "Database contains incorrect wallet metadata".to_string(),
            ));
        }

        let id = metadata.id.clone();

        // read cached and send to UI immediately
        let cached_balance: Balance = wallet.balance();
        let cached_transactions: Vec<Transaction> = wallet.transactions();
        deferred.queue(Message::WalletBalanceChanged(cached_balance.clone().into()));
        deferred.queue(Message::LedgerStateChanged(
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &WalletScanStatus::Idle),
        ));

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot {
            balance: cached_balance,
            transactions: cached_transactions,
        }));
        let unsigned_transactions = WalletBootstrapUnsignedTransactions::database(id.clone());
        let wallet_actor =
            WalletActor::new(wallet, sender.clone(), scan_status.clone(), wallet_snapshot.clone())
                .map_err(|e| Error::DatabaseCorruption { id: id.clone(), error: e.to_string() })?;
        let actor = task::spawn_actor(wallet_actor);

        let discovery_scanner = start_discovery_scanner(metadata.clone(), sender);

        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler,
            reconcile_receiver: Arc::new(receiver),
            scan_status,
            wallet_snapshot,
            unsigned_transactions,
            label_manager,
            discovery_scanner,
        })
    }

    #[uniffi::constructor]
    pub fn try_new_from_xpub(xpub: String) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::try_new_persisted_from_xpub(xpub)?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();
        let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
        let unsigned_transactions = WalletBootstrapUnsignedTransactions::database(id.clone());

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor =
            WalletActor::new(wallet, sender.clone(), scan_status.clone(), wallet_snapshot.clone())
                .map_err(|e| Error::DatabaseCorruption { id: id.clone(), error: e.to_string() })?;
        let actor = task::spawn_actor(wallet_actor);
        let discovery_scanner = start_discovery_scanner(metadata.clone(), sender.clone());
        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
            scan_status,
            wallet_snapshot,
            unsigned_transactions,
            label_manager,
            discovery_scanner,
        })
    }

    #[uniffi::constructor(default(backup = None, birthday = None))]
    pub fn try_new_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive_info: DeriveInfo,
        backup: Option<Vec<u8>>,
        birthday: Option<WalletBirthday>,
    ) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::try_new_persisted_from_tap_signer(
            tap_signer.clone(),
            derive_info,
            backup,
            birthday,
        )?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();
        let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
        let unsigned_transactions = WalletBootstrapUnsignedTransactions::database(id.clone());

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor =
            WalletActor::new(wallet, sender.clone(), scan_status.clone(), wallet_snapshot.clone())
                .map_err(|e| Error::DatabaseCorruption { id: id.clone(), error: e.to_string() })?;
        let actor = task::spawn_actor(wallet_actor);
        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
            scan_status,
            wallet_snapshot,
            unsigned_transactions,
            label_manager,
            discovery_scanner: None,
        })
    }
}
