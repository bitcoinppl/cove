mod state;

use std::sync::Arc;

use ahash::HashSet;
use bdk_wallet::LocalOutput;
use cove_types::{
    OutPoint,
    amount::Amount,
    unit::Unit,
    utxo::{Utxo, UtxoType},
};
use parking_lot::Mutex;

use crate::task;
use crate::{
    manager::deferred_sender::{self, DeferredSender},
    wallet::metadata::WalletMetadata,
};
use cove_macros::impl_manager_message_send;
use flume::{Receiver, Sender, TrySendError};
use tracing::{debug, error, trace, warn};

#[allow(dead_code)]
type Message = CoinControlManagerReconcileMessage;
type Action = CoinControlManagerAction;
type State = state::CoinControlManagerState;
type SortState = CoinControlListSortState;
type Reconciler = dyn CoinControlManagerReconciler;
type SingleOrMany = deferred_sender::SingleOrMany<Message>;
impl_manager_message_send!(RustCoinControlManager);

#[uniffi::export(callback_interface)]
pub trait CoinControlManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Clone, Debug, uniffi::Object)]
#[allow(dead_code)]
pub struct RustCoinControlManager {
    pub state: Arc<Mutex<State>>,
    pub reconciler: Sender<SingleOrMany>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,
}
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerReconcileMessage {
    ClearSort,
    UpdateSort(CoinControlListSort),
    UpdateUtxos(Vec<Utxo>),
    UpdateSearch(String),
    UpdateSelectedUtxos(Vec<Arc<OutPoint>>),
    UpdateUnit(Unit),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerAction {
    ChangeSort(CoinControlListSortKey),
    ClearSearch,

    ToggleSelectAll,
    ToggleUnit,

    NotifySelectedUtxosChanged(Vec<Arc<OutPoint>>),
    NotifySearchChanged(String),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ButtonPresentation {
    NotSelected,
    Selected(ListSortDirection),
}

#[uniffi::export]
impl RustCoinControlManager {
    #[uniffi::method]
    pub fn utxos(&self) -> Vec<Utxo> {
        self.state.lock().utxos()
    }

    #[uniffi::method]
    pub fn unit(&self) -> Unit {
        self.state.lock().unit
    }

    #[uniffi::method]
    pub fn total_selected_amount(&self) -> Amount {
        let selected_utxos_ids: HashSet<Arc<OutPoint>> =
            self.state.lock().selected_utxos.iter().cloned().collect();

        let final_amount_sats = self
            .utxos()
            .into_iter()
            .filter(|utxo| selected_utxos_ids.contains(&utxo.outpoint))
            .map(|utxo| utxo.amount.as_sats())
            .sum();

        Amount::from_sat(final_amount_sats)
    }

    #[uniffi::method]
    pub fn selected_utxos(&self) -> Vec<Utxo> {
        let selected_utxos_ids: HashSet<Arc<OutPoint>> =
            self.state.lock().selected_utxos.iter().cloned().collect();

        self.utxos()
            .into_iter()
            .filter(|utxo| selected_utxos_ids.contains(&utxo.outpoint))
            .collect()
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

        let sort = match sort {
            SortState::Active(sort) => sort,
            SortState::Inactive(_) => return Present::NotSelected,
        };

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
            Action::ChangeSort(sort_button_pressed) => {
                self.sort_button_pressed(sort_button_pressed)
            }
            Action::NotifySearchChanged(search) => self.notify_search_changed(search),
            Action::ClearSearch => {
                self.clone().notify_search_changed(String::new());
                self.send(Message::UpdateSearch(String::new()));
            }
            Action::ToggleSelectAll => {
                self.clone().toggle_select_all();
            }
            Action::ToggleUnit => {
                let new_unit = {
                    let mut state = self.state.lock();
                    let new_unit = state.unit.toggle();
                    state.unit = new_unit;
                    new_unit
                };

                self.send(Message::UpdateUnit(new_unit));
            }
            Action::NotifySelectedUtxosChanged(selected_utxos) => {
                self.state.lock().selected_utxos = selected_utxos;
            }
        }
    }
}

impl RustCoinControlManager {
    pub fn new(metadata: WalletMetadata, local_outputs: Vec<LocalOutput>) -> Self {
        let (sender, receiver) = flume::bounded(10);

        let mut state = State::new(metadata, local_outputs);

        state.sort_utxos(CoinControlListSort::Date(ListSortDirection::Descending));
        state.load_utxo_labels();

        Self {
            state: Arc::new(Mutex::new(state)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    fn sort_button_pressed(self: Arc<Self>, sort_button_pressed: CoinControlListSortKey) {
        use CoinControlListSort as Sort;
        use CoinControlListSortKey as Key;

        let current_sort = self.state.lock().sort;
        fn get_new_sort(current_sort: SortState, button: Key) -> CoinControlListSort {
            if !current_sort.is_active() {
                return button.to_default_sort();
            }

            match (current_sort.sorter(), button) {
                (Sort::Date(sort_direction), Key::Date) => Sort::Date(sort_direction.reverse()),

                (Sort::Amount(sort_direction), Key::Amount) => {
                    Sort::Amount(sort_direction.reverse())
                }

                (Sort::Name(sort_direction), Key::Name) => Sort::Name(sort_direction.reverse()),

                (Sort::Change(sort_direction), Key::Change) => {
                    Sort::Change(sort_direction.reverse())
                }

                _ => button.to_default_sort(),
            }
        }

        let mut sender = DeferredSender::new(self.clone());
        let sort = get_new_sort(current_sort, sort_button_pressed);

        self.state.lock().sort = SortState::Active(sort);
        sender.queue(Message::UpdateSort(sort));

        self.state.lock().sort_utxos(sort);
        sender.queue(Message::UpdateUtxos(self.utxos()));
    }

    fn toggle_select_all(self: Arc<Self>) {
        let mut sender = DeferredSender::new(self.clone());

        let selected_utxos = self.state.lock().selected_utxos.clone();
        if selected_utxos.is_empty() {
            let selected = self.utxos().into_iter().map(|utxo| utxo.outpoint).collect();
            self.state.lock().selected_utxos = selected;
        } else {
            self.state.lock().selected_utxos = vec![];
        }

        let new_selected = &self.state.lock().selected_utxos;
        if new_selected != &selected_utxos {
            sender.queue(Message::UpdateSelectedUtxos(new_selected.clone()));
        }
    }

    fn notify_search_changed(self: Arc<Self>, search: String) {
        if search == self.state.lock().search {
            return;
        }

        let mut sender = DeferredSender::new(self.clone());

        // update the search state
        self.state.lock().search = search.clone();

        if search.is_empty() {
            let sort = self.state.lock().sort.sorter();
            let sort_state = SortState::Active(sort);

            self.state.lock().sort = sort_state;
            self.state.lock().reset_search();

            let utxos = self.utxos();
            sender.queue(Message::UpdateUtxos(utxos));
            sender.queue(Message::UpdateSort(sort));

            return;
        }

        self.state.lock().filter_utxos(&search);

        let utxos = self.utxos();
        sender.queue(Message::UpdateUtxos(utxos));

        // clear the sort if searching
        let has_sort = self.state.lock().sort.is_active();

        if has_sort {
            let sort = self.state.lock().sort.sorter();
            self.state.lock().sort = SortState::Inactive(sort);
            sender.queue(Message::ClearSort);
        }
    }

    fn send(self: &Arc<Self>, message: impl Into<SingleOrMany>) {
        let message = message.into();
        debug!("send: {message:?}");
        match self.reconciler.try_send(message) {
            Ok(_) => {}
            Err(TrySendError::Full(message)) => {
                warn!("[WARN] unable to send, queue is full sending async");

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
pub enum CoinControlListSortState {
    Active(CoinControlListSort),
    Inactive(CoinControlListSort),
}

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

impl Default for SortState {
    fn default() -> Self {
        Self::Active(CoinControlListSort::default())
    }
}

impl SortState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active(_))
    }

    pub fn sorter(&self) -> CoinControlListSort {
        match self {
            Self::Active(sort) => *sort,
            Self::Inactive(sort) => *sort,
        }
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

impl CoinControlListSortKey {
    pub fn to_default_sort(self) -> CoinControlListSort {
        match self {
            Self::Date => CoinControlListSort::Date(ListSortDirection::Descending),
            Self::Amount => CoinControlListSort::Amount(ListSortDirection::Descending),
            Self::Name => CoinControlListSort::Name(ListSortDirection::Descending),
            Self::Change => CoinControlListSort::Change(UtxoType::Output),
        }
    }
}

mod ffi {

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
}
