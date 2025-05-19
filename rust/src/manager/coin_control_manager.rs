use std::{hash::Hasher, sync::Arc};

use bdk_wallet::LocalOutput;
use cove_types::{
    Network, OutPoint, WalletId,
    utxo::{Utxo, UtxoType},
};
use parking_lot::Mutex;

use crate::manager::deferred_sender::{self, DeferredSender};
use crate::task;
use cove_macros::impl_manager_message_send;
use flume::{Receiver, Sender, TrySendError};
use tracing::{debug, error, trace, warn};

#[allow(dead_code)]
type Message = CoinControlManagerReconcileMessage;
type Action = CoinControlManagerAction;
type State = CoinControlManagerState;
type Reconciler = dyn CoinControlManagerReconciler;
type SingleOrMany = deferred_sender::SingleOrMany<Message>;
impl_manager_message_send!(RustCoinControlManager);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerReconcileMessage {
    UpdateSort(CoinControlListSort),
    UpdateUtxos(Vec<Utxo>),
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
    sort: CoinControlListSort,
    selected_utxos: Vec<OutPoint>,
    search: String,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerAction {
    ChangeSort(CoinControlListSortKey),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ButtonPresentation {
    NotSelected,
    Selected(ListSortDirection),
}

#[uniffi::export]
impl RustCoinControlManager {
    #[uniffi::method]
    pub fn utxos(self: &Arc<Self>) -> Vec<Utxo> {
        self.state.lock().utxos.clone()
    }

    #[uniffi::method]
    pub fn button_presentation(
        self: &Arc<Self>,
        button: CoinControlListSortKey,
    ) -> ButtonPresentation {
        use ButtonPresentation as Present;
        use CoinControlListSort as Sort;
        use CoinControlListSortKey as Key;
        use ListSortDirection as D;
        let sort = self.state.lock().sort;

        match (sort, button) {
            (Sort::Date(d), Key::Date) => Present::Selected(d),
            (Sort::Amount(d), Key::Amount) => Present::Selected(d),
            (Sort::Name(d), Key::Name) => Present::Selected(d),
            (Sort::Change(UtxoType::Output), Key::Change) => Present::Selected(D::Ascending),
            (Sort::Change(UtxoType::Change), Key::Change) => Present::Selected(D::Descending),
            _ => Present::NotSelected,
        }
    }

    // MARK: boilerplate

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
    pub fn dispatch(self: Arc<Self>, action: Action) {
        match action {
            CoinControlManagerAction::ChangeSort(sort_button_pressed) => {
                self.sort_button_pressed(sort_button_pressed);
            }
        }
    }
}

impl RustCoinControlManager {
    pub fn new(id: WalletId, local_outputs: Vec<LocalOutput>, network: Network) -> Self {
        let (sender, receiver) = flume::bounded(10);

        let mut state = State::new(id, local_outputs, network);
        state.sort_utxos(CoinControlListSort::Date(ListSortDirection::Descending));

        Self {
            state: Arc::new(Mutex::new(state)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    fn sort_button_pressed(self: &Arc<Self>, sort_button_pressed: CoinControlListSortKey) {
        use CoinControlListSort as Sort;
        use CoinControlListSortKey as Key;

        let mut sender = DeferredSender::new(self.clone());

        let current_sort = self.state.lock().sort;
        let sort = match (current_sort, sort_button_pressed) {
            (Sort::Date(sort_direction), Key::Date) => Sort::Date(sort_direction.reverse()),
            (_, Key::Date) => Sort::Date(ListSortDirection::Descending),

            (Sort::Amount(sort_directino), Key::Amount) => Sort::Amount(sort_directino.reverse()),
            (_, Key::Amount) => Sort::Amount(ListSortDirection::Descending),

            (Sort::Name(sort_direction), Key::Name) => Sort::Name(sort_direction.reverse()),
            (_, Key::Name) => Sort::Name(ListSortDirection::Descending),

            (Sort::Change(sort_direction), Key::Change) => Sort::Change(sort_direction.reverse()),
            (_, Key::Change) => Sort::Change(UtxoType::Output),
        };

        self.state.lock().sort = sort;
        sender.queue(Message::UpdateSort(sort));

        self.state.lock().sort_utxos(sort);
        sender.queue(Message::UpdateUtxos(self.utxos()));
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

impl ListSortDirection {
    pub fn reverse(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

// MARK: STATE
impl State {
    pub fn new(wallet_id: WalletId, unspent: Vec<LocalOutput>, network: Network) -> Self {
        let utxos =
            unspent.into_iter().filter_map(|o| Utxo::try_from_local(o, network).ok()).collect();

        let sort = CoinControlListSort::Date(ListSortDirection::Descending);
        let selected_utxos = vec![];
        let search = String::new();

        Self { wallet_id, utxos, sort, selected_utxos, search }
    }

    pub fn sort_utxos(&mut self, sort: CoinControlListSort) {
        let mut utxos = self.utxos.clone();

        match sort {
            CoinControlListSort::Date(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.datetime.cmp(&b.datetime).reverse());
            }
            CoinControlListSort::Date(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.datetime.cmp(&b.datetime));
            }

            CoinControlListSort::Name(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.label.cmp(&b.label).reverse());
            }
            CoinControlListSort::Name(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.label.cmp(&b.label));
            }

            CoinControlListSort::Amount(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.amount.cmp(&b.amount).reverse());
            }

            CoinControlListSort::Amount(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.amount.cmp(&b.amount));
            }

            CoinControlListSort::Change(UtxoType::Output) => {
                utxos.sort_by(|a, b| a.type_.cmp(&b.type_).reverse());
            }

            CoinControlListSort::Change(UtxoType::Change) => {
                utxos.sort_by(|a, b| a.type_.cmp(&b.type_));
            }
        }
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
            let sort = CoinControlListSort::Date(ListSortDirection::Descending);
            let selected_utxos = vec![];
            let search = String::new();
            Self { wallet_id, utxos, sort, selected_utxos, search }
        }
    }
}
