mod state;

use std::{
    hash::{DefaultHasher, Hash, Hasher as _},
    sync::Arc,
};

use ahash::HashSet;
use bdk_wallet::LocalOutput;
use cove_types::{
    OutPoint, WalletId,
    amount::Amount,
    unit::BitcoinUnit,
    utxo::{Utxo, UtxoType},
};
use cove_util::result_ext::ResultExt as _;
use parking_lot::Mutex;

use crate::{
    label_manager::{LabelManager, LabelManagerError},
    manager::deferred_sender::{self, DeferredSender},
    wallet::metadata::WalletMetadata,
};
use cove_tokio::task;
use flume::Receiver;
use tracing::trace;

use super::deferred_sender::MessageSender;

type Message = CoinControlManagerReconcileMessage;
type Action = CoinControlManagerAction;
type State = state::CoinControlManagerState;
type SortState = CoinControlListSortState;
type Reconciler = dyn CoinControlManagerReconciler;
type SingleOrMany = deferred_sender::SingleOrMany<Message>;

#[uniffi::export(callback_interface)]
pub trait CoinControlManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustCoinControlManager {
    pub state: Arc<Mutex<State>>,
    pub reconciler: MessageSender<Message>,
    pub reconcile_receiver: Arc<Receiver<SingleOrMany>>,
}
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CoinControlManagerReconcileMessage {
    ClearSort,
    UpdateSort(CoinControlListSort),
    UpdateUtxos(Vec<Utxo>),
    UpdateSearch(String),
    UpdateSelectedUtxos { utxos: Vec<Arc<OutPoint>>, total_value: Arc<Amount> },
    UpdateUnit(BitcoinUnit),
    UpdateLockStateLoadFailed(bool),
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
    pub fn unit(&self) -> BitcoinUnit {
        self.state.lock().unit
    }

    #[uniffi::method]
    pub fn lock_state_load_failed(&self) -> bool {
        self.state.lock().lock_state_load_failed
    }

    #[uniffi::method]
    pub async fn reload_labels(&self) {
        let (utxos, selected_utxos, total_value, lock_state_load_failed) = {
            let mut state = self.state.lock();

            let old_utxos_hash = hash_utxos(&state.utxos());
            let old_lock_state_load_failed = state.lock_state_load_failed;

            let selection_changed = state.load_utxo_labels();

            let new_utxos = state.utxos();
            let lock_state_load_failed = state.lock_state_load_failed;
            let new_utxos_hash = hash_utxos(&new_utxos);

            if old_utxos_hash == new_utxos_hash
                && !selection_changed
                && old_lock_state_load_failed == lock_state_load_failed
            {
                return;
            }

            let selected_utxos = state.selected_utxos.clone();
            let total_value = total_value_of_spendable_utxos(&state.utxos, &selected_utxos).into();

            (new_utxos, selected_utxos, total_value, lock_state_load_failed)
        };

        self.reconciler.send_async(Message::UpdateUtxos(utxos)).await;
        self.reconciler
            .send_async(Message::UpdateSelectedUtxos { utxos: selected_utxos, total_value })
            .await;
        self.reconciler
            .send_async(Message::UpdateLockStateLoadFailed(lock_state_load_failed))
            .await;
    }

    #[uniffi::method]
    pub fn id(&self) -> WalletId {
        self.state.lock().wallet_id.clone()
    }

    #[uniffi::method]
    pub async fn set_utxo_spendability(
        &self,
        outpoint: Arc<OutPoint>,
        spendable: bool,
    ) -> Result<(), LabelManagerError> {
        let wallet_id = self.state.lock().wallet_id.clone();
        let outpoint = bitcoin::OutPoint::from(outpoint.as_ref());

        LabelManager::try_new(wallet_id)
            .map_err_str(LabelManagerError::SaveOutputLabels)?
            .set_output_spendability_for_outpoints(vec![outpoint], spendable)?;

        self.reload_labels().await;

        Ok(())
    }

    #[uniffi::method]
    pub fn selected_utxos(&self) -> Vec<Utxo> {
        let state = self.state.lock();
        let selected_utxos_ids: HashSet<Arc<OutPoint>> =
            state.selected_utxos.iter().cloned().collect();

        state
            .utxos
            .iter()
            .filter(|utxo| utxo.spendable && selected_utxos_ids.contains(&utxo.outpoint))
            .cloned()
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
                self.sort_button_pressed(sort_button_pressed);
            }
            Action::NotifySearchChanged(search) => self.notify_search_changed(search),
            Action::ClearSearch => {
                self.clone().notify_search_changed(String::new());
                self.reconciler.send(Message::UpdateSearch(String::new()));
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

                self.reconciler.send(Message::UpdateUnit(new_unit));
            }
            Action::NotifySelectedUtxosChanged(selected_utxos) => {
                self.notify_selected_utxos_changed(selected_utxos);
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
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        }
    }

    pub fn total_value_of_utxos(&self, selected_utxo_ids: &[Arc<OutPoint>]) -> Amount {
        let state = self.state.lock();
        total_value_of_spendable_utxos(&state.utxos, selected_utxo_ids)
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

        let mut sender = DeferredSender::new(self.reconciler.clone());
        let sort = get_new_sort(current_sort, sort_button_pressed);

        self.state.lock().sort = SortState::Active(sort);
        sender.queue(Message::UpdateSort(sort));

        self.state.lock().sort_utxos(sort);
        sender.queue(Message::UpdateUtxos(self.utxos()));
    }

    fn toggle_select_all(self: Arc<Self>) {
        let mut sender = DeferredSender::new(self.reconciler.clone());

        let old_selected_utxos = self.state.lock().selected_utxos.clone();

        let new_selected_utxos = if old_selected_utxos.is_empty() {
            self.utxos()
                .into_iter()
                .filter(|utxo| utxo.spendable)
                .map(|utxo| utxo.outpoint)
                .collect()
        } else {
            vec![]
        };

        if new_selected_utxos == old_selected_utxos {
            self.state.lock().selected_utxos = new_selected_utxos;
        } else {
            self.state.lock().selected_utxos = new_selected_utxos.clone();
            let total_value = self.total_value_of_utxos(&new_selected_utxos).into();
            sender.queue(Message::UpdateSelectedUtxos { utxos: new_selected_utxos, total_value });
        }
    }

    fn notify_selected_utxos_changed(self: Arc<Self>, selected_utxos: Vec<Arc<OutPoint>>) {
        let (selected_utxos, total_value) = {
            let mut state = self.state.lock();
            let spendable_outpoints = state
                .utxos
                .iter()
                .filter(|utxo| utxo.spendable)
                .map(|utxo| utxo.outpoint.clone())
                .collect::<HashSet<_>>();
            let selected_utxos = selected_utxos
                .into_iter()
                .filter(|outpoint| spendable_outpoints.contains(outpoint))
                .collect::<Vec<_>>();
            let total_value = total_value_of_spendable_utxos(&state.utxos, &selected_utxos).into();

            state.selected_utxos = selected_utxos.clone();

            (selected_utxos, total_value)
        };

        self.reconciler.send(Message::UpdateSelectedUtxos { utxos: selected_utxos, total_value });
    }

    fn notify_search_changed(self: Arc<Self>, search: String) {
        if search == self.state.lock().search {
            return;
        }

        let mut sender = DeferredSender::new(self.reconciler.clone());

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
}

fn total_value_of_spendable_utxos(utxos: &[Utxo], selected_utxo_ids: &[Arc<OutPoint>]) -> Amount {
    let selected_ids: HashSet<Arc<OutPoint>> = selected_utxo_ids.iter().cloned().collect();

    let final_amount_sats = utxos
        .iter()
        .filter(|utxo| utxo.spendable && selected_ids.contains(&utxo.outpoint))
        .map(|utxo| utxo.amount.as_sats())
        .sum();

    Amount::from_sat(final_amount_sats)
}

fn hash_utxos(utxos: &[Utxo]) -> u64 {
    let mut hasher = DefaultHasher::new();
    utxos.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::wallet_data::test_support::new_test_wallet_data_db;
    use bdk_wallet::{
        KeychainKind,
        chain::{BlockId, ChainPosition, ConfirmationBlockTime},
    };
    use bitcoin::{
        BlockHash, CompressedPublicKey, OutPoint as BitcoinOutPoint, PrivateKey, ScriptBuf, TxOut,
        Txid, hashes::Hash as _, secp256k1::SecretKey,
    };

    fn preview_manager_with_locked_first_utxo() -> Arc<RustCoinControlManager> {
        let manager = Arc::new(RustCoinControlManager::preview_new(2, 0));
        manager.state.lock().utxos[0].spendable = false;

        manager
    }

    fn confirmed_position() -> ChainPosition<ConfirmationBlockTime> {
        ChainPosition::Confirmed {
            anchor: ConfirmationBlockTime {
                block_id: BlockId { height: 1, hash: BlockHash::all_zeros() },
                confirmation_time: 1,
            },
            transitively: None,
        }
    }

    fn mainnet_script() -> ScriptBuf {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1; 32]).expect("failed to create secret key");
        let private_key = PrivateKey::new(secret_key, bitcoin::Network::Bitcoin);
        let public_key = CompressedPublicKey::from_private_key(&secp, &private_key)
            .expect("failed to create public key");

        bitcoin::Address::p2wpkh(&public_key, bitcoin::Network::Bitcoin).script_pubkey()
    }

    fn local_output(
        outpoint: BitcoinOutPoint,
        chain_position: ChainPosition<ConfirmationBlockTime>,
    ) -> LocalOutput {
        LocalOutput {
            outpoint,
            txout: TxOut {
                value: bitcoin::Amount::from_sat(1_000),
                script_pubkey: mainnet_script(),
            },
            keychain: KeychainKind::External,
            is_spent: false,
            derivation_index: 0,
            chain_position,
        }
    }

    #[test]
    fn load_utxo_labels_loads_spendability_and_prunes_locked_selection() {
        let manager = RustCoinControlManager::preview_new(2, 0);
        let wallet_id = manager.state.lock().wallet_id.clone();
        let locked = manager.state.lock().utxos[0].outpoint.clone();
        let unlocked = manager.state.lock().utxos[1].outpoint.clone();
        let (wallet_db, _tmp) = new_test_wallet_data_db(wallet_id);

        wallet_db
            .labels
            .set_output_spendability(bitcoin::OutPoint::from(locked.as_ref()), false)
            .expect("failed to lock output");

        let selection_changed = {
            let mut state = manager.state.lock();
            state.selected_utxos = vec![locked.clone(), unlocked.clone()];
            state.load_utxo_labels()
        };

        let state = manager.state.lock();

        assert!(selection_changed);
        assert!(!state.utxos[0].spendable);
        assert_eq!(state.selected_utxos, vec![unlocked]);
    }

    #[test]
    fn load_utxo_labels_fails_closed_when_wallet_database_cannot_open() {
        let mut state = State::preview_new(2, 0);
        state.wallet_id = WalletId::from("invalid\0wallet-id");
        state.selected_utxos = vec![state.utxos[0].outpoint.clone()];

        let selection_changed = state.load_utxo_labels();

        assert!(selection_changed);
        assert!(state.lock_state_load_failed);
        assert!(state.selected_utxos.is_empty());
        assert!(state.utxos.iter().all(|utxo| !utxo.spendable));
    }

    #[test]
    fn select_all_skips_locked_utxos() {
        let manager = preview_manager_with_locked_first_utxo();
        let locked = manager.state.lock().utxos[0].outpoint.clone();

        manager.clone().toggle_select_all();

        let selected = manager.state.lock().selected_utxos.clone();
        assert_eq!(selected.len(), 1);
        assert!(!selected.contains(&locked));
    }

    #[test]
    fn direct_selection_prunes_locked_utxos() {
        let manager = preview_manager_with_locked_first_utxo();
        let locked = manager.state.lock().utxos[0].outpoint.clone();
        let unlocked = manager.state.lock().utxos[1].outpoint.clone();

        manager.clone().notify_selected_utxos_changed(vec![locked, unlocked.clone()]);

        assert_eq!(manager.state.lock().selected_utxos, vec![unlocked]);
    }

    #[test]
    fn direct_selection_preserves_selected_utxos_outside_active_search() {
        let manager = Arc::new(RustCoinControlManager::preview_new(2, 0));
        let hidden = manager.state.lock().utxos[0].outpoint.clone();
        let visible = manager.state.lock().utxos[1].outpoint.clone();
        let expected_total = {
            let mut state = manager.state.lock();
            state.utxos[0].label = Some("outside active search".to_string());
            state.utxos[1].label = Some("needle".to_string());
            state.utxos[0].amount.as_sats() + state.utxos[1].amount.as_sats()
        };

        manager.clone().notify_search_changed("needle".to_string());

        let visible_outpoints =
            manager.utxos().into_iter().map(|utxo| utxo.outpoint).collect::<Vec<_>>();
        assert_eq!(visible_outpoints, vec![visible.clone()]);

        manager.clone().notify_selected_utxos_changed(vec![hidden.clone(), visible.clone()]);

        let selected_utxos = manager.state.lock().selected_utxos.clone();

        assert_eq!(selected_utxos, vec![hidden, visible]);
        assert_eq!(manager.total_value_of_utxos(&selected_utxos).as_sats(), expected_total);
        assert_eq!(manager.selected_utxos().len(), 2);
    }

    #[test]
    fn selected_total_excludes_locked_utxos() {
        let manager = preview_manager_with_locked_first_utxo();
        let locked = manager.state.lock().utxos[0].outpoint.clone();
        let unlocked = manager.state.lock().utxos[1].outpoint.clone();
        let unlocked_amount = manager.state.lock().utxos[1].amount.as_sats();

        let total = manager.total_value_of_utxos(&[locked, unlocked]);

        assert_eq!(total.as_sats(), unlocked_amount);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reload_labels_preserves_active_search_results() {
        let manager = Arc::new(RustCoinControlManager::preview_new(2, 0));
        let wallet_id = manager.state.lock().wallet_id.clone();
        let visible = manager.state.lock().utxos[1].outpoint.clone();
        let search = visible.txid.to_string();
        let (wallet_db, _tmp) = new_test_wallet_data_db(wallet_id);

        manager.clone().notify_search_changed(search);
        while manager.reconcile_receiver.try_recv().is_ok() {}

        wallet_db
            .labels
            .set_output_spendability(bitcoin::OutPoint::from(visible.as_ref()), false)
            .expect("failed to lock searched output");

        manager.reload_labels().await;

        let message = manager.reconcile_receiver.recv_async().await.expect("reconcile message");
        let SingleOrMany::Single(Message::UpdateUtxos(utxos)) = message else {
            panic!("expected utxo update");
        };

        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].outpoint, visible);
        assert!(!utxos[0].spendable);
    }

    #[test]
    fn pending_locked_output_appears_locked_after_confirmation() {
        let metadata = WalletMetadata::preview_new();
        let wallet_id = metadata.id.clone();
        let outpoint = BitcoinOutPoint { txid: Txid::from_byte_array([3; 32]), vout: 0 };
        let (wallet_db, _tmp) = new_test_wallet_data_db(wallet_id);

        wallet_db
            .labels
            .set_output_spendability(outpoint, false)
            .expect("failed to lock pending output");

        let mut pending_state = State::new(
            metadata.clone(),
            vec![local_output(
                outpoint,
                ChainPosition::Unconfirmed { first_seen: Some(1), last_seen: Some(1) },
            )],
        );
        pending_state.load_utxo_labels();

        assert!(pending_state.utxos.is_empty());

        let mut confirmed_state =
            State::new(metadata, vec![local_output(outpoint, confirmed_position())]);
        confirmed_state.load_utxo_labels();

        assert_eq!(confirmed_state.utxos.len(), 1);
        assert!(!confirmed_state.utxos[0].spendable);
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
#[uniffi::export(Display)]
pub enum CoinControlListSortKey {
    Date,
    Name,
    Amount,
    Change,
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
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active(_))
    }

    pub const fn sorter(&self) -> CoinControlListSort {
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
    pub const fn reverse(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

impl CoinControlListSortKey {
    pub const fn to_default_sort(self) -> CoinControlListSort {
        match self {
            Self::Date => CoinControlListSort::Date(ListSortDirection::Descending),
            Self::Amount => CoinControlListSort::Amount(ListSortDirection::Descending),
            Self::Name => CoinControlListSort::Name(ListSortDirection::Descending),
            Self::Change => CoinControlListSort::Change(UtxoType::Output),
        }
    }
}

// MARK: FFI
#[uniffi::export]
impl RustCoinControlManager {
    #[uniffi::constructor(default(output_count = 20, change_count = 4))]
    pub fn preview_new(output_count: u8, change_count: u8) -> Self {
        let (sender, receiver) = flume::bounded(10);

        let state = State::preview_new(output_count, change_count);
        Self {
            state: Arc::new(Mutex::new(state)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        }
    }
}
