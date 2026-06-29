mod address_input;
pub mod alert_state;
mod amount_input;
pub mod amount_or_max;
pub mod btc_on_change;
mod coin_control;
pub mod error;
mod fee_selection;
pub mod fiat_on_change;
mod finalize;
mod psbt_builder;
mod read_api;
mod sanitize;
pub mod state;
mod validation;

use std::{sync::Arc, time::Duration};

use cove_tokio::DebouncedTask;

use crate::{
    app::App,
    fee_client::FEE_CLIENT,
    fiat::client::PriceResponse,
    wallet::{
        Address,
        balance::Balance,
        metadata::{FiatOrBtc, WalletMetadata},
    },
};
use act_zero::WeakAddr;
use alert_state::SendFlowAlertState;
use amount_or_max::AmountOrMax;
use backon::{ExponentialBuilder, Retryable};
use btc_on_change::BtcOnChangeHandler;
use cove_common::consts::MIN_SEND_SATS;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
    unit::BitcoinUnit,
    utxo::Utxo,
};
use error::SendFlowError;
use fiat_on_change::FiatOnChangeHandler;
use flume::Receiver;
use parking_lot::Mutex;
use state::{CoinControlMode, EnterMode, FeeSelection, SendFlowManagerState, State};
use tracing::{debug, error, trace};

use super::{
    deferred_sender::{self, MessageSender},
    wallet_manager::{RustWalletManager, actor::WalletActor},
};

pub type Error = error::SendFlowError;
type Result<T, E = Error> = std::result::Result<T, E>;

type Action = SendFlowManagerAction;
type Message = SendFlowManagerReconcileMessage;
type Reconciler = dyn SendFlowManagerReconciler;
type SingleOrMany = deferred_sender::SingleOrMany<Message>;
type DeferredSender = deferred_sender::DeferredSender<Message>;

const LOCK_STATE_LOAD_FAILED_ERROR_ID: &str = "send_flow_lock_state_load_failed";

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SetAmountFocusField {
    Amount,
    Address,
}

#[uniffi::export(callback_interface)]
pub trait SendFlowManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Debug, uniffi::Object)]
pub struct RustSendFlowManager {
    app: App,

    wallet_manager: Arc<RustWalletManager>,
    pub state: Arc<Mutex<SendFlowManagerState>>,

    reconciler: MessageSender<Message>,
    reconcile_receiver: Arc<Receiver<SingleOrMany>>,

    fee_check_task: DebouncedTask<()>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerReconcileMessage {
    // reconcile state with swift
    UpdateEnteringBtcAmount(String),
    UpdateEnteringFiatAmount(String),
    UpdateEnteringAddress(String),
    UpdateAddress(Option<Arc<Address>>),

    SetMaxSelected(Arc<Amount>),
    UnsetMaxSelected,

    UpdateAmountSats(u64),
    UpdateAmountFiat(f64),

    UpdateFocusField(Option<SetAmountFocusField>),

    UpdateFeeSelection(FeeSelection),

    RefreshPresenters,

    // side effects
    SetAlert(SendFlowAlertState),
    ClearAlert,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerAction {
    ChangeEnteringAddress(String),

    ChangeSetAmountFocusField(Option<SetAmountFocusField>),

    SelectMaxSend,
    ClearSendAmount,
    ClearAddress,
    RefreshWalletBalance,

    SetCoinControlMode(Vec<Utxo>),
    DisableCoinControlMode,

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),

    // front end changing text fields
    NotifyEnteringBtcAmountChanged(String),
    NotifyEnteringFiatAmountChanged(String),
    NotifyEnteringAddressChanged(String),

    // front end lets the one of the values were changed
    NotifySelectedUnitedChanged { old: BitcoinUnit, new: BitcoinUnit },
    NotifyBtcOrFiatChanged { old: FiatOrBtc, new: FiatOrBtc },
    NotifyScanCodeChanged { old: String, new: String },
    NotifyPricesChanged(Arc<PriceResponse>),
    NotifyFocusFieldChanged { old: Option<SetAmountFocusField>, new: Option<SetAmountFocusField> },

    // starting with an amount and address from scan
    NotifyAddressChanged(Arc<Address>),
    NotifyAmountChanged(Arc<Amount>),

    // notify coin control custom amount changed
    NotifyCoinControlAmountChanged(f64),
    NotifyCoinControlEnteredAmountChanged(String, bool),

    // custom fee selection
    ChangeFeeRateOptions(Arc<FeeRateOptionsWithTotalFee>),

    FinalizeAndGoToNextScreen,
}

impl RustSendFlowManager {
    pub fn new(
        metadata: WalletMetadata,
        balance: Arc<Balance>,
        wallet_manager: Arc<RustWalletManager>,
    ) -> Arc<Self> {
        let (sender, receiver) = flume::bounded(50);
        let state = State::new(metadata, balance);
        let message_sender = MessageSender::new(sender);

        // immediately populate cached values if available
        let has_base_fees = if let Some(fee_response) = FEE_CLIENT.fees() {
            let base_options = FeeRateOptions::from(fee_response);
            let fee_options = FeeRateOptionsWithTotalFee::without_totals(base_options);
            let selected = Arc::new(fee_options.medium);
            let fee_selection = FeeSelection::new(Arc::new(fee_options), selected);

            let mut state_guard = state.lock();
            state_guard.fee_rate_options_base = Some(Arc::new(base_options));
            state_guard.fee_selection = Some(fee_selection);
            state_guard.has_base_fees = true;
            true
        } else {
            false
        };

        debug!(
            "SendFlowManager::new - has_base_fees: {}, balance: {:?}",
            has_base_fees,
            state.lock().wallet_balance
        );

        let me: Arc<Self> = Self {
            app: App::global().clone(),
            state: state.into_inner(),
            wallet_manager,
            reconciler: message_sender,
            reconcile_receiver: Arc::new(receiver),
            fee_check_task: DebouncedTask::new("fee_check", Duration::from_millis(200)),
        }
        .into();

        // run all init tasks in background (parallel)
        me.background_init_tasks();
        me
    }

    fn wallet_actor(&self) -> WeakAddr<WalletActor> {
        self.wallet_manager.actor.downgrade()
    }
}

#[uniffi::export]
impl RustSendFlowManager {
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        cove_tokio::task::spawn(async move {
            while let Ok(field) = reconcile_receiver.recv_async().await {
                trace!("reconcile_receiver: {field:?}");
                match field {
                    SingleOrMany::Single(message) => reconciler.reconcile(message),
                    SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
                }
            }
        });
    }

    // MARK: Validators
    #[uniffi::method(default(display_alert = false))]
    pub fn validate_address(self: &Arc<Self>, display_alert: bool) -> bool {
        self.validate_address_internal(display_alert)
    }

    #[uniffi::method(default(display_alert = false))]
    pub fn validate_fee_percentage(self: &Arc<Self>, display_alert: bool) -> bool {
        self.validate_fee_percentage_internal(display_alert)
    }

    #[uniffi::method(default(display_alert = false))]
    pub fn validate_amount(self: &Arc<Self>, display_alert: bool) -> bool {
        self.validate_amount_internal(display_alert)
    }
}

impl RustSendFlowManager {
    pub(crate) fn validate_address_internal(self: &Arc<Self>, display_alert: bool) -> bool {
        if self.state.lock().address.is_none() {
            if display_alert {
                let error =
                    SendFlowError::InvalidAddress(self.state.lock().entering_address.clone());
                self.reconciler.send(Message::SetAlert(error.into()));
            }

            return false;
        }

        true
    }
    pub(crate) fn validate_fee_percentage_internal(self: &Arc<Self>, display_alert: bool) -> bool {
        let Some(amount) = self.state.lock().amount_sats else { return false };
        let Some(fee_rate) = self.selected_fee_rate() else { return false };
        let Some(total_fee) = fee_rate.total_fee() else { return false };

        let fee_sats = total_fee.as_sats();
        let fee_percentage = fee_sats * 100 / amount;

        debug!("validate_fee_percentage: {fee_sats} / {amount} = {fee_percentage} ");
        if fee_percentage > 100 {
            let error = SendFlowAlertState::General {
                title: "Fee Too High!".to_string(),
                message: "The fee is higher than the amount you are sending".to_string(),
            };

            if display_alert {
                self.reconciler.send(Message::SetAlert(error));
            }

            return false;
        }

        if fee_percentage > 20 {
            let error = SendFlowAlertState::General {
                title: "Warning, High Fee!".to_string(),
                message: "The fee is higher than 20% of the amount you are sending".to_string(),
            };

            if display_alert {
                self.reconciler.send(Message::SetAlert(error));
            }

            // just a warning not a error
            return true;
        }

        true
    }
    pub(crate) fn validate_amount_internal(self: &Arc<Self>, display_alert: bool) -> bool {
        let mut sender = DeferredSender::new(self.reconciler.clone());
        let Some(amount) = self.state.lock().amount_sats else {
            let msg = Message::SetAlert(SendFlowError::InvalidNumber.into());
            if display_alert {
                sender.queue(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }

            return false;
        };

        if amount == 0 {
            let msg = Message::SetAlert(SendFlowError::ZeroAmount.into());
            if display_alert {
                sender.queue(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        if amount < MIN_SEND_SATS {
            let msg = Message::SetAlert(SendFlowError::SendAmountToLow.into());
            if display_alert {
                sender.queue(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        let (spendable_balance, unavailable_balance_alert, is_max_selected) = {
            let state = self.state.lock();
            (
                validation::spendable_balance_for_validation(state.unlocked_spendable_sats),
                validation::unavailable_spendable_balance_alert(
                    state.unlocked_spendable_sats,
                    state.lock_state_load_failed,
                ),
                state.max_selected.is_some(),
            )
        };

        if spendable_balance < amount {
            if let Some(alert) = unavailable_balance_alert {
                let msg = Message::SetAlert(alert);
                if display_alert {
                    sender.queue(msg);
                } else {
                    debug!("validate_amount_failed: {msg:?}");
                }

                return false;
            }

            if is_max_selected {
                let me = self.clone();
                cove_tokio::task::spawn(async move { me.select_max_send_report_error().await });
                return false;
            }

            let msg = Message::SetAlert(SendFlowError::InsufficientFunds.into());
            if display_alert {
                sender.queue(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        true
    }
}

#[uniffi::export]
impl RustSendFlowManager {
    #[uniffi::method]
    fn sanitize_btc_entering_amount(
        self: &Arc<Self>,
        old_value: &str,
        new_value: &str,
    ) -> Option<String> {
        let on_change_handler = BtcOnChangeHandler::new(self.state.clone());
        let changeset = on_change_handler.on_change(old_value, new_value);
        let entering_amount_btc = changeset.entering_amount_btc?;

        if entering_amount_btc == new_value {
            return None;
        }

        Some(entering_amount_btc)
    }

    #[uniffi::method]
    fn sanitize_fiat_entering_amount(
        self: &Arc<Self>,
        old_value: &str,
        new_value: &str,
    ) -> Option<String> {
        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;
        let max_selected = self.state.lock().max_selected.as_deref().copied();

        let handler = FiatOnChangeHandler::new(prices, selected_currency, max_selected);
        let changed = handler.on_change(old_value, new_value).ok()?.entering_fiat_amount?;

        if changed == new_value {
            return None;
        }

        Some(changed)
    }

    // MARK: Action handler
    /// action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(self: Arc<Self>, action: Action) {
        debug!("dispatch: {action:?}");

        match action {
            Action::NotifyEnteringBtcAmountChanged(string) => {
                let old_value = self.state.lock().entering_btc_amount.clone();
                self.handle_btc_field_changed(old_value, string);
            }

            Action::NotifyEnteringFiatAmountChanged(string) => {
                let old_value = self.state.lock().entering_fiat_amount.clone();
                self.handle_fiat_field_changed(old_value, string);
            }

            Action::NotifyEnteringAddressChanged(address) => {
                self.handle_entering_address_changed(address);
            }

            Action::ChangeSetAmountFocusField(set_amount_focus_field) => {
                self.state.lock().focus_field = set_amount_focus_field;
                self.reconciler.send(Message::UpdateFocusField(set_amount_focus_field));
            }

            Action::SelectFeeRate(fee_rate) => self.selected_fee_rate_changed(fee_rate),

            Action::SelectMaxSend => {
                let me = self.clone();
                cove_tokio::task::spawn(async move { me.select_max_send_report_error().await });
            }

            Action::ClearSendAmount => self.clear_send_amount(),
            Action::ClearAddress => self.clear_address(),
            Action::RefreshWalletBalance => {
                let me = self.clone();
                cove_tokio::task::spawn(async move {
                    me.get_wallet_balance().await;
                    me.get_or_update_fee_rate_options().await;
                    me.reconciler.send(Message::RefreshPresenters);
                });
            }

            Action::NotifySelectedUnitedChanged { old, new } => {
                self.handle_selected_unit_changed(old, new);
            }

            Action::NotifyScanCodeChanged { old, new } => {
                self.handle_scan_code_changed(old, new);
            }

            Action::NotifyBtcOrFiatChanged { old, new } => {
                self.handle_btc_or_fiat_changed(old, new);
            }

            Action::NotifyPricesChanged(prices) => self.handle_prices_changed(prices),

            Action::FinalizeAndGoToNextScreen => self.finalize_and_go_to_next_screen(),

            Action::NotifyAddressChanged(address) => {
                let mut state = self.state.lock();
                state.address = Some(address.clone());
                state.entering_address = address.to_string();
                state.payjoin_endpoint = None;
            }

            Action::NotifyAmountChanged(amount) => self.handle_amount_changed(*amount),

            Action::NotifyFocusFieldChanged { old, new } => {
                self.handle_focus_field_changed(old, new);
            }

            Action::ChangeFeeRateOptions(fee_options) => {
                let selection = self.fee_selection_for_options(fee_options);
                self.state.lock().fee_selection = Some(selection.clone());
                self.reconciler.send(Message::UpdateFeeSelection(selection));
            }

            Action::ChangeEnteringAddress(string) => {
                self.reconciler.send(Message::UpdateEnteringAddress(string.clone()));
                self.handle_entering_address_changed(string);
            }

            Action::DisableCoinControlMode => self.disable_coin_control_mode(),
            Action::SetCoinControlMode(utxos) => self.set_coin_control_mode(utxos),
            Action::NotifyCoinControlAmountChanged(amount) => {
                self.handle_coin_control_amount_changed(amount);
            }
            Action::NotifyCoinControlEnteredAmountChanged(amount, is_focused) => {
                self.handle_coin_control_entered_amount_changed(amount, is_focused);
            }
        }
    }
}

// MARK: Private getters
impl RustSendFlowManager {
    pub fn send_amount(&self) -> Option<Amount> {
        let amount_sats = self.state.lock().amount_sats?;
        Some(Amount::from_sat(amount_sats))
    }

    pub fn max_send_minus_fees(&self) -> Option<Amount> {
        let max_send = match self.state.lock().mode {
            EnterMode::SetAmount => return None,
            EnterMode::CoinControl(ref mode) => mode.max_send(),
        };

        let total_fee_sats = self
            .selected_fee_rate()
            .as_ref()
            .and_then(|f| f.total_fee.map(|fee| fee.as_sats()))
            .unwrap_or(1000);

        let max_send_without_fees = max_send.as_sats().saturating_sub(total_fee_sats);
        Some(Amount::from_sat(max_send_without_fees))
    }

    pub fn max_send_minus_fees_and_small_utxo(&self) -> Option<Amount> {
        static SMALL_UTXO: u64 = 600;
        let max_send = self.max_send_minus_fees()?;

        let small_utxo_amount = Amount::from_sat(SMALL_UTXO);
        if max_send <= small_utxo_amount {
            return None;
        }

        let amount = max_send - small_utxo_amount;
        Some(amount)
    }
}

/// MARK: State mutating impl
impl RustSendFlowManager {}

/// MARK: helper method impls
impl RustSendFlowManager {
    fn send_alert(self: &Arc<Self>, alert: impl Into<SendFlowAlertState>) {
        self.reconciler.send(Message::SetAlert(alert.into()));
    }

    async fn send_alert_async(self: &Arc<Self>, alert: impl Into<SendFlowAlertState>) {
        self.reconciler.send_async(Message::SetAlert(alert.into())).await;
    }

    fn total_spent_btc_amount(self: &Arc<Self>) -> Option<Amount> {
        let send_amount = self.send_amount()?;
        let total_fee = self.selected_fee_rate()?.total_fee?;
        Some(send_amount + total_fee)
    }

    /// Background refresh tasks that run in parallel
    /// If no cached fees exist, fetches fees first and sets has_base_fees
    fn background_init_tasks(self: &Arc<Self>) {
        let me = self.clone();
        cove_tokio::task::spawn(async move {
            // run all refreshes concurrently
            tokio::join!(
                me.get_first_address(),
                me.get_or_update_fee_rate_options(),
                me.get_wallet_balance(),
            );
        });

        let state = self.state.clone();
        cove_tokio::task::spawn(async move {
            let result = (|| crate::fee_client::get_and_update_fees()).retry(
                ExponentialBuilder::default()
                    .with_min_delay(std::time::Duration::from_millis(100))
                    .with_max_delay(std::time::Duration::from_secs(3))
                    .with_total_delay(Some(std::time::Duration::from_secs(18))),
            );

            if let Err(e) = result.await {
                return error!("failed to fetch fees: error={e:?}");
            }

            let Some(fee_response) = crate::fee_client::FEE_CLIENT.fees() else {
                return;
            };

            let base_options = FeeRateOptions::from(fee_response);
            let fee_options = FeeRateOptionsWithTotalFee::without_totals(base_options);
            let previous_selected =
                state.lock().fee_selection.as_ref().map(|selection| selection.selected.clone());
            let selected = previous_selected
                .and_then(|selected| match selected.fee_speed {
                    FeeSpeed::Fast => Some(Arc::new(fee_options.fast)),
                    FeeSpeed::Medium => Some(Arc::new(fee_options.medium)),
                    FeeSpeed::Slow => Some(Arc::new(fee_options.slow)),
                    FeeSpeed::Custom { .. } => {
                        fee_options.get_fee_rate_with(selected.fee_rate.sat_per_vb())
                    }
                })
                .unwrap_or_else(|| Arc::new(fee_options.medium));
            let fee_selection = FeeSelection::new(Arc::new(fee_options), selected);

            let mut state_guard = state.lock();
            state_guard.fee_rate_options_base = Some(Arc::new(base_options));
            state_guard.fee_selection = Some(fee_selection);
            state_guard.has_base_fees = true;
        });
    }

    async fn get_first_address(self: &Arc<Self>) {
        if let Ok(first_address) = self.wallet_manager.first_address().await {
            let address = first_address.address.clone().into();
            self.state.lock().first_address = Some(Arc::new(address));
        }
    }

    fn schedule_fee_rate_update(self: &Arc<Self>) {
        let me = self.clone();
        self.fee_check_task.replace(async move {
            me.get_or_update_fee_rate_options().await;
        });
    }

    async fn get_wallet_balance(self: &Arc<Self>) {
        let balance = self.wallet_manager.balance().await;
        let unlocked_spendable_sats =
            self.wallet_manager.unlocked_spendable_balance().await.map(|amount| amount.as_sats());
        if let Err(error) = &unlocked_spendable_sats {
            error!(
                error_id = LOCK_STATE_LOAD_FAILED_ERROR_ID,
                "failed to get unlocked spendable balance: {error}"
            );
        }

        let wallet_balance = Arc::new(balance);
        let mut state = self.state.lock();
        state.wallet_balance = Some(wallet_balance);
        match unlocked_spendable_sats {
            Ok(unlocked_spendable_sats) => {
                state.unlocked_spendable_sats = Some(unlocked_spendable_sats);
                state.lock_state_load_failed = false;
            }
            Err(_) => {
                state.unlocked_spendable_sats = None;
                state.lock_state_load_failed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        manager::{deferred_sender::SingleOrMany, wallet_manager::RustWalletManager},
        wallet::{balance::Balance, metadata::WalletMetadata},
    };
    fn manager_for_validation() -> Arc<super::RustSendFlowManager> {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();

        let (sender, receiver) = flume::bounded(50);
        let balance = Arc::new(Balance::default());
        let state = super::State::new(WalletMetadata::preview_new(), balance);
        let wallet_manager = Arc::new(RustWalletManager::preview_new_wallet());

        Arc::new(super::RustSendFlowManager {
            app: super::App::global().clone(),
            wallet_manager,
            state: state.into_inner(),
            reconciler: super::MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
            fee_check_task: super::DebouncedTask::new(
                "fee_check",
                super::Duration::from_millis(200),
            ),
        })
    }

    #[test]
    fn validate_amount_blocks_send_when_lock_state_load_failed() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = manager_for_validation();
        {
            let mut state = manager.state.lock();
            state.amount_sats = Some(50_000);
            state.unlocked_spendable_sats = None;
            state.lock_state_load_failed = true;
        }

        assert!(!manager.validate_amount(true));

        let message = manager.reconcile_receiver.try_recv().expect("alert is reconciled");
        let SingleOrMany::Single(super::Message::SetAlert(alert)) = message else {
            panic!("expected a single lock-state load failure alert");
        };

        assert!(matches!(
            alert,
            super::SendFlowAlertState::General { title, .. } if title.contains("Locked")
        ));
    }
}
