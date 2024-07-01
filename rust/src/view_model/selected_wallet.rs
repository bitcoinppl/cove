use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{
    database::Database,
    wallet::{Network, WalletId, WalletMetadata},
};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SelectedWalletViewModelReconcileMessage {
    NoOp,
}

#[uniffi::export(callback_interface)]
pub trait SelectedWalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: SelectedWalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustSelectedWalletViewModel {
    pub state: Arc<RwLock<SelectedWalletViewModelState>>,
    pub reconciler: Sender<SelectedWalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<SelectedWalletViewModelReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectedWalletViewModelState {
    pub wallet_metadata: WalletMetadata,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SelectedWalletViewModelAction {
    NoOp,
}

pub type Error = SelectedWalletViewModelError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum SelectedWalletViewModelError {
    #[error("failed to get selected wallet: {0}")]
    GetSelectedWalletError(String),

    #[error("wallet does not exist")]
    WalletDoesNotExist,
}

#[uniffi::export]
impl RustSelectedWalletViewModel {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(id: WalletId) -> Result<Self, Error> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        let wallet_metadata = Database::global()
            .wallets
            .get_selected_wallet(id, Network::Bitcoin)
            .map_err(|error| Error::GetSelectedWalletError(error.to_string()))?
            .ok_or(Error::WalletDoesNotExist)?;

        let state = SelectedWalletViewModelState::new(wallet_metadata);

        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        })
    }

    #[uniffi::method]
    pub fn get_state(&self) -> SelectedWalletViewModelState {
        self.state.read().clone()
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn SelectedWalletViewModelReconciler>) {
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
    pub fn dispatch(&self, action: SelectedWalletViewModelAction) {
        let state = self.state.clone();

        match action {
            SelectedWalletViewModelAction::NoOp => {}
        }
    }
}

impl SelectedWalletViewModelState {
    pub fn new(wallet_metadata: WalletMetadata) -> Self {
        Self { wallet_metadata }
    }
}
