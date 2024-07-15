use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletViewModelReconcileMessage {
    NoOp,
}

#[uniffi::export(callback_interface)]
pub trait ImportWalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: ImportWalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustImportWalletViewModel {
    pub state: Arc<RwLock<ImportWalletViewModelState>>,
    pub reconciler: Sender<ImportWalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<ImportWalletViewModelReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImportWalletViewModelState {}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletViewModelAction {
    NoOp,
}

#[uniffi::export]
impl RustImportWalletViewModel {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(ImportWalletViewModelState::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn ImportWalletViewModelReconciler>) {
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
    pub fn dispatch(&self, action: ImportWalletViewModelAction) {
        let state = self.state.clone();

        match action {
            ImportWalletViewModelAction::NoOp => {}
        }
    }
}

impl ImportWalletViewModelState {
    pub fn new() -> Self {
        Self {}
    }
}
