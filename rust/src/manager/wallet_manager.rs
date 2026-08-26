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

use std::sync::Arc;

use act_zero::{Addr, call, send};
use actor::WalletActor;
pub use balance_presentation::BalancePresentation;
use bdk_wallet::{AddUtxoError, error::CreateTxError};
use futures::channel::oneshot;
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
    database::{Database, error::DatabaseError, wallet_data::WalletDataDb},
    discovery_scanner::{ScannerResponse, WalletDiscoveryScanner},
    fee_client::{FEE_CLIENT, FeeClientError, FeeResponse},
    fiat::client::PriceResponse,
    keychain::{Keychain, KeychainError},
    label_manager::LabelManager,
    psbt::Psbt,
    router::Route,
    transaction::{
        Amount, Transaction, TransactionDetailsPresentation, TxId, Unit, ffi::BitcoinTransaction,
        unsigned_transaction::UnsignedTransaction,
    },
    wallet::{
        AddressInfo, Wallet, WalletAddressType, WalletError,
        balance::Balance,
        fingerprint::Fingerprint,
        metadata::{DiscoveryState, FiatOrBtc, WalletColor, WalletId, WalletMetadata, WalletType},
    },
    wallet_lifecycle::{
        WalletActorRegistration, WalletLifecycleFailure, WalletManagerLifecycleToken,
    },
    word_validator::WordValidator,
};

use cove_types::confirm::{ConfirmDetails, SplitOutput};
use cove_types::{confirm::AddressAndAmount, fees::FeeRateOptions};

use super::{
    coin_control_manager::RustCoinControlManager,
    deferred_sender::{self, DeferredSender},
    reconcile_channel::ReconcileChannel,
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
    TransactionDetailsUpdated(Arc<TransactionDetailsPresentation>),

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
    PayjoinPollingStarted { deadline_secs: u64 },
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

#[derive(Debug, Clone)]
pub(crate) struct WalletSnapshot {
    pub balance: Balance,
    pub transactions: Vec<Transaction>,
}

impl WalletSnapshot {
    pub(crate) fn from_wallet(wallet: &Wallet) -> Self {
        Self { balance: wallet.balance(), transactions: wallet.transactions() }
    }

    fn load_state(
        &self,
        ledger_state: WalletLedgerState,
        scan_status: &WalletScanStatus,
    ) -> WalletLoadState {
        match (ledger_state, scan_status) {
            (_, WalletScanStatus::Scanning(_) | WalletScanStatus::ScanningPendingProgress(_)) => {
                WalletLoadState::Scanning(self.transactions.clone())
            }
            (WalletLedgerState::Complete, WalletScanStatus::Idle) => {
                WalletLoadState::Loaded(self.transactions.clone())
            }
            (WalletLedgerState::InitialScanIncomplete(_), WalletScanStatus::Idle)
                if self.transactions.is_empty() =>
            {
                WalletLoadState::Loading
            }
            (WalletLedgerState::InitialScanIncomplete(_), WalletScanStatus::Idle) => {
                WalletLoadState::Scanning(self.transactions.clone())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum WalletBootstrapUnsignedTransactions {
    Database(WalletId),
    InMemory(Vec<Arc<UnsignedTransaction>>),
}

impl WalletBootstrapUnsignedTransactions {
    pub(crate) fn database(wallet_id: WalletId) -> Self {
        Self::Database(wallet_id)
    }

    pub(crate) fn in_memory(unsigned_transactions: Vec<Arc<UnsignedTransaction>>) -> Self {
        Self::InMemory(unsigned_transactions)
    }

    pub(crate) const fn is_in_memory(&self) -> bool {
        matches!(self, Self::InMemory(_))
    }

    fn load(&self) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
        match self {
            Self::Database(wallet_id) => unsigned_transactions_for_wallet(wallet_id),
            Self::InMemory(unsigned_transactions) => Ok(unsigned_transactions.clone()),
        }
    }
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
    pub reconciler: ReconcileChannel<Message>,
    scan_status: Arc<RwLock<WalletScanStatus>>,
    wallet_snapshot: Arc<RwLock<WalletSnapshot>>,
    unsigned_transactions: WalletBootstrapUnsignedTransactions,

    label_manager: Arc<LabelManager>,
    discovery_scanner: Option<Addr<WalletDiscoveryScanner>>,
    lifecycle: Option<WalletManagerLifecycleToken>,
    _actor_registration: Option<WalletActorRegistration>,
}

pub type Error = WalletManagerError;
#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum WalletManagerError {
    #[error("failed to get selected wallet: {0}")]
    GetSelectedWalletError(String),

    #[error("wallet does not exist")]
    WalletDoesNotExist,

    #[error("operation is unavailable for a preview wallet")]
    PreviewOperationUnavailable,

    #[error("unable to delete wallet: {0}")]
    DeleteWalletError(String),

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

    #[error("send amount is below the dust limit")]
    OutputBelowDustLimit,

    #[error("selected UTXOs include locked outputs")]
    LockedOutputsSelected,

    #[error("Unable to get confirm details, {0}")]
    GetConfirmDetailsError(String),

    #[error("signing failed: {0}")]
    SigningError(String),

    #[error("broadcast failed: {0}")]
    BroadcastError(String),

    #[error("{0}")]
    PayjoinSessionError(String),

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

    #[error("unable to load pending unsigned transactions: {0}")]
    PendingUnsignedTransactionsLoadError(String),

    #[error("Receive address error: {0}")]
    ReceiveAddressError(String),

    #[error("wallet manager is closed")]
    ManagerClosed,

    #[error("wallet lifecycle error: {0}")]
    WalletLifecycle(WalletLifecycleFailure),

    #[error("address type switch committed with recovery pending")]
    AddressTypeSwitchCommittedWithRecoveryPending {
        address_type: WalletAddressType,
        failures: Vec<AddressTypeSwitchRecoveryFailure>,
    },
}

/// Repair phase that failed after an address-store rename committed
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AddressTypeSwitchRecoveryStage {
    /// File or parent-directory durability synchronization
    Durability,
    /// Reloading the newly published wallet store
    StoreReload,
    /// Exact metadata commit
    Metadata,
    /// Scan-state reset or restart
    ScanRestart,
}

/// One post-publication repair failure
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct AddressTypeSwitchRecoveryFailure {
    /// Repair phase that failed
    pub stage: AddressTypeSwitchRecoveryStage,
    /// Underlying source error without duplicated phase context
    pub source_detail: String,
}

impl From<WalletLifecycleFailure> for WalletManagerError {
    fn from(error: WalletLifecycleFailure) -> Self {
        match error {
            WalletLifecycleFailure::ManagerRecoveryRequired { .. } => Self::ManagerClosed,
            other => Self::WalletLifecycle(other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletManagerBuildTxError {
    #[error(transparent)]
    Create(#[from] CreateTxError),

    #[error(transparent)]
    Psbt(#[from] bitcoin::psbt::Error),
}

impl From<WalletManagerBuildTxError> for WalletManagerError {
    fn from(error: WalletManagerBuildTxError) -> Self {
        match error {
            WalletManagerBuildTxError::Create(CreateTxError::OutputBelowDustLimit(_)) => {
                Self::OutputBelowDustLimit
            }
            WalletManagerBuildTxError::Create(CreateTxError::CoinSelection(error)) => {
                Self::InsufficientFunds(error.to_string())
            }
            error => Self::BuildTxError(error.to_string()),
        }
    }
}

impl From<CreateTxError> for WalletManagerError {
    fn from(error: CreateTxError) -> Self {
        Self::from(WalletManagerBuildTxError::from(error))
    }
}

impl From<AddUtxoError> for WalletManagerError {
    fn from(error: AddUtxoError) -> Self {
        Self::AddUtxosError(error.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletManagerFeesError {
    #[error(transparent)]
    Fetch(#[from] FeeClientError),

    #[error(transparent)]
    Psbt(#[from] bitcoin::psbt::Error),
}

impl From<WalletManagerFeesError> for WalletManagerError {
    fn from(error: WalletManagerFeesError) -> Self {
        Self::FeesError(error.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{context}: {source}")]
pub(crate) struct WalletManagerPendingUnsignedTransactionsLoadError {
    context: String,

    #[source]
    source: crate::database::Error,
}

impl WalletManagerPendingUnsignedTransactionsLoadError {
    pub(crate) fn new(context: impl Into<String>, source: crate::database::Error) -> Self {
        Self { context: context.into(), source }
    }
}

impl From<WalletManagerPendingUnsignedTransactionsLoadError> for WalletManagerError {
    fn from(error: WalletManagerPendingUnsignedTransactionsLoadError) -> Self {
        Self::PendingUnsignedTransactionsLoadError(error.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub(crate) struct WalletManagerDatabaseCorruptionError {
    id: WalletId,

    #[source]
    source: crate::database::wallet_data::WalletDataError,
}

impl WalletManagerDatabaseCorruptionError {
    pub(crate) fn new(id: WalletId, source: crate::database::wallet_data::WalletDataError) -> Self {
        Self { id, source }
    }
}

impl From<WalletManagerDatabaseCorruptionError> for WalletManagerError {
    fn from(error: WalletManagerDatabaseCorruptionError) -> Self {
        Self::DatabaseCorruption { id: error.id, error: error.source.to_string() }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub(crate) struct WalletManagerUnableToSwitchError {
    wallet_address_type: WalletAddressType,

    #[source]
    source: oneshot::Canceled,
}

impl WalletManagerUnableToSwitchError {
    pub(crate) const fn new(
        wallet_address_type: WalletAddressType,
        source: oneshot::Canceled,
    ) -> Self {
        Self { wallet_address_type, source }
    }
}

impl From<WalletManagerUnableToSwitchError> for WalletManagerError {
    fn from(error: WalletManagerUnableToSwitchError) -> Self {
        Self::UnableToSwitch(error.wallet_address_type, error.source.to_string())
    }
}

fn initial_state_from_snapshot(
    metadata: WalletMetadata,
    scan_status: WalletScanStatus,
    snapshot: WalletSnapshot,
    unsigned_transactions: Vec<Arc<UnsignedTransaction>>,
) -> WalletInitialState {
    let ledger_state = WalletLedgerState::from_metadata_and_scan_status(&metadata, &scan_status);
    let balance_presentation = BalancePresentation::for_ledger_state(ledger_state);
    let load_state = snapshot.load_state(ledger_state, &scan_status);

    WalletInitialState {
        metadata,
        ledger_state,
        load_state,
        scan_status,
        balance_presentation,
        balance: Arc::new(snapshot.balance),
        unsigned_transactions,
    }
}

fn initial_state_from_snapshot_with_pending_unsigned_transactions(
    metadata: WalletMetadata,
    scan_status: WalletScanStatus,
    snapshot: WalletSnapshot,
    unsigned_transactions: Result<Vec<Arc<UnsignedTransaction>>, Error>,
) -> WalletInitialState {
    let unsigned_transactions = match unsigned_transactions {
        Ok(unsigned_transactions) => unsigned_transactions,
        Err(error) => {
            warn!("unable to load pending unsigned transactions for initial wallet state: {error}");
            Vec::new()
        }
    };

    initial_state_from_snapshot(metadata, scan_status, snapshot, unsigned_transactions)
}

fn unsigned_transactions_for_wallet(
    wallet_id: &WalletId,
) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
    let db = Database::global();
    let context = format!("wallet id={wallet_id:?}");
    let txns = db.unsigned_transactions().get_by_wallet_id(wallet_id).map_err(|source| {
        WalletManagerPendingUnsignedTransactionsLoadError::new(context.clone(), source)
    })?;

    let txns =
        txns.into_iter().map(|txn| Arc::new(txn.into())).collect::<Vec<Arc<UnsignedTransaction>>>();

    Ok(txns)
}

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    /// Returns the bootstrap wallet snapshot used before reconcile messages arrive
    #[uniffi::method]
    pub fn initial_state(&self) -> WalletInitialState {
        let unsigned_transactions = if self.ensure_active().is_ok() {
            self.unsigned_transactions.load()
        } else {
            Ok(Vec::new())
        };

        initial_state_from_snapshot_with_pending_unsigned_transactions(
            self.current_metadata(),
            self.current_scan_status(),
            self.wallet_snapshot.read().clone(),
            unsigned_transactions,
        )
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
        let unspent =
            call!(self.actor.list_unspent()).await.map_err(|_| Error::ManagerClosed)??;
        let lifecycle = self.lifecycle.clone().ok_or(Error::PreviewOperationUnavailable)?;

        let manager = RustCoinControlManager::new(metadata, unspent, lifecycle);
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
        self.ensure_active()?;
        let fee_client = &FEE_CLIENT;
        let fees = fee_client.fetch_and_get_fees().await.map_err(WalletManagerFeesError::from)?;

        fees.fee_rate_options().map_err_str(Error::FeesError)
    }

    #[uniffi::method]
    pub async fn first_address(&self) -> Result<AddressInfo, Error> {
        self.ensure_active()?;
        let address_info = call!(self.actor.address_at(0))
            .await
            .map_err(|_| Error::UnknownError("failed to get first address".to_string()))?;

        Ok(address_info)
    }

    #[uniffi::method]
    pub fn save_unsigned_transaction(&self, details: Arc<ConfirmDetails>) -> Result<(), Error> {
        self.ensure_active()?;
        self.save_unsigned_transaction_internal(details)
    }

    #[uniffi::method]
    pub async fn split_transaction_outputs(
        &self,
        outputs: Vec<AddressAndAmount>,
    ) -> Result<SplitOutput, Error> {
        self.ensure_active()?;
        let outputs = call!(self.actor.split_transaction_outputs(outputs))
            .await
            .map_err(|_| Error::UnknownError("failed to split outputs".to_string()))?;

        Ok(outputs)
    }

    #[uniffi::method]
    pub fn get_unsigned_transactions(&self) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
        self.ensure_active()?;
        self.get_unsigned_transactions_internal()
    }

    #[uniffi::method]
    pub async fn get_transactions(&self) -> Result<(), Error> {
        self.ensure_active()?;
        let txns = call!(self.actor.transactions()).await.map_err(|_| Error::ManagerClosed)?;

        self.reconciler.send(Message::UpdatedTransactions(txns));
        Ok(())
    }

    #[uniffi::method]
    pub fn delete_unsigned_transaction(&self, tx_id: Arc<TxId>) -> Result<(), Error> {
        self.ensure_active()?;
        self.delete_unsigned_transaction_internal(tx_id)
    }

    #[uniffi::method]
    pub async fn balance(&self) -> Result<Balance, Error> {
        self.ensure_active()?;
        call!(self.actor.balance()).await.map_err(|_| Error::ManagerClosed)
    }

    #[uniffi::method]
    pub async fn unlocked_spendable_balance(&self) -> Result<Amount, Error> {
        self.ensure_active()?;
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
        call!(self.actor.initiate_payment(psbt.into(), payjoin_endpoint))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        self.force_wallet_scan().await?;

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

        call!(self.actor.broadcast_transaction(txn.into()))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        if let Err(error) = self.delete_unsigned_transaction(tx_id.into()) {
            error!("unable to delete unsigned transaction record: {error}");
        }

        self.force_wallet_scan().await?;

        Ok(())
    }

    #[uniffi::method]
    pub async fn cancel_payjoin(&self) -> Result<(), Error> {
        call!(self.actor.cancel_payjoin()).await.unwrap()?;
        let _ = self.force_wallet_scan().await;
        Ok(())
    }

    #[uniffi::method]
    pub async fn current_block_height(&self) -> Result<u32, Error> {
        self.ensure_active()?;
        let height = call!(self.actor.get_height(false))
            .await
            .map_err(|_| Error::GetHeightError)?
            .map_err(|_| Error::GetHeightError)?;

        Ok(height as u32)
    }

    #[uniffi::method]
    pub async fn force_update_height(&self) -> Result<u32, Error> {
        self.ensure_active()?;
        let height = call!(self.actor.get_height(true))
            .await
            .map_err(|_| Error::GetHeightError)?
            .map_err(|_| Error::GetHeightError)?;

        Ok(height as u32)
    }

    #[uniffi::method]
    pub async fn transaction_details(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<Arc<TransactionDetailsPresentation>, Error> {
        self.ensure_active()?;
        let tx_id = Arc::unwrap_or_clone(tx_id);
        let actor = self.actor.clone();

        let presentation = task::spawn(async move {
            call!(actor.transaction_details(tx_id))
                .await
                .map_err_str(Error::TransactionDetailsError)
        })
        .await
        .map_err_str(Error::TransactionDetailsError)??;

        let tx_id = presentation.tx_id().0;
        send!(self.actor.monitor_transaction_confirmation(tx_id));

        Ok(Arc::new(presentation))
    }

    /// Get address at the given index
    #[uniffi::method]
    pub async fn address_at(&self, index: u32) -> Result<AddressInfo, Error> {
        self.ensure_active()?;
        let address =
            call!(self.actor.address_at(index)).await.map_err(|_| Error::ActorNotFound)?;

        Ok(address)
    }

    #[uniffi::method]
    pub async fn delete_wallet(&self) -> Result<(), Error> {
        self.delete_wallet_internal().await
    }

    /// Retry a deletion after a typed shutdown block
    #[uniffi::method]
    pub async fn retry_delete_wallet(
        &self,
        attempt_id: crate::wallet_lifecycle::ShutdownAttemptId,
    ) -> Result<(), Error> {
        self.retry_delete_wallet_internal(attempt_id).await
    }

    #[uniffi::method]
    pub async fn set_wallet_type(&self, wallet_type: WalletType) -> Result<(), Error> {
        self.ensure_active()?;
        self.set_wallet_type_internal(wallet_type).await
    }

    #[uniffi::method]
    pub async fn validate_metadata(&self) -> Result<(), Error> {
        self.ensure_active()?;
        self.validate_metadata_internal().await
    }

    #[uniffi::method]
    pub async fn start_wallet_scan(&self) -> Result<(), Error> {
        self.ensure_active()?;
        debug!("start_wallet_scan: {}", self.id);

        send!(self.actor.wallet_scan_and_notify(false));

        Ok(())
    }

    #[uniffi::method]
    pub async fn force_wallet_scan(&self) -> Result<(), Error> {
        self.ensure_active()?;
        debug!("force_wallet_scan: {}", self.id);

        send!(self.actor.wallet_scan_and_notify(true));
        Ok(())
    }

    #[uniffi::method]
    pub async fn rescan_wallet_with_gap_limit(&self, gap_limit: u32) -> Result<(), Error> {
        self.ensure_active()?;
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
    pub async fn mark_wallet_as_verified(&self) -> Result<(), Error> {
        self.ensure_active()?;
        self.mark_wallet_as_verified_internal().await
    }

    #[uniffi::method]
    pub fn wallet_metadata(&self) -> WalletMetadata {
        self.wallet_metadata_internal()
    }

    #[uniffi::method]
    pub fn non_default_account_number(&self) -> Result<Option<u32>, Error> {
        self.ensure_active()?;
        if !self.uses_persistent_storage() {
            return Ok(None);
        }

        Ok(wallet_account_number(&self.id).filter(|account| *account != 0))
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
    pub fn deletion_warning_message(&self) -> Result<String, Error> {
        self.ensure_active()?;
        let (wallet_type, verified) = {
            let metadata = self.metadata.read();
            (metadata.wallet_type, metadata.verified)
        };

        if wallet_type == WalletType::Hot && !verified {
            let message = if self.has_recovery_words()? {
                "This wallet is not backed up. Make sure you have your secret words saved before deleting."
                    .to_string()
            } else {
                "This wallet is not backed up. Make sure you have your extended private key saved before deleting."
                    .to_string()
            };

            return Ok(message);
        }

        Ok("This action cannot be undone.".to_string())
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
        self.ensure_active()?;
        let mnemonic = Keychain::global()
            .get_wallet_key(&self.metadata.read().id)?
            .ok_or(Error::WalletDoesNotExist)?;

        let validator = WordValidator::new(mnemonic);

        Ok(validator)
    }

    /// Returns whether this hot wallet is backed by BIP39 recovery words
    #[uniffi::method]
    pub fn has_recovery_words(&self) -> Result<bool, Error> {
        self.ensure_active()?;
        let secret = Keychain::global()
            .get_wallet_secret(&self.metadata.read().id)
            .map_err(Error::SecretRetrievalError)?;

        Ok(secret.is_some_and(|secret| secret.as_mnemonic().is_some()))
    }

    /// Returns whether this hot wallet is backed by an extended private key (no mnemonic)
    #[uniffi::method]
    pub fn has_xprv_secret(&self) -> Result<bool, Error> {
        self.ensure_active()?;
        let secret = Keychain::global()
            .get_wallet_secret(&self.metadata.read().id)
            .map_err(Error::SecretRetrievalError)?;

        Ok(secret.is_some_and(|secret| secret.as_xprv().is_some()))
    }

    /// Returns the wallet's master extended private key string for export
    ///
    /// Note: the returned String crosses FFI into a Swift/Kotlin string that cannot be
    /// zeroized; same limitation as displaying the mnemonic words
    #[uniffi::method]
    pub fn expose_xprv(&self) -> Result<String, Error> {
        self.ensure_active()?;
        let secret = Keychain::global()
            .get_wallet_secret(&self.metadata.read().id)?
            .ok_or(Error::WalletDoesNotExist)?;
        let xprv = secret
            .as_xprv()
            .ok_or(Error::SecretRetrievalError(KeychainError::WalletSecretTypeMismatch))?;

        Ok(xprv.expose().to_string())
    }

    pub fn fees(&self) -> Option<FeeResponse> {
        FEE_CLIENT.fees()
    }

    pub async fn fee_rate_options(&self) -> Result<FeeRateOptions, Error> {
        self.ensure_active()?;
        let fee_client = &FEE_CLIENT;
        let fees = fee_client.fetch_and_get_fees().await.map_err(WalletManagerFeesError::from)?;

        fees.fee_rate_options().map_err_str(Error::FeesError)
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen(move |field| match field {
            SingleOrMany::Single(message) => reconciler.reconcile(message),
            SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
        });
    }

    /// Finalize a signed PSBT
    #[uniffi::method]
    pub async fn finalize_psbt(&self, psbt: Arc<Psbt>) -> Result<BitcoinTransaction, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let actor = self.actor.clone();
        let psbt = Arc::unwrap_or_clone(psbt).into();
        let transaction =
            call!(actor.finalize_psbt(psbt)).await.map_err(|_| Error::ManagerClosed)??;

        Ok(BitcoinTransaction::from(transaction))
    }

    #[uniffi::method]
    pub async fn switch_to_different_wallet_address_type(
        &self,
        wallet_address_type: WalletAddressType,
    ) -> Result<(), Error> {
        self.ensure_active()?;
        let discovery_state = self.metadata.read().discovery_state.clone();
        let switch_result = match discovery_state {
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

                let actor = self.actor.clone();
                call!(actor.switch_descriptor_to_new_address_type(descriptors, wallet_address_type))
                    .await
                    .map_err(|source| {
                        WalletManagerUnableToSwitchError::new(wallet_address_type, source)
                    })?
            }

            DiscoveryState::FoundAddressesFromMnemonic(_)
            | DiscoveryState::FoundAddressesFromXprv(_) => {
                let actor = self.actor.clone();
                let result =
                    call!(actor.switch_private_wallet_to_new_address_type(wallet_address_type))
                        .await
                        .map_err(|source| {
                            WalletManagerUnableToSwitchError::new(wallet_address_type, source)
                        })?;

                debug!("switch done");
                result
            }

            DiscoveryState::Single
            | DiscoveryState::StartedMnemonic
            | DiscoveryState::StartedXprv
            | DiscoveryState::NoneFound
            | DiscoveryState::ChoseAdressType
            | DiscoveryState::StartedJson(_) => Err(Error::UnableToSwitch(
                wallet_address_type,
                format!("wallet in unexpected discovery state: {discovery_state:?}"),
            )),
        };

        if matches!(
            &switch_result,
            Ok(()) | Err(Error::AddressTypeSwitchCommittedWithRecoveryPending { .. })
        ) {
            // the exported manager owns the one route reset after publication commits
            FfiApp::global().load_and_reset_default_route(Route::SelectedWallet(self.id.clone()));
        }

        switch_result
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) -> Result<(), Error> {
        self.ensure_active()?;
        match action {
            Action::OpenReceiveAddress => {
                send!(self.actor.open_receive_address_intent());
            }

            Action::CreateNewReceiveAddress => {
                send!(self.actor.create_new_receive_address_intent());
            }

            Action::CloseReceiveAddress(request_id) => {
                send!(self.actor.close_receive_address(request_id));
            }

            action => {
                send!(self.actor.dispatch_metadata_action(action));
            }
        }

        Ok(())
    }
}

impl RustWalletManager {
    fn uses_persistent_storage(&self) -> bool {
        !self.unsigned_transactions.is_in_memory()
    }

    fn current_scan_status(&self) -> WalletScanStatus {
        self.scan_status.read().clone()
    }

    fn current_metadata(&self) -> WalletMetadata {
        self.metadata.read().clone()
    }

    fn ensure_ledger_ready_for_spend(&self) -> Result<(), Error> {
        self.ensure_active()?;
        if self.current_metadata().internal.performed_full_scan_at.is_some() {
            return Ok(());
        }

        Err(Error::InitialScanIncomplete)
    }

    pub(crate) fn ensure_active(&self) -> Result<(), Error> {
        match &self.lifecycle {
            Some(lifecycle) => lifecycle.ensure_active(),
            None => Ok(()),
        }
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
        let channel = ReconcileChannel::new(100);

        let wallet = Wallet::preview_new_wallet_with_metadata(metadata.clone());
        let wallet_data_db = WalletDataDb::new_in_memory(wallet.metadata.id.clone())
            .expect("failed to open in-memory wallet data database for preview wallet");
        let label_manager = LabelManager::new_with_db(wallet_data_db.clone()).into();
        let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
        let unsigned_transactions = WalletBootstrapUnsignedTransactions::in_memory(Vec::new());
        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let shared_metadata = Arc::new(RwLock::new(metadata.clone()));
        let wallet_actor = WalletActor::new_with_metadata_and_db(
            wallet,
            channel.raw_sender(),
            scan_status.clone(),
            wallet_snapshot.clone(),
            wallet_data_db,
            shared_metadata.clone(),
        );
        let actor = task::spawn_actor(wallet_actor);

        Self {
            id: metadata.id.clone(),
            actor,
            metadata: shared_metadata,
            reconciler: channel,
            scan_status,
            wallet_snapshot,
            unsigned_transactions,
            label_manager,
            discovery_scanner: None,
            lifecycle: None,
            _actor_registration: None,
        }
    }
}

impl Drop for RustWalletManager {
    fn drop(&mut self) {
        if self._actor_registration.is_none() {
            send!(self.actor.shutdown());

            if let Some(discovery_scanner) = &self.discovery_scanner {
                send!(discovery_scanner.shutdown());
            }
        }
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

    let has_private_key = match Keychain::global().get_wallet_secret(&metadata.id) {
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
    use std::sync::Arc;

    use bdk_wallet::{coin_selection::InsufficientFunds, error::CreateTxError};
    use bitcoin::Amount;

    use super::{
        Balance, BalancePresentation, Error, PREVIEW_FULL_SCAN_COMPLETED_AT, RustWalletManager,
        WalletLedgerState, WalletLoadState, WalletManagerError, WalletScanPhase,
        WalletScanProgress, WalletScanStatus, WalletSnapshot, initial_state_from_snapshot,
        initial_state_from_snapshot_with_pending_unsigned_transactions, ledger_state,
        preview_ledger_ready_metadata,
    };

    use crate::transaction::{SentAndReceived, Transaction, TxId, UnconfirmedTransaction};
    use crate::wallet::metadata::WalletMetadata;

    fn progress() -> WalletScanProgress {
        WalletScanProgress {
            phase: WalletScanPhase::Full,
            checked: 0,
            gap: 0,
            stop_gap: 150,
            progress_basis_points: 0,
        }
    }

    fn transaction() -> Transaction {
        Transaction::Unconfirmed(Arc::new(UnconfirmedTransaction {
            txid: TxId::preview_new(),
            sent_and_received: SentAndReceived::preview_new(),
            last_seen: 0,
            fiat: None,
            labels: Default::default(),
        }))
    }

    #[test]
    fn preview_manager_construction_has_no_persistent_side_effects() {
        crate::test_support::ensure_tokio_runtime();

        let metadata = WalletMetadata::preview_new();
        let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id);
        let wallet_data_paths =
            crate::database::wallet_data::wallet_data_artifact_paths(&metadata.id);

        let manager = RustWalletManager::preview_new_wallet_with_metadata(metadata);

        assert!(manager.get_unsigned_transactions().unwrap().is_empty());
        assert_eq!(manager.non_default_account_number(), Ok(None));
        assert!(bdk_paths.iter().all(|path| !path.exists()));
        assert!(wallet_data_paths.iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn preview_manager_rejects_xpub_export_without_persistent_side_effects() {
        crate::test_support::ensure_tokio_runtime();

        let metadata = WalletMetadata::preview_new();
        let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id);
        let wallet_data_paths =
            crate::database::wallet_data::wallet_data_artifact_paths(&metadata.id);
        let manager = RustWalletManager::preview_new_wallet_with_metadata(metadata);

        assert!(matches!(
            manager.export_xpub_for_share().await,
            Err(Error::PreviewOperationUnavailable)
        ));
        assert!(bdk_paths.iter().all(|path| !path.exists()));
        assert!(wallet_data_paths.iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn preview_manager_rejects_wallet_deletion() {
        crate::test_support::ensure_tokio_runtime();

        let manager = RustWalletManager::preview_new_wallet();

        assert_eq!(manager.delete_wallet().await, Err(Error::PreviewOperationUnavailable));
    }

    #[test]
    fn preview_wallet_metadata_is_ledger_ready_for_spend() {
        let metadata = preview_ledger_ready_metadata(WalletMetadata::preview_new());

        assert_eq!(metadata.internal.performed_full_scan_at, Some(PREVIEW_FULL_SCAN_COMPLETED_AT));
        assert_eq!(
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &WalletScanStatus::Idle),
            WalletLedgerState::Complete
        );
    }

    #[test]
    fn initial_state_from_snapshot_uses_idle_ledger_state_and_matching_balance_presentation() {
        let metadata = WalletMetadata::preview_new();
        let snapshot = WalletSnapshot { balance: Balance::zero(), transactions: Vec::new() };

        let state = initial_state_from_snapshot(
            metadata.clone(),
            WalletScanStatus::Idle,
            snapshot,
            Vec::new(),
        );

        let expected_ledger_state =
            WalletLedgerState::InitialScanIncomplete(ledger_state::InitialScanActivity::Idle);
        assert_eq!(state.metadata, metadata);
        assert_eq!(state.load_state, WalletLoadState::Loading);
        assert_eq!(state.scan_status, WalletScanStatus::Idle);
        assert_eq!(state.ledger_state, expected_ledger_state);
        assert_eq!(
            state.balance_presentation,
            BalancePresentation::for_ledger_state(expected_ledger_state)
        );
        assert_eq!(state.balance.as_ref(), &Balance::zero());
        assert!(state.unsigned_transactions.is_empty());
    }

    #[test]
    fn initial_state_from_snapshot_marks_completed_idle_wallet_loaded() {
        let metadata = preview_ledger_ready_metadata(WalletMetadata::preview_new());
        let snapshot = WalletSnapshot { balance: Balance::zero(), transactions: Vec::new() };

        let state =
            initial_state_from_snapshot(metadata, WalletScanStatus::Idle, snapshot, Vec::new());

        assert_eq!(state.ledger_state, WalletLedgerState::Complete);
        assert_eq!(state.load_state, WalletLoadState::Loaded(Vec::new()));
        assert_eq!(state.scan_status, WalletScanStatus::Idle);
    }

    #[test]
    fn initial_state_from_snapshot_keeps_incomplete_idle_empty_wallet_loading() {
        let metadata = WalletMetadata::preview_new();
        let snapshot = WalletSnapshot { balance: Balance::zero(), transactions: Vec::new() };

        let state =
            initial_state_from_snapshot(metadata, WalletScanStatus::Idle, snapshot, Vec::new());

        assert_eq!(
            state.ledger_state,
            WalletLedgerState::InitialScanIncomplete(ledger_state::InitialScanActivity::Idle)
        );
        assert_eq!(state.load_state, WalletLoadState::Loading);
        assert_eq!(state.scan_status, WalletScanStatus::Idle);
    }

    #[test]
    fn initial_state_from_snapshot_marks_active_scan_scanning() {
        let metadata = WalletMetadata::preview_new();
        let transactions = vec![transaction()];
        let snapshot =
            WalletSnapshot { balance: Balance::zero(), transactions: transactions.clone() };

        let state = initial_state_from_snapshot(
            metadata,
            WalletScanStatus::Scanning(progress()),
            snapshot,
            Vec::new(),
        );

        assert_eq!(
            state.ledger_state,
            WalletLedgerState::InitialScanIncomplete(ledger_state::InitialScanActivity::Active)
        );
        assert_eq!(state.load_state, WalletLoadState::Scanning(transactions));
        assert_eq!(state.scan_status, WalletScanStatus::Scanning(progress()));
    }

    #[test]
    fn initial_state_from_snapshot_marks_incomplete_idle_wallet_with_cached_transactions_scanning()
    {
        let metadata = WalletMetadata::preview_new();
        let transactions = vec![transaction()];
        let snapshot =
            WalletSnapshot { balance: Balance::zero(), transactions: transactions.clone() };

        let state =
            initial_state_from_snapshot(metadata, WalletScanStatus::Idle, snapshot, Vec::new());

        assert_eq!(
            state.ledger_state,
            WalletLedgerState::InitialScanIncomplete(ledger_state::InitialScanActivity::Idle)
        );
        assert_eq!(state.load_state, WalletLoadState::Scanning(transactions));
        assert_eq!(state.scan_status, WalletScanStatus::Idle);
    }

    #[test]
    fn initial_state_from_snapshot_uses_empty_unsigned_transactions_on_load_errors() {
        let metadata = WalletMetadata::preview_new();
        let snapshot = WalletSnapshot { balance: Balance::zero(), transactions: Vec::new() };
        let error = Error::PendingUnsignedTransactionsLoadError("read failed".to_string());

        let state = initial_state_from_snapshot_with_pending_unsigned_transactions(
            metadata,
            WalletScanStatus::Idle,
            snapshot,
            Err(error),
        );

        assert!(state.unsigned_transactions.is_empty());
        assert_eq!(state.load_state, WalletLoadState::Loading);
    }

    #[test]
    fn create_tx_output_below_dust_maps_to_wallet_output_below_dust() {
        let error = WalletManagerError::from(CreateTxError::OutputBelowDustLimit(0));

        assert!(matches!(error, WalletManagerError::OutputBelowDustLimit));
    }

    #[test]
    fn create_tx_coin_selection_maps_to_insufficient_funds() {
        let error = WalletManagerError::from(CreateTxError::CoinSelection(InsufficientFunds {
            needed: Amount::from_sat(10_000),
            available: Amount::from_sat(1_000),
        }));

        assert!(matches!(error, WalletManagerError::InsufficientFunds(_)));
    }
}

#[uniffi::export]
impl WalletLoadState {
    fn is_equal(&self, other: WalletLoadState) -> bool {
        self == &other
    }
}
