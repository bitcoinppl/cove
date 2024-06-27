use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use nid::Nanoid;
use parking_lot::RwLock;

use crate::{
    impl_default_for,
    keychain::{Keychain, KeychainError},
    new_type,
    word_validator::WordValidator,
};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelReconcileMessage {}

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
    id: WalletId,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {}

type Error = WalletViewModelError;
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletViewModelError {
    #[error("Unable to retrieve the secret words for the wallet {0}")]
    SecretRetrievalError(#[from] KeychainError),

    #[error("Wallet does not exist")]
    WalletDoesNotExist,
}

new_type!(WalletId, String);
impl_default_for!(WalletId);
impl WalletId {
    pub fn new() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }
}

#[uniffi::export]
impl RustWalletViewModel {
    #[uniffi::constructor]
    pub fn new(id: WalletId) -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(WalletViewModelState::new(id))),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn get_state(&self) -> WalletViewModelState {
        self.state.read().clone()
    }

    #[uniffi::method]
    pub fn word_validator(&self) -> Result<WordValidator, Error> {
        let mnemonic = Keychain::global()
            .get_wallet_key(self.state.read().id.clone())?
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
        let state = self.state.clone();

        match action {}
    }
}

impl WalletViewModelState {
    pub fn new(id: WalletId) -> Self {
        Self { id }
    }
}
