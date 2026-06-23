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
    transaction::{Transaction, unsigned_transaction::UnsignedTransaction},
    wallet::{
        Wallet,
        balance::Balance,
        metadata::{DiscoveryState, WalletBirthday, WalletId, WalletMetadata},
    },
};

use super::{
    BalancePresentation, DeferredSender, Error, Message, MessageSender, RustWalletManager,
    SingleOrMany, WalletActor, WalletInitialState, WalletLedgerState, WalletLoadState,
    WalletScanStatus, downgrade_and_notify_if_needed,
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

fn initial_state_for_wallet(
    metadata: WalletMetadata,
    load_state: WalletLoadState,
    balance: Balance,
    unsigned_transactions: Vec<Arc<UnsignedTransaction>>,
) -> WalletInitialState {
    let scan_status = WalletScanStatus::Idle;
    let ledger_state = WalletLedgerState::from_metadata_and_scan_status(&metadata, &scan_status);
    let balance_presentation = BalancePresentation::for_ledger_state(ledger_state);

    WalletInitialState {
        metadata,
        ledger_state,
        load_state,
        scan_status,
        balance_presentation,
        balance: Arc::new(balance),
        unsigned_transactions,
    }
}

fn unsigned_transactions_for_initial_state(wallet_id: &WalletId) -> Vec<Arc<UnsignedTransaction>> {
    match Database::global().unsigned_transactions().get_by_wallet_id(wallet_id) {
        Ok(txns) => txns.into_iter().map(|txn| Arc::new(txn.into())).collect(),
        Err(error) => {
            warn!("unable to read unsigned transactions for wallet {wallet_id}: {error}");
            Vec::new()
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

        let initial_load_state = if cached_transactions.is_empty() {
            WalletLoadState::Loading
        } else {
            WalletLoadState::Scanning(cached_transactions)
        };
        let initial_state = initial_state_for_wallet(
            metadata.clone(),
            initial_load_state,
            cached_balance,
            unsigned_transactions_for_initial_state(&id),
        );

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor = WalletActor::new(wallet, sender.clone(), scan_status.clone())
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
            label_manager,
            initial_state,
            discovery_scanner,
        })
    }

    #[uniffi::constructor]
    pub fn try_new_from_xpub(xpub: String) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::try_new_persisted_from_xpub(xpub)?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();
        let initial_state = initial_state_for_wallet(
            metadata.clone(),
            WalletLoadState::Loading,
            wallet.balance(),
            unsigned_transactions_for_initial_state(&id),
        );

        let discovery_scanner = start_discovery_scanner(metadata.clone(), sender.clone());

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor = WalletActor::new(wallet, sender.clone(), scan_status.clone())
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
            label_manager,
            initial_state,
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
        let initial_state = initial_state_for_wallet(
            metadata.clone(),
            WalletLoadState::Loading,
            wallet.balance(),
            unsigned_transactions_for_initial_state(&id),
        );

        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor = WalletActor::new(wallet, sender.clone(), scan_status.clone())
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
            label_manager,
            initial_state,
            discovery_scanner: None,
        })
    }
}
