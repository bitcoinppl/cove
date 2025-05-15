use std::sync::Arc;

use cove_types::utxo::UtxoType;
use parking_lot::Mutex;

use crate::manager::deferred_sender;
use crate::task;
use cove_macros::{impl_default_for, impl_manager_message_send};
use flume::{Receiver, Sender, TrySendError};
use tracing::{debug, error, trace, warn};

#[allow(dead_code)]
type DeferredSender = deferred_sender::DeferredSender<Arc<RustCoinControlManager>, Message>;
type Message = CoinControlManagerReconcileMessage;
type Action = CoinControlManagerAction;
type State = CoinControlManagerState;
type Reconciler = dyn CoinControlManagerReconciler;
type SingleOrMany = deferred_sender::SingleOrMany<Message>;
impl_manager_message_send!(RustCoinControlManager);

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerReconcileMessage {
    NoOp,
}

#[uniffi::export(callback_interface)]
pub trait CoinControlManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, message: Vec<Message>);
}

#[derive(Clone, Debug, uniffi::Object)]
#[allow(dead_code)]
pub struct RustCoinControlManager {
    pub state: Arc<Mutex<CoinControlManagerState>>,
    pub reconciler: Sender<SingleOrMany>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct CoinControlManagerState {}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerAction {
    NoOp,
}

impl_default_for!(RustCoinControlManager);
#[uniffi::export]
impl RustCoinControlManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let (sender, receiver) = flume::bounded(10);

        Self {
            state: Arc::new(Mutex::new(CoinControlManagerState::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        task::spawn(async move {
            while let Ok(field) = reconcile_receiver.recv_async().await {
                trace!("reconcile_receiver: {field:?}");
                match field {
                    SingleOrMany::Single(message) => reconciler.reconcile(message),
                    SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
                }
            }
        });
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) {
        match action {
            CoinControlManagerAction::NoOp => {}
        }
    }
}

impl RustCoinControlManager {
    fn send(self: &Arc<Self>, message: impl Into<SingleOrMany>) {
        let message = message.into();
        debug!("send: {message:?}");
        match self.reconciler.try_send(message.clone()) {
            Ok(_) => {}
            Err(TrySendError::Full(err)) => {
                warn!("[WARN] unable to send, queue is full: {err:?}, sending async");

                let me = self.clone();
                task::spawn(async move { me.send_async(message).await });
            }
            Err(e) => {
                error!("unable to send message to send flow manager: {e:?}");
            }
        }
    }

    async fn send_async(self: &Arc<Self>, message: impl Into<SingleOrMany>) {
        let message = message.into();
        debug!("send_async: {message:?}");
        if let Err(err) = self.reconciler.send_async(message).await {
            error!("unable to send message to send flow manager: {err}");
        }
    }
}

// MARK: Sorting
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum)]
pub enum CoinControlListSort {
    Date(ListSortDirection),
    Name(ListSortDirection),
    Amount(ListSortDirection),
    Change(UtxoType),
}

impl Default for CoinControlListSort {
    fn default() -> Self {
        Self::Date(ListSortDirection::Descending)
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum)]
pub enum ListSortDirection {
    Ascending,
    Descending,
}

impl_default_for!(State);
impl State {
    pub fn new() -> Self {
        Self {}
    }
}
