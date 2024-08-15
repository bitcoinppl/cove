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
    keychain::{Keychain, KeychainError},
    router::Route,
    task,
    transaction::{Transaction, Unit},
    wallet::{
        balance::Balance,
        fingerprint::Fingerprint,
        metadata::{WalletColor, WalletId, WalletMetadata},
        AddressInfo, Wallet, WalletError,
    },
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
    pub metadata: Arc<RwLock<WalletMetadata>>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<WalletViewModelReconcileMessage>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {
    UpdateName(String),
    UpdateColor(WalletColor),
    UpdateUnit(Unit),
    UpdateFiatCurrency(String),
    ToggleSensitiveVisibility,
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum WalletLoadState {
    Loading,
    Scanning(Vec<Transaction>),
    Loaded(Vec<Transaction>),
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

        Ok(Self {
            id,
            actor,
            metadata: Arc::new(RwLock::new(metadata)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        })
    }

    #[uniffi::method]
    pub async fn balance(&self) -> Balance {
        call!(self.actor.balance()).await.unwrap_or_default()
    }

    /// Get the next address for the wallet
    #[uniffi::method]
    pub async fn next_address(&self) -> Result<AddressInfo, Error> {
        let address = call!(self.actor.next_address())
            .await
            .map_err(|error| Error::NextAddressError(error.to_string()))?;

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
        FfiApp::global().reset_default_route_to(Route::ListWallets);

        Ok(())
    }

    #[uniffi::method]
    pub async fn start_wallet_scan(&self) -> Result<(), Error> {
        debug!("start_wallet_scan: {}", self.id);

        let actor = self.actor.clone();

        tokio::spawn(async move {
            send!(actor.wallet_scan_and_notify());
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
        }

        let metadata = self.metadata.read();

        self.reconciler
            .send(WalletViewModelReconcileMessage::WalletMetadataChanged(
                metadata.clone(),
            ))
            .unwrap();

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
