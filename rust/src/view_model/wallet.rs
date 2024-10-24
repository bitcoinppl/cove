mod actor;

use std::sync::Arc;

use act_zero::{call, send, Addr};
use actor::WalletActor;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;
use tap::TapFallible as _;
use tracing::{debug, error};

use crate::{
    app::FfiApp,
    database::{error::DatabaseError, Database},
    fiat::{client::FIAT_CLIENT, FiatCurrency},
    format::NumberFormatter,
    keychain::{Keychain, KeychainError},
    router::Route,
    task::{self, spawn_actor},
    transaction::{Amount, SentAndReceived, Transaction, TransactionDetails, TxId, Unit},
    wallet::{
        balance::Balance,
        fingerprint::Fingerprint,
        metadata::{DiscoveryState, FiatOrBtc, WalletColor, WalletId, WalletMetadata},
        AddressInfo, Wallet, WalletAddressType, WalletError,
    },
    wallet_scanner::{ScannerResponse, WalletScanner},
    word_validator::WordValidator,
};

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelReconcileMessage {
    StartedWalletScan,
    AvailableTransactions(Vec<Transaction>),
    ScanComplete(Vec<Transaction>),

    NodeConnectionFailed(String),
    WalletMetadataChanged(WalletMetadata),
    WalletBalanceChanged(Balance),

    WalletError(WalletViewModelError),
    UnknownError(String),

    WalletScannerResponse(ScannerResponse),
}

#[uniffi::export(callback_interface)]
pub trait WalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: WalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustWalletViewModel {
    pub id: WalletId,
    pub actor: Addr<WalletActor>,

    // cache, metadata already exists in the database and in the actor state,  this cache makes it
    // faster to access, but adds complexity to the code because we have to make sure its updated
    // in all the places
    pub metadata: Arc<RwLock<WalletMetadata>>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<WalletViewModelReconcileMessage>>,
    #[allow(dead_code)]
    pub scanner: Option<Addr<WalletScanner>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {
    UpdateName(String),
    UpdateColor(WalletColor),
    UpdateUnit(Unit),
    UpdateFiatCurrency(FiatCurrency),
    ToggleSensitiveVisibility,
    ToggleDetailsExpanded,
    ToggleFiatOrBtc,
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
}

pub type Error = WalletViewModelError;
#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletViewModelError {
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
}

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletViewModel {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let network = Database::global().global_config.selected_network();

        let metadata = Database::global()
            .wallets
            .get(&id, network)
            .map_err(|e| Error::GetSelectedWalletError(e.to_string()))?
            .ok_or(Error::WalletDoesNotExist)?;

        let id = metadata.id.clone();
        let wallet = Wallet::try_load_persisted(id.clone())?;
        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));

        // only creates the scanner if its not already complet
        let scanner = WalletScanner::try_new(metadata.clone(), sender.clone())
            .ok()
            .map(spawn_actor);

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            scanner,
        })
    }

    #[uniffi::constructor(name = "try_new_from_xpub")]
    pub fn try_new_from_xpub(xpub: String) -> Result<Self, Error> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let wallet = Wallet::try_new_persisted_from_xpub(xpub)?;
        let id = wallet.id.clone();
        let metadata = wallet.metadata.clone();

        let scanner = WalletScanner::try_new(metadata.clone(), sender.clone())
            .ok()
            .map(spawn_actor);

        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            scanner,
        })
    }

    #[uniffi::method]
    pub async fn balance(&self) -> Balance {
        call!(self.actor.balance()).await.unwrap_or_default()
    }

    #[uniffi::method]
    pub async fn balance_in_fiat(&self) -> Result<f64, Error> {
        let currency = self.metadata.read().selected_fiat_currency;
        let balance = call!(self.actor.balance())
            .await
            .map_err(|_| Error::WalletBalanceError("unable to get balance".to_string()))?;

        self.amount_in_fiat(balance.confirmed, currency).await
    }

    #[uniffi::method]
    pub async fn amount_in_fiat(
        &self,
        amount: Arc<Amount>,
        currency: FiatCurrency,
    ) -> Result<f64, Error> {
        FIAT_CLIENT
            .value_in_currency(*amount, currency)
            .await
            .map_err(|error| {
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
        amount.fmt_string(unit)
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

    #[uniffi::method]
    pub fn display_fiat_amount(&self, amount: f64) -> String {
        {
            let sensitive_visible = self.metadata.read().sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let fiat = amount.thousands_fiat();
        format!("${fiat} {}", self.metadata.read().selected_fiat_currency)
    }

    #[uniffi::method]
    pub async fn sent_and_received_fiat(
        &self,
        sent_and_received: Arc<SentAndReceived>,
    ) -> Result<f64, Error> {
        let amount = sent_and_received.amount();
        let currency = self.metadata.read().selected_fiat_currency;

        let fiat = FIAT_CLIENT
            .value_in_currency(amount, currency)
            .await
            .map_err(|error| {
                Error::FiatError(format!("unable to get fiat value for amount: {error}"))
            })?;

        Ok(fiat)
    }

    #[uniffi::method]
    pub async fn current_block_height(&self) -> Result<u32, Error> {
        let height = call!(self.actor.get_height(false))
            .await
            .map_err(|_| Error::GetHeightError)?;

        Ok(height as u32)
    }

    #[uniffi::method]
    pub async fn force_update_height(&self) -> Result<u32, Error> {
        let height = call!(self.actor.get_height(true))
            .await
            .map_err(|_| Error::GetHeightError)?;

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
    pub async fn next_address(&self) -> Result<AddressInfo, Error> {
        let address = call!(self.actor.next_address())
            .await
            .map_err(|error| Error::NextAddressError(error.to_string()))?;

        Ok(address)
    }

    /// Get address at the given index
    #[uniffi::method]
    pub async fn address_at(&self, index: u32) -> Result<AddressInfo, Error> {
        let address = call!(self.actor.address_at(index))
            .await
            .map_err(|_| Error::ActorNotFound)?;

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

        // delete the secret key from the keychain
        keychain.delete_wallet_key(&wallet_id);

        // delete the xpub from keychain
        keychain.delete_wallet_xpub(&wallet_id);

        // delete the wallet persisted bdk data
        if let Err(error) = crate::wallet::delete_data_path(&wallet_id) {
            error!("Unable to delete wallet persisted bdk data: {error}");
        }

        // unselect the wallet in the database
        match database.global_config.selected_wallet() {
            Some(selected_wallet_id) if selected_wallet_id == wallet_id => {
                let _ = database
                    .global_config
                    .clear_selected_wallet()
                    .tap_err(|error| {
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
    pub async fn start_wallet_scan(&self) -> Result<(), Error> {
        debug!("start_wallet_scan: {}", self.id);

        let actor = self.actor.clone();
        tokio::spawn(async move {
            send!(actor.wallet_scan_and_notify(false));
        });

        Ok(())
    }

    #[uniffi::method]
    pub async fn force_wallet_scan(&self) -> Result<(), Error> {
        debug!("force_wallet_scan: {}", self.id);

        let actor = self.actor.clone();
        tokio::spawn(async move {
            send!(actor.wallet_scan_and_notify(true));
        });

        Ok(())
    }

    #[uniffi::method]
    pub fn mark_wallet_as_verified(&self) -> Result<(), Error> {
        let wallet_metadata = &self.metadata.read();

        let database = Database::global();
        database
            .wallets
            .mark_wallet_as_verified(wallet_metadata.id.clone())
            .map_err(Error::MarkWalletAsVerifiedError)?;

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
    pub fn fingerprint(&self) -> String {
        let wallet_id = &self.metadata.read().id;

        Fingerprint::try_new(wallet_id)
            .map(|f| f.to_uppercase())
            .unwrap_or_else(|_| "Unknown".to_string())
    }

    #[uniffi::method]
    pub fn word_validator(&self) -> Result<WordValidator, Error> {
        let mnemonic = Keychain::global()
            .get_wallet_key(&self.metadata.read().id)?
            .ok_or(Error::WalletDoesNotExist)?;

        let validator = WordValidator::new(mnemonic);

        Ok(validator)
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn WalletViewModelReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                // call the reconcile method on the frontend
                reconciler.reconcile(field);
            }
        });
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
                call!(actor.switch_descriptor_to_new_address_type(
                            descriptors,
                            wallet_address_type
                        ))
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
    pub fn dispatch(&self, action: WalletViewModelAction) {
        match action {
            WalletViewModelAction::UpdateName(name) => {
                let mut metadata = self.metadata.write();
                metadata.name = name;
            }

            WalletViewModelAction::UpdateColor(color) => {
                let mut metadata = self.metadata.write();
                metadata.color = color;
            }

            WalletViewModelAction::UpdateUnit(unit) => {
                let mut metadata = self.metadata.write();
                metadata.selected_unit = unit;
            }

            WalletViewModelAction::UpdateFiatCurrency(fiat_currency) => {
                let mut metadata = self.metadata.write();
                metadata.selected_fiat_currency = fiat_currency;
            }

            WalletViewModelAction::ToggleSensitiveVisibility => {
                let mut metadata = self.metadata.write();
                metadata.sensitive_visible = !metadata.sensitive_visible;
            }

            WalletViewModelAction::ToggleFiatOrBtc => {
                let mut metadata = self.metadata.write();
                metadata.fiat_or_btc = match metadata.fiat_or_btc {
                    FiatOrBtc::Btc => FiatOrBtc::Fiat,
                    FiatOrBtc::Fiat => FiatOrBtc::Btc,
                };
            }

            WalletViewModelAction::ToggleDetailsExpanded => {
                let mut metadata = self.metadata.write();
                metadata.details_expanded = !metadata.details_expanded;
            }

            WalletViewModelAction::SelectCurrentWalletAddressType => {
                let mut metadata = self.metadata.write();
                metadata.discovery_state = DiscoveryState::ChoseAdressType;
            }

            WalletViewModelAction::SelectDifferentWalletAddressType(wallet_address_type) => {
                {
                    let mut metadata = self.metadata.write();
                    metadata.address_type = wallet_address_type;
                    metadata.discovery_state = DiscoveryState::ChoseAdressType;

                    self.reconciler
                        .send(WalletViewModelReconcileMessage::WalletMetadataChanged(
                            metadata.clone(),
                        ))
                        .unwrap();

                    // update wallet_metadata in the database
                    let _ = Database::global()
                        .wallets
                        .update_wallet_metadata(metadata.clone());
                }
            }
        }

        let metadata = self.metadata.read();
        let metdata_changed_msg =
            WalletViewModelReconcileMessage::WalletMetadataChanged(metadata.clone());

        self.reconciler.send(metdata_changed_msg).unwrap();

        // update wallet_metadata in the database
        if let Err(error) = Database::global()
            .wallets
            .update_wallet_metadata(self.metadata.read().clone())
        {
            error!("Unable to update wallet metadata: {error:?}")
        }
    }
}

#[uniffi::export]
impl RustWalletViewModel {
    #[uniffi::constructor]
    pub fn preview_new_wallet() -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let wallet = Wallet::preview_new_wallet();
        let metadata = WalletMetadata::preview_new();
        let actor = task::spawn_actor(WalletActor::new(wallet, sender.clone()));

        Self {
            id: metadata.id.clone(),
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            scanner: None,
        }
    }
}

impl Drop for RustWalletViewModel {
    fn drop(&mut self) {
        debug!("[DROP] Wallet View Model: {}", self.id);
    }
}

mod ffi {
    use super::*;

    #[uniffi::export]
    fn wallet_state_is_equal(lhs: WalletLoadState, rhs: WalletLoadState) -> bool {
        lhs == rhs
    }
}
