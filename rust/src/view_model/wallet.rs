use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{
    database::Database,
    keychain::{Keychain, KeychainError},
    wallet::{Network, WalletId, WalletMetadata},
    word_validator::WordValidator,
};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelReconcileMessage {
    NoOp,
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

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {
    NoOp,
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
}

#[uniffi::export]
impl RustWalletViewModel {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let wallet_metadata = Database::global()
            .wallets
            .get_selected_wallet(id, Network::Bitcoin)
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
    pub fn get_state(&self) -> WalletViewModelState {
        self.state.read().clone()
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
    pub fn word_validator(&self) -> Result<WordValidator, Error> {
        let mnemonic = Keychain::global()
            .get_wallet_key(&self.state.read().wallet_metadata.id)?
            .ok_or(Error::WalletDoesNotExist)?;

        let validator = WordValidator::new(mnemonic);

        Ok(validator)
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: WalletViewModelAction) {
        let _state = self.state.clone();

        match action {
            WalletViewModelAction::NoOp => {}
        }
    }
}

impl WalletViewModelState {
    pub fn new(wallet_metadata: WalletMetadata) -> Self {
        Self { wallet_metadata }
    }
}
