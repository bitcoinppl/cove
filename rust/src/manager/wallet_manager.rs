pub mod actor;
pub mod balance_presentation;
mod construction;
mod display;
mod exports;
pub mod ledger_state;
mod payjoin;
pub mod receive_address;
mod send_transactions;
mod transaction_locks;
mod unsigned_transactions;
mod wallet_admin;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use act_zero::{Addr, call, send};
use actor::WalletActor;
pub use balance_presentation::BalancePresentation;
use flume::Receiver;
pub use ledger_state::WalletLedgerState;
use parking_lot::RwLock;
use receive_address::{ReceiveAddressPresentation, ReceiveAddressState};
use tracing::{debug, error, warn};

use cove_common::consts::MAX_RESCAN_GAP_LIMIT;
use cove_tokio::task;
use cove_util::result_ext::ResultExt as _;

use crate::{
    app::FfiApp,
    converter::{Converter, ConverterError},
    database::{Database, error::DatabaseError},
    discovery_scanner::{ScannerResponse, WalletDiscoveryScanner},
    fee_client::{FEE_CLIENT, FEES, FeeResponse},
    fiat::client::PriceResponse,
    keychain::{Keychain, KeychainError},
    label_manager::LabelManager,
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    psbt::Psbt,
    router::Route,
    transaction::{
        Amount, Transaction, TransactionDetails, TxId, Unit, ffi::BitcoinTransaction,
        unsigned_transaction::UnsignedTransaction,
    },
    wallet::{
        AddressInfo, Wallet, WalletAddressType, WalletError,
        balance::Balance,
        fingerprint::Fingerprint,
        metadata::{DiscoveryState, FiatOrBtc, WalletColor, WalletId, WalletMetadata, WalletType},
    },
    word_validator::WordValidator,
};

use cove_types::confirm::{ConfirmDetails, SplitOutput};
use cove_types::{confirm::AddressAndAmount, fees::FeeRateOptions};

use super::{
    coin_control_manager::RustCoinControlManager,
    deferred_sender::{self, DeferredSender, MessageSender},
    send_flow_manager::RustSendFlowManager,
};

type Action = WalletManagerAction;
type Message = WalletManagerReconcileMessage;
type Reconciler = dyn WalletManagerReconciler;
pub type SingleOrMany = deferred_sender::SingleOrMany<Message>;

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletManagerReconcileMessage {
    WalletScanStatusChanged(WalletScanStatus),
    LedgerStateChanged(WalletLedgerState),

    AvailableTransactions(Vec<Transaction>),
    ScanComplete(Vec<Transaction>),
    UpdatedTransactions(Vec<Transaction>),
    TransactionUpdated(Transaction),
    TransactionDetailsUpdated(Arc<TransactionDetails>),
    TransactionConfirmationsUpdated(TransactionConfirmationUpdate),

    NodeConnectionFailed(String),
    WalletMetadataChanged(Box<WalletMetadata>),
    WalletBalanceChanged(Arc<Balance>),

    WalletError(WalletManagerError),
    UnknownError(String),

    WalletScannerResponse(ScannerResponse),
    UnsignedTransactionsChanged,

    SendFlowError(SendFlowErrorAlert),
    HotWalletKeyMissing(WalletId),
    ReceiveAddressUpdated(ReceiveAddressState),
    ReceiveAddressPresentationUpdated(ReceiveAddressPresentation),
    ReceiveAddressLoadingChanged(bool),
    ReceiveAddressError(String),
    ReceiveAddressClosed(u64),

    PayjoinTxBroadcast,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletManagerAction {
    UpdateName(String),
    UpdateColor(WalletColor),
    UpdateUnit(Unit),
    UpdateFiatOrBtc(FiatOrBtc),
    ToggleSensitiveVisibility,
    ToggleDetailsExpanded,
    ToggleFiatOrBtc,
    ToggleFiatBtcPrimarySecondary,
    ToggleShowLabels,
    SelectCurrentWalletAddressType,
    SelectedWalletDisappeared,
    StartTransactionWatcher(Arc<TxId>),
    OpenReceiveAddress,
    CreateNewReceiveAddress,
    CloseReceiveAddress(u64),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletLoadState {
    Loading,
    Scanning(Vec<Transaction>),
    Loaded(Vec<Transaction>),
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletScanPhase {
    Full,
    Rescan,
    Incremental,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletScanProgress {
    pub phase: WalletScanPhase,
    pub checked: u32,
    pub gap: u32,
    pub stop_gap: u32,
    pub progress_basis_points: u32,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TransactionConfirmationUpdate {
    pub tx_id: Arc<TxId>,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletScanStatus {
    Idle,
    Scanning(WalletScanProgress),
    ScanningPendingProgress(WalletScanPhase),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletErrorAlert {
    NodeConnectionFailed(String),
    NoBalance,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowErrorAlert {
    SignAndBroadcast(String),
    ConfirmDetails(String),
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LabelExportResult {
    pub content: String,
    pub filename: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionExportResult {
    pub content: String,
    pub filename: String,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TransactionLockState {
    None,
    Unlocked,
    Locked,
    Mixed,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XpubExportResult {
    pub content: String,
    pub filename: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletInitialState {
    pub metadata: WalletMetadata,
    pub ledger_state: WalletLedgerState,
    pub load_state: WalletLoadState,
    pub scan_status: WalletScanStatus,
    pub balance_presentation: BalancePresentation,
    pub balance: Arc<Balance>,
    pub unsigned_transactions: Vec<Arc<UnsignedTransaction>>,
}

#[uniffi::export(callback_interface)]
pub trait WalletManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustWalletManager {
    pub id: WalletId,
    pub actor: Addr<WalletActor>,

    // cache, metadata already exists in the database and in the actor state,  this cache makes it
    // faster to access, but adds complexity to the code because we have to make sure its updated
    // in all the places
    pub metadata: Arc<RwLock<WalletMetadata>>,
    pub reconciler: MessageSender<Message>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,
    scan_status: Arc<RwLock<WalletScanStatus>>,

    label_manager: Arc<LabelManager>,
    initial_state: WalletInitialState,
    discovery_scanner: Option<Addr<WalletDiscoveryScanner>>,
}

pub type Error = WalletManagerError;
#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum WalletManagerError {
    #[error("failed to get selected wallet: {0}")]
    GetSelectedWalletError(String),

    #[error("wallet does not exist")]
    WalletDoesNotExist,

    #[error("unable to retrieve the secret words for the wallet {0}")]
    SecretRetrievalError(#[from] KeychainError),

    #[error("unable to mark wallet as verified")]
    MarkWalletAsVerifiedError(#[from] DatabaseError),

    #[error("unable to load wallet: {0}")]
    LoadWalletError(#[from] WalletError),

    #[error("unable to connect to node: {0}")]
    NodeConnectionFailed(String),

    #[error("unable to start wallet scan: {0}")]
    WalletScanError(String),

    #[error("unable to get transactions: {0}")]
    TransactionsRetrievalError(String),

    #[error("unable to get wallet balance: {0}")]
    WalletBalanceError(String),

    #[error("unable to get next address: {0}")]
    NextAddressError(String),

    #[error("unable to set wallet type: {0}")]
    SetWalletTypeError(String),

    #[error("unable to get height")]
    GetHeightError,

    #[error("unable to get transaction details: {0}")]
    TransactionDetailsError(String),

    #[error("actor error, not found")]
    ActorNotFound,

    #[error("unable to switch wallet to address type {0}, error: {1}")]
    UnableToSwitch(WalletAddressType, String),

    #[error("unable to get balance in fiat")]
    FiatError(String),

    #[error("unable to get fees: {0}")]
    FeesError(String),

    #[error("Can't send until initial scan completes.")]
    InitialScanIncomplete,

    #[error("unable to build transaction: {0}")]
    BuildTxError(String),

    #[error("insufficient funds: {0}")]
    InsufficientFunds(String),

    #[error("selected UTXOs include locked outputs")]
    LockedOutputsSelected,

    #[error("Unable to get confirm details, {0}")]
    GetConfirmDetailsError(String),

    #[error("Unable to sign and broadcast transaction, {0}")]
    SignAndBroadcastError(String),

    #[error(transparent)]
    Converter(#[from] ConverterError),

    #[error("Unknown error: {0}")]
    UnknownError(String),

    #[error("Error finalizing PSBT: {0}")]
    PsbtFinalizeError(String),

    #[error("Unable to get historical prices for transactions: {0}")]
    GetHistoricalPricesError(String),

    #[error("Unable to create report CSV: {0}")]
    CsvCreationError(String),

    #[error("Unable to add UTXOs to PSBT: {0}")]
    AddUtxosError(String),

    #[error("Unable to get output labels: {0}")]
    OutputLabelsError(String),

    #[error("Wallet database corrupted for {id}: {error}")]
    DatabaseCorruption { id: WalletId, error: String },

    #[error("Receive address error: {0}")]
    ReceiveAddressError(String),
}

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    #[uniffi::method]
    pub fn initial_state(&self) -> WalletInitialState {
        self.initial_state.clone()
    }

    /// Returns the metadata-derived bootstrap snapshot; live scan activity arrives through reconcile messages
    #[uniffi::method]
    pub fn ledger_state(&self) -> WalletLedgerState {
        let metadata = self.current_metadata();

        WalletLedgerState::from_metadata_and_scan_status(&metadata, &WalletScanStatus::Idle)
    }

    #[uniffi::method]
    pub fn balance_presentation(&self, scan_status: WalletScanStatus) -> BalancePresentation {
        let metadata = self.current_metadata();

        let ledger_state =
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &scan_status);

        BalancePresentation::for_ledger_state(ledger_state)
    }

    #[uniffi::method]
    pub fn balance_presentation_for_state(
        &self,
        ledger_state: WalletLedgerState,
    ) -> BalancePresentation {
        // ffi adapter for platform-owned ledger snapshots; no manager state is needed
        BalancePresentation::for_ledger_state(ledger_state)
    }

    #[uniffi::method]
    pub fn label_manager(&self) -> Arc<LabelManager> {
        self.label_manager.clone()
    }

    #[uniffi::method]
    pub fn new_send_flow_manager(
        self: Arc<Self>,
        balance: Arc<Balance>,
    ) -> Result<Arc<RustSendFlowManager>, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let me = self.clone();
        let metadata = self.current_metadata();

        Ok(RustSendFlowManager::new(metadata, balance, me))
    }

    #[uniffi::method]
    pub async fn new_coin_control_manager(&self) -> Result<Arc<RustCoinControlManager>, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let metadata = self.current_metadata();
        let unspent = call!(self.actor.list_unspent()).await.expect("actor failed")?;

        let manager = RustCoinControlManager::new(metadata, unspent);
        Ok(Arc::new(manager))
    }

    #[uniffi::method]
    pub fn convert_from_fiat_string(
        &self,
        fiat_amount: &str,
        prices: Arc<PriceResponse>,
    ) -> Amount {
        Converter::new().convert_from_fiat_string(
            fiat_amount,
            self.selected_fiat_currency(),
            *prices.as_ref(),
        )
    }

    #[uniffi::method]
    pub async fn get_fee_options(&self) -> Result<FeeRateOptions, Error> {
        let fee_client = &FEE_CLIENT;
        let fees = fee_client.fetch_and_get_fees().await.map_err_str(Error::FeesError)?;

        Ok(fees.into())
    }

    #[uniffi::method]
    pub async fn first_address(&self) -> Result<AddressInfo, Error> {
        let address_info = call!(self.actor.address_at(0))
            .await
            .map_err(|_| Error::UnknownError("failed to get first address".to_string()))?;

        Ok(address_info)
    }

    #[uniffi::method]
    pub fn save_unsigned_transaction(&self, details: Arc<ConfirmDetails>) -> Result<(), Error> {
        self.save_unsigned_transaction_internal(details)
    }

    #[uniffi::method]
    pub async fn split_transaction_outputs(
        &self,
        outputs: Vec<AddressAndAmount>,
    ) -> Result<SplitOutput, Error> {
        let outputs = call!(self.actor.split_transaction_outputs(outputs))
            .await
            .map_err(|_| Error::UnknownError("failed to split outputs".to_string()))?;

        Ok(outputs)
    }

    #[uniffi::method]
    pub fn get_unsigned_transactions(&self) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
        self.get_unsigned_transactions_internal()
    }

    #[uniffi::method]
    pub async fn get_transactions(&self) {
        let Ok(txns) = call!(self.actor.transactions()).await else { return };

        self.reconciler.send(Message::UpdatedTransactions(txns));
    }

    #[uniffi::method]
    pub fn delete_unsigned_transaction(&self, tx_id: Arc<TxId>) -> Result<(), Error> {
        self.delete_unsigned_transaction_internal(tx_id)
    }

    #[uniffi::method]
    pub async fn balance(&self) -> Balance {
        call!(self.actor.balance()).await.unwrap_or_default()
    }

    #[uniffi::method]
    pub async fn unlocked_spendable_balance(&self) -> Result<Amount, Error> {
        let amount = call!(self.actor.unlocked_trusted_spendable_balance())
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(amount.into())
    }

    /// Send entry point for unsigned hot wallet PSBTs
    #[uniffi::method]
    pub async fn initiate_payment(
        &self,
        psbt: Arc<Psbt>,
        payjoin_endpoint: Option<String>,
    ) -> Result<(), Error> {
        self.ensure_ledger_ready_for_spend()?;

        let psbt = Arc::unwrap_or_clone(psbt);
        call!(self.actor.initiate_payment(psbt.into(), payjoin_endpoint)).await.unwrap()?;

        self.force_wallet_scan().await;

        Ok(())
    }

    #[uniffi::method]
    pub async fn broadcast_transaction(
        &self,
        signed_transaction: Arc<BitcoinTransaction>,
    ) -> Result<(), Error> {
        self.ensure_ledger_ready_for_spend()?;

        let txn = Arc::unwrap_or_clone(signed_transaction);
        let tx_id = txn.tx_id();

        call!(self.actor.broadcast_transaction(txn.into())).await.unwrap()?;

        if let Err(error) = self.delete_unsigned_transaction(tx_id.into()) {
            error!("unable to delete unsigned transaction record: {error}");
        }

        self.force_wallet_scan().await;

        Ok(())
    }

    #[uniffi::method]
    pub async fn current_block_height(&self) -> Result<u32, Error> {
        let height =
            call!(self.actor.get_height(false)).await.map_err(|_| Error::GetHeightError)?;

        Ok(height as u32)
    }

    #[uniffi::method]
    pub async fn force_update_height(&self) -> Result<u32, Error> {
        let height = call!(self.actor.get_height(true)).await.map_err(|_| Error::GetHeightError)?;

        Ok(height as u32)
    }

    #[uniffi::method]
    pub async fn transaction_details(&self, tx_id: Arc<TxId>) -> Result<TransactionDetails, Error> {
        let tx_id = Arc::unwrap_or_clone(tx_id);
        let actor = self.actor.clone();

        let details = task::spawn(async move {
            call!(actor.transaction_details(tx_id))
                .await
                .map_err_str(Error::TransactionDetailsError)
        })
        .await
        .map_err_str(Error::TransactionDetailsError)??;

        // for unconfirmed transactions, trigger a background sync to update status
        // this uses SyncRequest with just this txid so it's fast
        if !details.is_confirmed() {
            send!(self.actor.perform_scan_for_single_tx_id(details.tx_id().0));
        }

        Ok(details)
    }

    /// Get address at the given index
    #[uniffi::method]
    pub async fn address_at(&self, index: u32) -> Result<AddressInfo, Error> {
        let address =
            call!(self.actor.address_at(index)).await.map_err(|_| Error::ActorNotFound)?;

        Ok(address)
    }

    #[uniffi::method]
    pub fn delete_wallet(&self) -> Result<(), Error> {
        self.delete_wallet_internal()
    }

    #[uniffi::method]
    pub fn set_wallet_type(&self, wallet_type: WalletType) -> Result<(), Error> {
        self.set_wallet_type_internal(wallet_type)
    }

    #[uniffi::method]
    pub fn validate_metadata(&self) {
        self.validate_metadata_internal()
    }

    #[uniffi::method]
    pub async fn start_wallet_scan(&self) -> Result<(), Error> {
        debug!("start_wallet_scan: {}", self.id);

        send!(self.actor.wallet_scan_and_notify(false));

        Ok(())
    }

    #[uniffi::method]
    pub async fn force_wallet_scan(&self) {
        debug!("force_wallet_scan: {}", self.id);

        send!(self.actor.wallet_scan_and_notify(true));
    }

    #[uniffi::method]
    pub async fn rescan_wallet_with_gap_limit(&self, gap_limit: u32) -> Result<(), Error> {
        debug!("rescan_wallet_with_gap_limit: {} gap_limit={}", self.id, gap_limit);

        if gap_limit == 0 || gap_limit > MAX_RESCAN_GAP_LIMIT {
            return Err(Error::WalletScanError(format!(
                "gap_limit must be between 1 and {MAX_RESCAN_GAP_LIMIT}",
            )));
        }

        send!(self.actor.perform_rescan_full_scan(gap_limit));

        Ok(())
    }

    #[uniffi::method]
    pub fn mark_wallet_as_verified(&self) -> Result<(), Error> {
        self.mark_wallet_as_verified_internal()
    }

    #[uniffi::method]
    pub fn wallet_metadata(&self) -> WalletMetadata {
        self.wallet_metadata_internal()
    }

    #[uniffi::method]
    pub fn non_default_account_number(&self) -> Option<u32> {
        wallet_account_number(&self.id).filter(|account| *account != 0)
    }

    /// Returns the number of confirmation steps required to delete this wallet
    /// - 2: Cold wallets, xpub-only wallets, or verified hot wallets
    /// - 3: Hot wallets that are NOT verified (highest risk)
    #[uniffi::method]
    pub fn required_deletion_confirmations(&self) -> u8 {
        let (wallet_type, verified) = {
            let metadata = self.metadata.read();
            (metadata.wallet_type, metadata.verified)
        };

        // cold wallets and xpub-only don't need backup, treat as "verified"
        if wallet_type != WalletType::Hot {
            return 2;
        }

        // hot wallets: verified → 2, not verified → 3
        if verified { 2 } else { 3 }
    }

    /// Returns the warning message for the first delete confirmation dialog
    #[uniffi::method]
    pub fn deletion_warning_message(&self) -> String {
        let (wallet_type, verified) = {
            let metadata = self.metadata.read();
            (metadata.wallet_type, metadata.verified)
        };

        match (wallet_type, verified) {
            (WalletType::Hot, false) => {
                "This wallet is not backed up. Make sure you have your secret words saved before deleting.".to_string()
            }
            _ => "This action cannot be undone.".to_string(),
        }
    }

    // only called from the frontend, to make sure all metadata places are up to date,
    // this would not be needed if we didn't keep a metadata cache in the view model
    #[uniffi::method]
    fn set_wallet_metadata(&self, metadata: WalletMetadata) {
        self.metadata.write().clone_from(&metadata);
    }

    #[uniffi::method]
    pub fn master_fingerprint(&self) -> Option<String> {
        let fingerprint = self.metadata.read().master_fingerprint.clone()?;
        let fingerprint = fingerprint.as_ref();

        if *fingerprint == Fingerprint::default() {
            return None;
        }

        Some(fingerprint.as_uppercase())
    }

    #[uniffi::method]
    pub fn word_validator(&self) -> Result<WordValidator, Error> {
        let mnemonic = Keychain::global()
            .get_wallet_key(&self.metadata.read().id)?
            .ok_or(Error::WalletDoesNotExist)?;

        let validator = WordValidator::new(mnemonic);

        Ok(validator)
    }

    pub fn fees(&self) -> Option<FeeResponse> {
        let cached_fees = *FEES.load().as_ref();

        match cached_fees {
            Some(cached_fees)
                if cached_fees.last_fetched > Instant::now() - Duration::from_secs(30) =>
            {
                cove_tokio::task::spawn(
                    async move { crate::fee_client::get_and_update_fees().await },
                );
            }
            None => {
                cove_tokio::task::spawn(
                    async move { crate::fee_client::get_and_update_fees().await },
                );
            }
            _ => {}
        }

        if let Some(cached_fees) = cached_fees {
            return Some(cached_fees.fees);
        }

        None
    }

    pub async fn fee_rate_options(&self) -> Result<FeeRateOptions, Error> {
        let fee_client = &FEE_CLIENT;
        let fees = fee_client.fetch_and_get_fees().await.map_err_str(Error::FeesError)?;

        Ok(fees.into())
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                match field {
                    SingleOrMany::Single(message) => reconciler.reconcile(message),
                    SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
                }
            }
        });
    }

    /// Finalize a signed PSBT
    #[uniffi::method]
    pub async fn finalize_psbt(&self, psbt: Arc<Psbt>) -> Result<BitcoinTransaction, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let actor = self.actor.clone();
        let psbt = Arc::unwrap_or_clone(psbt).into();
        let transaction = call!(actor.finalize_psbt(psbt)).await.unwrap()?;

        Ok(BitcoinTransaction::from(transaction))
    }

    #[uniffi::method]
    pub async fn switch_to_different_wallet_address_type(
        &self,
        wallet_address_type: WalletAddressType,
    ) -> Result<(), Error> {
        let discovery_state = self.metadata.read().discovery_state.clone();
        match discovery_state {
            DiscoveryState::FoundAddressesFromJson(_vec, json) => {
                let descriptors = match wallet_address_type {
                    WalletAddressType::WrappedSegwit => json.bip49.clone(),
                    WalletAddressType::Legacy => json.bip44.clone(),
                    _ => {
                        error!("trying to switch to native segwit, but already segwit");
                        return Ok(());
                    }
                };

                let descriptors = descriptors.ok_or_else(|| {
                    Error::UnableToSwitch(
                        wallet_address_type,
                        "No descriptors found for address type".to_string(),
                    )
                })?;

                let id = self.id.clone();
                let actor = self.actor.clone();
                call!(
                    actor.switch_descriptor_to_new_address_type(descriptors, wallet_address_type)
                )
                .await
                .map_err(|e| Error::UnableToSwitch(wallet_address_type, e.to_string()))?;
                self.refresh_metadata_from_database()?;

                // reset route as a navigation fallback; actor scan refreshes transactions
                FfiApp::global().load_and_reset_default_route(Route::SelectedWallet(id));
            }

            DiscoveryState::FoundAddressesFromMnemonic(_) => {
                let id = self.id.clone();
                let actor = self.actor.clone();
                call!(actor.switch_mnemonic_to_new_address_type(wallet_address_type))
                    .await
                    .map_err(|e| Error::UnableToSwitch(wallet_address_type, e.to_string()))?;
                self.refresh_metadata_from_database()?;

                debug!("switch done");

                // reset route as a navigation fallback; actor scan refreshes transactions
                FfiApp::global().load_and_reset_default_route(Route::SelectedWallet(id));
            }

            DiscoveryState::Single
            | DiscoveryState::StartedMnemonic
            | DiscoveryState::NoneFound
            | DiscoveryState::ChoseAdressType
            | DiscoveryState::StartedJson(_) => {
                return Err(Error::UnableToSwitch(
                    wallet_address_type,
                    format!("wallet in unexpected discovery state: {discovery_state:?}"),
                ));
            }
        }

        Ok(())
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) {
        let before_metadata = self.metadata.read().clone();
        let mut candidate = before_metadata.clone();

        match action {
            Action::UpdateName(name) => candidate.name = name,

            Action::UpdateColor(color) => candidate.color = color,

            Action::UpdateUnit(unit) => candidate.selected_unit = unit,

            Action::ToggleSensitiveVisibility => {
                candidate.sensitive_visible = !candidate.sensitive_visible;
            }

            Action::ToggleFiatOrBtc => {
                candidate.fiat_or_btc = match candidate.fiat_or_btc {
                    FiatOrBtc::Btc => FiatOrBtc::Fiat,
                    FiatOrBtc::Fiat => FiatOrBtc::Btc,
                };
            }

            Action::UpdateFiatOrBtc(fiat_or_btc) => candidate.fiat_or_btc = fiat_or_btc,

            Action::ToggleFiatBtcPrimarySecondary => {
                const ORDER: &[(FiatOrBtc, Unit); 4] = &[
                    (FiatOrBtc::Btc, Unit::Btc),
                    (FiatOrBtc::Fiat, Unit::Btc),
                    (FiatOrBtc::Btc, Unit::Sat),
                    (FiatOrBtc::Fiat, Unit::Sat),
                ];

                let current = (candidate.fiat_or_btc, candidate.selected_unit);

                let current_index = ORDER
                    .iter()
                    .position(|option| option == &current)
                    .expect("all options covered");

                let next_index = (current_index + 1) % ORDER.len();
                let (fiat_or_btc, unit) = ORDER[next_index];

                candidate.fiat_or_btc = fiat_or_btc;
                candidate.selected_unit = unit;
            }

            Action::ToggleDetailsExpanded => {
                candidate.details_expanded = !candidate.details_expanded;
            }

            Action::SelectCurrentWalletAddressType => {
                candidate.discovery_state = DiscoveryState::ChoseAdressType;
            }

            Action::ToggleShowLabels => candidate.show_labels = !candidate.show_labels,

            Action::SelectedWalletDisappeared => {
                self.shutdown_actors();
                return;
            }

            Action::StartTransactionWatcher(tx_id) => {
                let tx_id = tx_id.as_ref().0;
                send!(self.actor.start_transaction_watcher(tx_id));
                return;
            }

            Action::OpenReceiveAddress => {
                send!(self.actor.open_receive_address_intent());
                return;
            }

            Action::CreateNewReceiveAddress => {
                send!(self.actor.create_new_receive_address_intent());
                return;
            }

            Action::CloseReceiveAddress(request_id) => {
                send!(self.actor.close_receive_address(request_id));
                return;
            }
        }

        let candidate = match Database::global().wallets.update_wallet_metadata(candidate.clone()) {
            Ok(candidate) => candidate,
            Err(error) => {
                error!("Unable to update wallet metadata: {error:?}");
                return;
            }
        };

        *self.metadata.write() = candidate.clone();
        self.reconciler.send(Message::WalletMetadataChanged(Box::new(candidate.clone())));
        let scan_status = self.current_scan_status();
        self.reconciler.send(Message::LedgerStateChanged(
            WalletLedgerState::from_metadata_and_scan_status(&candidate, &scan_status),
        ));
        CLOUD_BACKUP_MANAGER.handle_wallet_metadata_update(&before_metadata, &candidate);
    }

    pub fn shutdown(&self) {
        self.shutdown_actors();
    }

    fn shutdown_actors(&self) {
        send!(self.actor.shutdown());

        if let Some(discovery_scanner) = &self.discovery_scanner {
            send!(discovery_scanner.shutdown());
        }
    }
}

impl RustWalletManager {
    fn current_scan_status(&self) -> WalletScanStatus {
        self.scan_status.read().clone()
    }

    fn current_metadata(&self) -> WalletMetadata {
        let cached_metadata = self.metadata.read().clone();
        let database_metadata = Database::global().wallets().get(
            &self.id,
            cached_metadata.network,
            cached_metadata.wallet_mode,
        );

        match database_metadata {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                let id = &self.id;
                let network = cached_metadata.network;
                let wallet_mode = cached_metadata.wallet_mode;
                warn!(
                    "wallet metadata missing id={id:?} network={network:?} wallet_mode={wallet_mode}, using cached metadata"
                );
                cached_metadata
            }
            Err(error) => {
                let id = &self.id;
                let network = cached_metadata.network;
                let wallet_mode = cached_metadata.wallet_mode;
                warn!(
                    "unable to load wallet metadata id={id:?} network={network:?} wallet_mode={wallet_mode}: {error}, using cached metadata"
                );
                cached_metadata
            }
        }
    }

    fn ensure_ledger_ready_for_spend(&self) -> Result<(), Error> {
        if self.current_metadata().internal.performed_full_scan_at.is_some() {
            return Ok(());
        }

        Err(Error::InitialScanIncomplete)
    }

    fn refresh_metadata_from_database(&self) -> Result<WalletMetadata, Error> {
        let before_metadata = self.metadata.read().clone();
        let metadata = Database::global()
            .wallets()
            .get(&self.id, before_metadata.network, before_metadata.wallet_mode)?
            .ok_or(Error::WalletDoesNotExist)?;

        *self.metadata.write() = metadata.clone();
        self.reconciler.send(Message::WalletMetadataChanged(Box::new(metadata.clone())));
        let scan_status = self.current_scan_status();

        // address type switches may already have reconciled idle; repeating it is harmless
        self.reconciler.send(Message::LedgerStateChanged(
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &scan_status),
        ));
        CLOUD_BACKUP_MANAGER.handle_wallet_metadata_update(&before_metadata, &metadata);

        Ok(metadata)
    }
}

const PREVIEW_FULL_SCAN_COMPLETED_AT: u64 = u64::MAX;

fn preview_ledger_ready_metadata(mut metadata: WalletMetadata) -> WalletMetadata {
    metadata.internal.performed_full_scan_at.get_or_insert(PREVIEW_FULL_SCAN_COMPLETED_AT);
    metadata
}

#[uniffi::export]
impl RustWalletManager {
    #[uniffi::constructor]
    pub fn preview_new_wallet() -> Self {
        let metadata = WalletMetadata::preview_new();
        Self::preview_new_wallet_with_metadata(metadata)
    }

    #[uniffi::constructor]
    pub fn preview_new_wallet_with_metadata(metadata: WalletMetadata) -> Self {
        let metadata = preview_ledger_ready_metadata(metadata);
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::preview_new_wallet_with_metadata(metadata.clone());
        let label_manager = LabelManager::new(wallet.metadata.id.clone()).into();
        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_actor = WalletActor::new(wallet, sender.clone(), scan_status.clone())
            .expect("failed to open wallet database for preview wallet");
        let actor = task::spawn_actor(wallet_actor);

        Self {
            id: metadata.id.clone(),
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
            scan_status,
            label_manager,
            initial_state: WalletInitialState {
                metadata: metadata.clone(),
                ledger_state: WalletLedgerState::from_metadata_and_scan_status(
                    &metadata,
                    &WalletScanStatus::Idle,
                ),
                load_state: WalletLoadState::Loading,
                scan_status: WalletScanStatus::Idle,
                balance_presentation: BalancePresentation::provisional(),
                balance: Arc::new(Balance::zero()),
                unsigned_transactions: Vec::new(),
            },
            discovery_scanner: None,
        }
    }
}

impl Drop for RustWalletManager {
    fn drop(&mut self) {
        self.shutdown();
        debug!("[DROP] Wallet View manager: {}", self.id);
    }
}

/// If a hot wallet's private key is missing from the keychain, downgrade it to
/// watch-only and queue a `HotWalletKeyMissing` notification so the UI can alert the user
fn downgrade_and_notify_if_needed(
    metadata: WalletMetadata,
    deferred: &mut DeferredSender<Message>,
) -> Result<WalletMetadata, Error> {
    if metadata.wallet_type != WalletType::Hot {
        return Ok(metadata);
    }

    let has_private_key = match Keychain::global().get_wallet_key(&metadata.id) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(error) => {
            return Err(Error::UnknownError(format!(
                "failed to read keychain for {}: {error}",
                metadata.id
            )));
        }
    };

    if has_private_key {
        return Ok(metadata);
    }

    let id = metadata.id.clone();
    warn!("hot wallet {id} is missing private key in keychain, downgrading to watch-only",);

    let mut updated = metadata;
    updated.wallet_type = WalletType::WatchOnly;
    updated.hardware_metadata = None;

    let updated =
        Database::global().wallets.update_wallet_metadata(updated.clone()).map_err(|e| {
            Error::UnknownError(format!("failed to persist watch-only downgrade for {id}: {e}",))
        })?;

    deferred.queue(Message::HotWalletKeyMissing(updated.id.clone()));
    Ok(updated)
}

fn wallet_account_number(id: &WalletId) -> Option<u32> {
    use cove_bdk::descriptor_ext::DescriptorExt as _;

    match Wallet::try_load_persisted(id.clone()) {
        Ok(wallet) => {
            wallet.bdk.public_descriptor(bdk_wallet::KeychainKind::External).account_index()
        }
        Err(_) => Keychain::global()
            .get_public_descriptor(id)
            .ok()
            .flatten()
            .and_then(|(external, _)| external.account_index()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PREVIEW_FULL_SCAN_COMPLETED_AT, WalletLedgerState, WalletScanStatus,
        preview_ledger_ready_metadata,
    };

    use crate::wallet::metadata::WalletMetadata;

    #[test]
    fn preview_wallet_metadata_is_ledger_ready_for_spend() {
        let metadata = preview_ledger_ready_metadata(WalletMetadata::preview_new());

        assert_eq!(metadata.internal.performed_full_scan_at, Some(PREVIEW_FULL_SCAN_COMPLETED_AT));
        assert_eq!(
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &WalletScanStatus::Idle),
            WalletLedgerState::Complete
        );
    }
}

#[uniffi::export]
impl WalletLoadState {
    fn is_equal(&self, other: WalletLoadState) -> bool {
        self == &other
    }
}
