use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use log::error;
use parking_lot::RwLock;

use crate::{
    app::FfiApp,
    database::{error::DatabaseError, Database},
    fingerprint::Fingerprint,
    keychain::{Keychain, KeychainError},
    router::Route,
    wallet::{WalletColor, WalletId, WalletMetadata},
    word_validator::WordValidator,
};

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelReconcileMessage {
    WalletMetadataChanged(WalletMetadata),
}

#[uniffi::export(callback_interface)]
pub trait WalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: WalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustWalletViewModel {
    pub state: Arc<RwLock<WalletViewModelState>>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<WalletViewModelReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct WalletViewModelState {
    pub wallet_metadata: WalletMetadata,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {
    UpdateName(String),
    UpdateColor(WalletColor),
}

pub type Error = WalletViewModelError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletViewModelError {
    #[error("failed to get selected wallet: {0}")]
    GetSelectedWalletError(String),

    #[error("wallet does not exist")]
    WalletDoesNotExist,

    #[error("unable to retrieve the secret words for the wallet {0}")]
    SecretRetrievalError(#[from] KeychainError),

    #[error("unable to mark wallet as verified")]
    MarkWalletAsVerifiedError(#[from] DatabaseError),
}

#[uniffi::export]
impl RustWalletViewModel {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let network = Database::global().global_config.selected_network();

        let wallet_metadata = Database::global()
            .wallets
            .get_selected_wallet(id, network)
            .map_err(|error| Error::GetSelectedWalletError(error.to_string()))?
            .ok_or(Error::WalletDoesNotExist)?;

        let state = WalletViewModelState::new(wallet_metadata);

        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        })
    }

    #[uniffi::method]
    pub fn delete_wallet(&self) -> Result<(), Error> {
        let wallet_id = self.state.read().wallet_metadata.id.clone();
        log::debug!("deleting wallet {wallet_id}");

        // delete the wallet from the database
        let database = Database::global();
        database.wallets.delete(&wallet_id)?;

        // delete the secret key from the keychain
        Keychain::global().delete_wallet_key(&wallet_id);

        // delete the xpub from keychain
        Keychain::global().delete_wallet_key(&wallet_id);

        // reset the default route to list wallets
        FfiApp::global().reset_default_route_to(Route::ListWallets);

        Ok(())
    }

    #[uniffi::method]
    pub fn word_validator(&self) -> Result<WordValidator, Error> {
        let mnemonic = Keychain::global()
            .get_wallet_key(&self.state.read().wallet_metadata.id)?
            .ok_or(Error::WalletDoesNotExist)?;

        let validator = WordValidator::new(mnemonic);

        Ok(validator)
    }

    #[uniffi::method]
    pub fn mark_wallet_as_verified(&self) -> Result<(), Error> {
        let wallet_metadata = self.state.read().wallet_metadata.clone();

        let database = Database::global();
        database
            .wallets
            .mark_wallet_as_verified(wallet_metadata.id)
            .map_err(Error::MarkWalletAsVerifiedError)?;

        Ok(())
    }

    #[uniffi::method]
    pub fn wallet_metadata(&self) -> WalletMetadata {
        self.state.read().wallet_metadata.clone()
    }

    #[uniffi::method]
    pub fn fingerprint(&self) -> String {
        let wallet_id = self.state.read().wallet_metadata.id.clone();

        Fingerprint::try_new(wallet_id)
            .map(|f| f.to_uppercase())
            .unwrap_or_else(|_| "Unknown".to_string())
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
                let mut state = self.state.write();
                state.wallet_metadata.name = name;

                self.reconciler
                    .send(WalletViewModelReconcileMessage::WalletMetadataChanged(
                        state.wallet_metadata.clone(),
                    ))
                    .unwrap();
            }
            WalletViewModelAction::UpdateColor(color) => {
                let mut state = self.state.write();
                state.wallet_metadata.color = color;

                self.reconciler
                    .send(WalletViewModelReconcileMessage::WalletMetadataChanged(
                        state.wallet_metadata.clone(),
                    ))
                    .unwrap();
            }
        }

        // update wallet_metadata in the database
        if let Err(error) = Database::global()
            .wallets
            .update_wallet_metadata(self.state.read().wallet_metadata.clone())
        {
            error!("Unable to update wallet metadata: {error:?}")
        }
    }
}

impl WalletViewModelState {
    pub fn new(wallet_metadata: WalletMetadata) -> Self {
        Self { wallet_metadata }
    }
}
