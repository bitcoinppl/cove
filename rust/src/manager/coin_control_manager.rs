use std::{hash::Hasher, sync::Arc};

use bdk_wallet::LocalOutput;
use cove_types::{
    Network, WalletId,
    utxo::{Utxo, UtxoType},
};
use parking_lot::Mutex;

use crate::manager::deferred_sender;
use crate::task;
use cove_macros::impl_manager_message_send;
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
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Clone, Debug, uniffi::Object)]
#[allow(dead_code)]
pub struct RustCoinControlManager {
    pub state: Arc<Mutex<CoinControlManagerState>>,
    pub reconciler: Sender<SingleOrMany>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, uniffi::Object)]
pub struct CoinControlManagerState {
    wallet_id: WalletId,
    utxos: Vec<Utxo>,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerAction {
    NoOp,
}

#[uniffi::export]
impl RustCoinControlManager {
    #[uniffi::method]
    pub fn utxos(&self) -> Vec<Utxo> {
        self.state.lock().utxos.clone()
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
    pub fn new(id: WalletId, local_outputs: Vec<LocalOutput>, network: Network) -> Self {
        let (sender, receiver) = flume::bounded(10);

        let state = State::new(id, local_outputs, network);
        Self {
            state: Arc::new(Mutex::new(state)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

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

#[derive(
    Debug, Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum, derive_more::Display,
)]
pub enum CoinControlListSortKey {
    Date,
    Name,
    Amount,
    Change,
}

#[uniffi::export]
fn coin_control_list_sort_key_to_string(key: CoinControlListSortKey) -> String {
    key.to_string()
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

impl State {
    pub fn new(wallet_id: WalletId, unspent: Vec<LocalOutput>, network: Network) -> Self {
        let utxos =
            unspent.into_iter().filter_map(|o| Utxo::try_from_local(o, network).ok()).collect();

        Self { wallet_id, utxos }
    }
}

// MARK: impl
impl std::hash::Hash for RustCoinControlManager {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        let RustCoinControlManager { state, reconciler: _, reconcile_receiver: _ } = self;
        state.lock().hash(hasher);
    }
}

impl Eq for RustCoinControlManager {}
impl PartialEq for RustCoinControlManager {
    fn eq(&self, other: &Self) -> bool {
        let RustCoinControlManager { state, reconciler: _, reconcile_receiver: _ } = self;
        let RustCoinControlManager { state: other_state, reconciler: _, reconcile_receiver: _ } =
            other;

        state.lock().eq(&other_state.lock())
    }
}

mod ffi {
    use cove_types::utxo::ffi_preview::preview_new_utxo_list;

    use super::*;

    #[uniffi::export]
    impl RustCoinControlManager {
        #[uniffi::constructor(default(output_count = 20, change_count = 4))]
        pub fn preview_new(output_count: u8, change_count: u8) -> Self {
            let (sender, receiver) = flume::bounded(10);

            let state = State::preview_new(output_count, change_count);
            Self {
                state: Arc::new(Mutex::new(state)),
                reconciler: sender,
                reconcile_receiver: Arc::new(receiver),
            }
        }
    }

    #[uniffi::export]
    impl CoinControlManagerState {
        #[uniffi::constructor(default(output_count = 20, change_count = 4))]
        pub fn preview_new(output_count: u8, change_count: u8) -> Self {
            let wallet_id = WalletId::preview_new();
            let utxos = preview_new_utxo_list(output_count, change_count);
            Self { wallet_id, utxos }
        }
    }
}
