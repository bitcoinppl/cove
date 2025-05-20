pub mod actor;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use act_zero::{Addr, call, send};
use actor::WalletActor;
use flume::{Receiver, Sender, TrySendError};
use parking_lot::RwLock;
use tap::TapFallible as _;
use tracing::{debug, error, warn};

use cove_util::format::NumberFormatter as _;

use crate::{
    app::FfiApp,
    converter::{Converter, ConverterError},
    database::{Database, error::DatabaseError},
    fee_client::{FEE_CLIENT, FEES, FeeResponse},
    fiat::{
        FiatCurrency,
        client::{FIAT_CLIENT, PriceResponse},
    },
    keychain::{Keychain, KeychainError},
    label_manager::LabelManager,
    psbt::Psbt,
    reporting::HistoricalFiatPriceReport,
    router::Route,
    tap_card::tap_signer_reader::DeriveInfo,
    task::{self, spawn_actor},
    transaction::{
        Amount, FeeRate, SentAndReceived, Transaction, TransactionDetails, TxId, Unit,
        ffi::BitcoinTransaction, unsigned_transaction::UnsignedTransaction,
    },
    wallet::{
        Address, AddressInfo, Wallet, WalletAddressType, WalletError,
        balance::Balance,
        fingerprint::Fingerprint,
        metadata::{DiscoveryState, FiatOrBtc, WalletColor, WalletId, WalletMetadata},
    },
    wallet_scanner::{ScannerResponse, WalletScanner},
    word_validator::WordValidator,
};

use cove_types::fees::FeeRateOptions;
use cove_types::{
    address::AddressInfoWithDerivation,
    confirm::{AddressAndAmount, ConfirmDetails, SplitOutput},
};

use super::{
    coin_control_manager::RustCoinControlManager, deferred_sender,
    send_flow_manager::RustSendFlowManager,
};

type Action = WalletManagerAction;
type Message = WalletManagerReconcileMessage;
type Reconciler = dyn WalletManagerReconciler;
pub type SingleOrMany = deferred_sender::SingleOrMany<Message>;
cove_macros::impl_manager_message_send!(RustWalletManager);

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletManagerReconcileMessage {
    StartedInitialFullScan,
    StartedExpandedFullScan(Vec<Transaction>),

    AvailableTransactions(Vec<Transaction>),
    ScanComplete(Vec<Transaction>),
    UpdatedTransactions(Vec<Transaction>),

    NodeConnectionFailed(String),
    WalletMetadataChanged(WalletMetadata),
    WalletBalanceChanged(Arc<Balance>),

    WalletError(WalletManagerError),
    UnknownError(String),

    WalletScannerResponse(ScannerResponse),
    UnsignedTransactionsChanged,

    SendFlowError(SendFlowErrorAlert),
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
    SelectDifferentWalletAddressType(WalletAddressType),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletLoadState {
    Loading,
    Scanning(Vec<Transaction>),
    Loaded(Vec<Transaction>),
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
    pub reconciler: Sender<SingleOrMany>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,

    label_manager: Arc<LabelManager>,

    #[allow(dead_code)]
    scanner: Option<Addr<WalletScanner>>,
}

pub type Error = WalletManagerError;
#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
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

    #[error("unable to build transaction: {0}")]
    BuildTxError(String),

    #[error("insufficient funds: {0}")]
    InsufficientFunds(String),

    #[error("Unable to get confirm details, {0}")]
    GetConfirmDetailsError(String),

    #[error("Unable to sign and broadcast transaction, {0}")]
    SignAndBroadcastError(String),

    #[error(transparent)]
    ConverterError(#[from] ConverterError),

    #[error("Unknown error: {0}")]
    UnknownError(String),

    #[error("Error finalizing PSBT: {0}")]
    PsbtFinalizeError(String),

    #[error("Unable to get historical prices for transactions: {0}")]
    GetHistoricalPricesError(String),

    #[error("Unable to create report CSV: {0}")]
    CsvCreationError(String),
}

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(10);

        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let metadata = Database::global()
            .wallets
            .get(&id, network, mode)
            .map_err(|e| Error::GetSelectedWalletError(e.to_string()))?
            .ok_or(Error::WalletDoesNotExist)?;

        let id = metadata.id.clone();
        let wallet = Wallet::try_load_persisted(id.clone())?;
        let metadata = wallet.metadata.clone();
        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));

        // only creates the scanner if its not already complet
        let scanner =
            WalletScanner::try_new(metadata.clone(), sender.clone()).ok().map(spawn_actor);

        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            label_manager,
            scanner,
        })
    }

    #[uniffi::method]
    pub fn label_manager(&self) -> Arc<LabelManager> {
        self.label_manager.clone()
    }

    #[uniffi::method]
    pub fn new_send_flow_manager(self: Arc<Self>) -> Arc<RustSendFlowManager> {
        let me = self.clone();
        let metadata = self.metadata.read().clone();

        RustSendFlowManager::new(metadata, me)
    }

    #[uniffi::method]
    pub async fn new_coin_control_manager(&self) -> Arc<RustCoinControlManager> {
        let metadata = self.metadata.read().clone();
        let unspent = call!(self.actor.list_unspent()).await.expect("actor failed");

        let manager = RustCoinControlManager::new(metadata, unspent);
        Arc::new(manager)
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

    #[uniffi::constructor]
    pub fn try_new_from_xpub(xpub: String) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::try_new_persisted_from_xpub(xpub)?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();

        let scanner =
            WalletScanner::try_new(metadata.clone(), sender.clone()).ok().map(spawn_actor);

        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));
        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            label_manager,
            scanner,
        })
    }

    #[uniffi::constructor(default(backup = None))]
    pub fn try_new_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive_info: DeriveInfo,
        backup: Option<Vec<u8>>,
    ) -> Result<Self, Error> {
        let (sender, receiver) = flume::bounded(100);

        let wallet =
            Wallet::try_new_persisted_from_tap_signer(tap_signer.clone(), derive_info, backup)?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();

        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));
        let label_manager = LabelManager::new(id.clone()).into();

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            label_manager,
            scanner: None,
        })
    }

    #[uniffi::method]
    pub fn selected_fiat_currency(&self) -> FiatCurrency {
        Database::global().global_config.fiat_currency().unwrap_or_default()
    }

    #[uniffi::method]
    pub async fn get_fee_options(&self) -> Result<FeeRateOptions, Error> {
        let fee_client = &FEE_CLIENT;
        let fees = fee_client
            .fetch_and_get_fees()
            .await
            .map_err(|error| Error::FeesError(error.to_string()))?;

        Ok(fees.into())
    }

    #[uniffi::method]
    pub async fn create_transactions_with_fiat_export(&self) -> Result<String, Error> {
        let fiat_currency = Database::global().global_config.fiat_currency().unwrap_or_default();

        let txns_with_prices = call!(self.actor.txns_with_prices()).await.unwrap().unwrap();

        let report = HistoricalFiatPriceReport::new(fiat_currency, txns_with_prices);
        let csv = report.create_csv().map_err(|e| Error::CsvCreationError(e.to_string()))?;

        Ok(csv.into_string())
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
        let wallet_id = self.id.clone();
        let tx_id = details.psbt.tx_id();
        let db = Database::global();

        let confirm_details = Arc::unwrap_or_clone(details);

        let db = db.unsigned_transactions();

        if db.get_tx(&tx_id)?.is_some() {
            warn!("tx {} already exists", tx_id.0.to_raw_hash().to_string());
            return Ok(());
        }

        // save the tx to the database
        db.save_tx(
            tx_id,
            UnsignedTransaction {
                wallet_id,
                tx_id,
                confirm_details,
                created_at: jiff::Timestamp::now().as_second() as u64,
            }
            .into(),
        )?;

        self.send(Message::UnsignedTransactionsChanged);

        Ok(())
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
        let wallet_id = &self.id;

        let db = Database::global();
        let txns = db.unsigned_transactions().get_by_wallet_id(wallet_id)?;

        let txns = txns
            .into_iter()
            .map(|txn| Arc::new(txn.into()))
            .collect::<Vec<Arc<UnsignedTransaction>>>();

        Ok(txns)
    }

    /// gets the transactions for the wallet that are currently available
    #[uniffi::method]
    pub async fn get_transactions(&self) {
        let Ok(txns) = call!(self.actor.transactions()).await else { return };

        self.send(Message::UpdatedTransactions(txns));
    }

    #[uniffi::method]
    pub fn delete_unsigned_transaction(&self, tx_id: Arc<TxId>) -> Result<(), Error> {
        debug!("deleting unsigned transaction: {tx_id:?}");
        let db = Database::global();

        let txn = db.unsigned_transactions().delete_tx(tx_id.as_ref())?;
        send!(self.actor.cancel_txn(txn.confirm_details.psbt.0.unsigned_tx));

        self.send(Message::UnsignedTransactionsChanged);

        Ok(())
    }

    #[uniffi::method]
    pub async fn balance(&self) -> Balance {
        call!(self.actor.balance()).await.unwrap_or_default()
    }

    #[uniffi::method]
    pub async fn sign_and_broadcast_transaction(&self, psbt: Arc<Psbt>) -> Result<(), Error> {
        let psbt = Arc::unwrap_or_clone(psbt);
        call!(self.actor.sign_and_broadcast_transaction(psbt.into())).await.unwrap()?;

        self.force_wallet_scan().await;

        Ok(())
    }

    #[uniffi::method]
    pub async fn broadcast_transaction(
        &self,
        signed_transaction: Arc<BitcoinTransaction>,
    ) -> Result<(), Error> {
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
    pub async fn balance_in_fiat(&self) -> Result<f64, Error> {
        let balance = call!(self.actor.balance())
            .await
            .map_err(|_| Error::WalletBalanceError("unable to get balance".to_string()))?;

        self.amount_in_fiat(balance.total().into()).await
    }

    #[uniffi::method]
    pub async fn amount_in_fiat(&self, amount: Arc<Amount>) -> Result<f64, Error> {
        let currency = self.selected_fiat_currency();

        FIAT_CLIENT.current_value_in_currency(*amount, currency).await.map_err(|error| {
            Error::FiatError(format!("unable to get fiat value for amount: {error}"))
        })
    }

    #[uniffi::method]
    pub fn display_amount(&self, amount: Arc<Amount>) -> String {
        {
            let sensitive_visible = self.metadata.read().sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let unit = self.metadata.read().selected_unit;
        amount.fmt_string_with_unit(unit)
    }

    #[uniffi::method]
    pub fn display_sent_and_received_amount(
        &self,
        sent_and_received: Arc<SentAndReceived>,
    ) -> String {
        {
            let sensitive_visible = self.metadata.read().sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let unit = self.metadata.read().selected_unit;
        sent_and_received.amount_fmt(unit)
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount(&self, amount: f64, with_suffix: bool) -> String {
        {
            let sensitive_visible = self.metadata.read().sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let fiat = amount.thousands_fiat();

        let currency = self.selected_fiat_currency();
        let symbol = currency.symbol();
        let suffix = currency.suffix();

        if with_suffix && !suffix.is_empty() {
            return format!("{symbol}{fiat} {suffix}");
        }

        format!("{symbol}{fiat}")
    }

    #[uniffi::method]
    pub fn convert_to_fiat(&self, amount: Arc<Amount>, prices: Arc<PriceResponse>) -> f64 {
        let currency = self.selected_fiat_currency();
        let price = prices.get_for_currency(currency) as f64;
        ((amount.as_btc() * price) * 100.0).ceil() / 100.0
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn convert_and_display_fiat(
        &self,
        amount: Arc<Amount>,
        prices: Arc<PriceResponse>,
        with_suffix: bool,
    ) -> String {
        let fiat = self.convert_to_fiat(amount, prices);
        self.display_fiat_amount(fiat, with_suffix)
    }

    #[uniffi::method]
    pub async fn sent_and_received_fiat(
        &self,
        sent_and_received: Arc<SentAndReceived>,
    ) -> Result<f64, Error> {
        let amount = sent_and_received.amount();
        let currency = self.selected_fiat_currency();

        let fiat =
            FIAT_CLIENT.current_value_in_currency(amount, currency).await.map_err(|error| {
                Error::FiatError(format!("unable to get fiat value for amount: {error}"))
            })?;

        Ok(fiat)
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
                .map_err(|error| Error::TransactionDetailsError(error.to_string()))
        })
        .await
        .unwrap()?;

        Ok(details)
    }

    #[uniffi::method]
    pub async fn number_of_confirmations(&self, block_height: u32) -> Result<u32, Error> {
        let current_height = self.current_block_height().await?;
        Ok(current_height - block_height + 1)
    }

    #[uniffi::method]
    pub async fn number_of_confirmations_fmt(&self, block_height: u32) -> Result<String, Error> {
        let number_of_confirmations = self.number_of_confirmations(block_height).await?;
        Ok(number_of_confirmations.thousands_int())
    }

    /// Get the next address for the wallet
    #[uniffi::method]
    pub async fn next_address(&self) -> Result<AddressInfoWithDerivation, Error> {
        let address = call!(self.actor.next_address())
            .await
            .map_err(|error| Error::NextAddressError(error.to_string()))?;

        Ok(address)
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
        let wallet_id = self.metadata.read().id.clone();
        tracing::debug!("deleting wallet {wallet_id}");

        let database = Database::global();
        let keychain = Keychain::global();

        // delete the wallet from the database
        database.wallets.delete(&wallet_id)?;

        // delete the secret key, xpub and public descriptor from the keychain
        keychain.delete_wallet_items(&wallet_id);

        // delete the wallet persisted bdk data
        if let Err(error) = crate::wallet::delete_wallet_specific_data(&wallet_id) {
            error!("Unable to delete wallet persisted bdk data and wallet data database: {error}");
        }

        // unselect the wallet in the database
        match database.global_config.selected_wallet() {
            Some(selected_wallet_id) if selected_wallet_id == wallet_id => {
                let _ = database.global_config.clear_selected_wallet().tap_err(|error| {
                    error!("Unable to clear selected wallet: {error}");
                });
            }
            _ => (),
        }

        // reset the default route to list wallets
        FfiApp::global().load_and_reset_default_route(Route::ListWallets);

        Ok(())
    }

    #[uniffi::method]
    pub fn validate_metadata(&self) {
        if self.metadata.read().name.trim().is_empty() {
            let name = self
                .metadata
                .read()
                .master_fingerprint
                .as_deref()
                .map(Fingerprint::as_uppercase)
                .unwrap_or_else(|| "Unnamed Wallet".to_string());

            self.dispatch(Action::UpdateName(name));
        }
    }

    #[uniffi::method]
    pub async fn start_wallet_scan(&self) -> Result<(), Error> {
        debug!("start_wallet_scan: {}", self.id);

        let actor = self.actor.clone();
        tokio::spawn(async move {
            send!(actor.wallet_scan_and_notify(false));
        });

        Ok(())
    }

    #[uniffi::method]
    pub async fn force_wallet_scan(&self) {
        debug!("force_wallet_scan: {}", self.id);

        let actor = self.actor.clone();
        tokio::spawn(async move {
            send!(actor.wallet_scan_and_notify(true));
        });
    }

    #[uniffi::method]
    pub fn mark_wallet_as_verified(&self) -> Result<(), Error> {
        {
            let mut wallet_metadata = self.metadata.write();
            wallet_metadata.verified = true;

            self.send(Message::WalletMetadataChanged(wallet_metadata.clone()));
        }

        let id = self.metadata.read().id.clone();
        let database = Database::global();

        database.wallets.mark_wallet_as_verified(&id).map_err(Error::MarkWalletAsVerifiedError)?;

        Ok(())
    }

    #[uniffi::method]
    pub fn wallet_metadata(&self) -> WalletMetadata {
        self.metadata.read().clone()
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
                crate::task::spawn(async move { crate::fee_client::get_and_update_fees().await });
            }
            None => {
                crate::task::spawn(async move { crate::fee_client::get_and_update_fees().await });
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
        let fees = fee_client
            .fetch_and_get_fees()
            .await
            .map_err(|error| Error::FeesError(error.to_string()))?;

        Ok(fees.into())
    }

    pub async fn build_drain_transaction(
        &self,
        address: Arc<Address>,
        fee: Arc<FeeRate>,
    ) -> Result<Psbt, Error> {
        let address = Arc::unwrap_or_clone(address);
        let fee = Arc::unwrap_or_clone(fee);

        let psbt: Psbt =
            call!(self.actor.build_ephemeral_drain_tx(address, fee)).await.unwrap()?.into();

        Ok(psbt)
    }

    pub async fn build_transaction(
        &self,
        amount: Arc<Amount>,
        address: Arc<Address>,
    ) -> Result<Psbt, Error> {
        let medium_fee = self
            .fees()
            .map(|fees| FeeRateOptions::from(fees).medium.fee_rate)
            .unwrap_or_else(|| FeeRate::from_sat_per_vb(10.0));

        self.build_transaction_with_fee_rate(amount, address, Arc::new(medium_fee)).await
    }

    pub async fn build_transaction_with_fee_rate(
        &self,
        amount: Arc<Amount>,
        address: Arc<Address>,
        fee_rate: Arc<FeeRate>,
    ) -> Result<Psbt, Error> {
        let actor = self.actor.clone();

        let amount = Arc::unwrap_or_clone(amount).into();
        let address = Arc::unwrap_or_clone(address);
        let fee_rate = Arc::unwrap_or_clone(fee_rate).into();

        let psbt = call!(actor.build_ephemeral_tx(amount, address, fee_rate)).await.unwrap()?;

        Ok(psbt.into())
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
                        error!("trying to swtich to native segwit, but already segwit");
                        return Ok(());
                    }
                };

                let descriptors = descriptors.ok_or(Error::UnableToSwitch(
                    wallet_address_type,
                    "No descriptors found for address type".to_string(),
                ))?;

                let id = self.id.clone();
                let actor = self.actor.clone();
                call!(
                    actor.switch_descriptor_to_new_address_type(descriptors, wallet_address_type)
                )
                .await
                .map_err(|e| Error::UnableToSwitch(wallet_address_type, e.to_string()))?;

                // reset route so it reloads the wallet with new txns
                FfiApp::global().load_and_reset_default_route(Route::SelectedWallet(id));
            }

            DiscoveryState::FoundAddressesFromMnemonic(_) => {
                let id = self.id.clone();
                let actor = self.actor.clone();
                call!(actor.switch_mnemonic_to_new_address_type(wallet_address_type))
                    .await
                    .map_err(|e| Error::UnableToSwitch(wallet_address_type, e.to_string()))?;

                debug!("switch done");

                // reset route so it reloads the wallet with new txns
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
        match action {
            Action::UpdateName(name) => {
                let mut metadata = self.metadata.write();
                metadata.name = name;
            }

            Action::UpdateColor(color) => {
                let mut metadata = self.metadata.write();
                metadata.color = color;
            }

            Action::UpdateUnit(unit) => {
                let mut metadata = self.metadata.write();
                metadata.selected_unit = unit;
            }

            Action::ToggleSensitiveVisibility => {
                let mut metadata = self.metadata.write();
                metadata.sensitive_visible = !metadata.sensitive_visible;
            }

            Action::ToggleFiatOrBtc => {
                let mut metadata = self.metadata.write();
                metadata.fiat_or_btc = match metadata.fiat_or_btc {
                    FiatOrBtc::Btc => FiatOrBtc::Fiat,
                    FiatOrBtc::Fiat => FiatOrBtc::Btc,
                };
            }

            Action::UpdateFiatOrBtc(fiat_or_btc) => {
                let mut metadata = self.metadata.write();
                metadata.fiat_or_btc = fiat_or_btc;
            }

            Action::ToggleFiatBtcPrimarySecondary => {
                let order = [
                    (FiatOrBtc::Btc, Unit::Btc),
                    (FiatOrBtc::Fiat, Unit::Btc),
                    (FiatOrBtc::Btc, Unit::Sat),
                    (FiatOrBtc::Fiat, Unit::Sat),
                ];

                let current =
                    (self.metadata.read().fiat_or_btc, self.metadata.read().selected_unit);

                let current_index = order
                    .iter()
                    .position(|option| option == &current)
                    .expect("all options covered");

                let next_index = (current_index + 1) % order.len();
                let (fiat_or_btc, unit) = order[next_index];

                self.dispatch(Action::UpdateFiatOrBtc(fiat_or_btc));
                self.dispatch(Action::UpdateUnit(unit));
            }

            Action::ToggleDetailsExpanded => {
                let mut metadata = self.metadata.write();
                metadata.details_expanded = !metadata.details_expanded;
            }

            Action::SelectCurrentWalletAddressType => {
                let mut metadata = self.metadata.write();
                metadata.discovery_state = DiscoveryState::ChoseAdressType;
            }

            Action::SelectDifferentWalletAddressType(wallet_address_type) => {
                let mut metadata = self.metadata.write();
                metadata.address_type = wallet_address_type;
                metadata.discovery_state = DiscoveryState::ChoseAdressType;
            }

            Action::ToggleShowLabels => {
                let mut metadata = self.metadata.write();
                metadata.show_labels = !metadata.show_labels;
            }
        }

        let metadata = self.metadata.read();
        let metadata_changed_msg = Message::WalletMetadataChanged(metadata.clone());
        self.send(metadata_changed_msg);

        // update wallet_metadata in the database
        if let Err(error) =
            Database::global().wallets.update_wallet_metadata(self.metadata.read().clone())
        {
            error!("Unable to update wallet metadata: {error:?}")
        }
    }
}

impl RustWalletManager {
    fn send(&self, message: impl Into<SingleOrMany>) {
        let message = message.into();
        debug!("send: {message:?}");
        match self.reconciler.try_send(message) {
            Ok(_) => {}
            Err(TrySendError::Full(message)) => {
                warn!("[WARN] unable to send, queue is full, sending async");

                let me = self.clone();
                task::spawn(async move { me.send_async(message).await });
            }
            Err(e) => {
                error!("unable to send message to send flow manager: {e:?}");
            }
        }
    }

    async fn send_async(&self, message: impl Into<SingleOrMany>) {
        let message = message.into();
        debug!("send_async: {message:?}");
        if let Err(err) = self.reconciler.send_async(message).await {
            error!("unable to send message to send flow manager: {err}");
        }
    }

    pub async fn confirm_txn(
        &self,
        amount: Amount,
        address: Arc<Address>,
        fee_rate: FeeRate,
    ) -> Result<ConfirmDetails, Error> {
        let actor = self.actor.clone();

        let amount = amount.into();
        let address = Arc::unwrap_or_clone(address);
        let fee_rate = fee_rate.into();

        let psbt = call!(actor.build_tx(amount, address, fee_rate)).await.unwrap()?;
        let details = call!(self.actor.get_confirm_details(psbt, fee_rate)).await.unwrap()?;

        Ok(details)
    }
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
        let (sender, receiver) = flume::bounded(100);

        let wallet = Wallet::preview_new_wallet();
        let label_manager = LabelManager::new(wallet.metadata.id.clone()).into();
        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));

        Self {
            id: metadata.id.clone(),
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            scanner: None,
            label_manager,
        }
    }
}

impl Drop for RustWalletManager {
    fn drop(&mut self) {
        debug!("[DROP] Wallet View manager: {}", self.id);
    }
}

#[uniffi::export]
fn wallet_state_is_equal(lhs: WalletLoadState, rhs: WalletLoadState) -> bool {
    lhs == rhs
}

#[uniffi::export]
fn describe_wallet_manager_error(error: WalletManagerError) -> String {
    error.to_string()
}
