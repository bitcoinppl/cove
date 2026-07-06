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
use alert_state::{SendFlowAlertState, SendFlowWarningKind};
use amount_or_max::AmountOrMax;
use backon::{ExponentialBuilder, Retryable};
use btc_on_change::BtcOnChangeHandler;
use cove_common::consts::LOW_SEND_WARNING_SATS;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
    unit::BitcoinUnit,
    utxo::Utxo,
};
use error::SendFlowError;
use fiat_on_change::FiatOnChangeHandler;
use parking_lot::Mutex;
use state::{CoinControlMode, EnterMode, FeeSelection, SendFlowManagerState, State};
use tracing::{debug, error, trace};

use super::{
    deferred_sender,
    reconcile_channel::ReconcileChannel,
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
const HIGH_FEE_WARNING_PERCENT: f64 = 5.0;
const VERY_HIGH_FEE_WARNING_PERCENT: f64 = 20.0;

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

    reconciler: ReconcileChannel<Message>,

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
    AcknowledgeWarningAndFinalize(SendFlowWarningKind),
}

impl RustSendFlowManager {
    pub fn new(
        metadata: WalletMetadata,
        balance: Arc<Balance>,
        wallet_manager: Arc<RustWalletManager>,
    ) -> Arc<Self> {
        let state = State::new(metadata, balance);

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
            reconciler: ReconcileChannel::new(50),
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
        self.reconciler.listen_async(move |field| {
            trace!("reconcile_receiver: {field:?}");
            match field {
                SingleOrMany::Single(message) => reconciler.reconcile(message),
                SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
            }
        });
    }

    // MARK: Validators
    #[uniffi::method(default(display_alert = false))]
    pub fn validate_address(self: &Arc<Self>, display_alert: bool) -> bool {
        self.validate_address_internal(display_alert)
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

    pub(crate) fn validate_amount_internal(self: &Arc<Self>, display_alert: bool) -> bool {
        let mut sender = self.reconciler.deferred_sender();
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

    fn pending_send_warning(&self) -> Option<SendFlowAlertState> {
        let (
            amount_sats,
            fee_sats,
            small_amount_acknowledged,
            high_fee_acknowledged,
            very_high_fee_acknowledged,
        ) = {
            let state = self.state.lock();
            let amount_sats = state.amount_sats?;
            let fee_sats = state
                .fee_selection
                .as_ref()
                .and_then(|selection| selection.selected.total_fee)
                .map(|fee| fee.as_sats());

            (
                amount_sats,
                fee_sats,
                state.has_acknowledged_warning(SendFlowWarningKind::SmallAmount),
                state.has_acknowledged_warning(SendFlowWarningKind::HighFee),
                state.has_acknowledged_warning(SendFlowWarningKind::VeryHighFee),
            )
        };

        if amount_sats > 0 && amount_sats < LOW_SEND_WARNING_SATS && !small_amount_acknowledged {
            return Some(SendFlowAlertState::Warning {
                kind: SendFlowWarningKind::SmallAmount,
                title: "Small On-chain Payment".to_string(),
                message: "On-chain payments always pay a network fee, so they are usually best for larger amounts. If you are making lots of small payments, Lightning may be a better fit outside Cove because it is built for fast, low-fee microtransactions.".to_string(),
            });
        }

        let fee_sats = fee_sats?;
        if amount_sats == 0 {
            return None;
        }

        let fee_percentage = fee_sats as f64 / amount_sats as f64 * 100.0;
        let display_fee_percentage = format!("{fee_percentage:.0}");

        if fee_percentage >= VERY_HIGH_FEE_WARNING_PERCENT {
            if !very_high_fee_acknowledged {
                return Some(SendFlowAlertState::Warning {
                    kind: SendFlowWarningKind::VeryHighFee,
                    title: "Very High Network Fee".to_string(),
                    message: format!(
                        "The network fee is {display_fee_percentage}% of the amount you are sending. That is unusually high for an on-chain payment. Consider sending a larger amount, lowering the fee rate if timing allows, or using an external Lightning option for small payments."
                    ),
                });
            }

            return None;
        }

        if fee_percentage >= HIGH_FEE_WARNING_PERCENT && !high_fee_acknowledged {
            return Some(SendFlowAlertState::Warning {
                kind: SendFlowWarningKind::HighFee,
                title: "High Network Fee".to_string(),
                message: format!(
                    "The network fee is {display_fee_percentage}% of the amount you are sending. On-chain fees are per transaction, so they take a bigger bite out of small payments. Consider a larger amount or an external Lightning option."
                ),
            });
        }

        None
    }

    fn acknowledge_warning_and_finalize(self: &Arc<Self>, kind: SendFlowWarningKind) {
        if let Some(SendFlowAlertState::Warning { kind: pending_kind, .. }) =
            self.pending_send_warning()
            && pending_kind == kind
        {
            self.state.lock().acknowledge_warning(kind);
        }

        self.reconciler.send(Message::ClearAlert);
        self.finalize_and_go_to_next_screen();
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
            Action::AcknowledgeWarningAndFinalize(kind) => {
                self.acknowledge_warning_and_finalize(kind);
            }

            Action::NotifyAddressChanged(address) => {
                let mut state = self.state.lock();
                if state.address.as_ref() != Some(&address) {
                    state.clear_warning_acknowledgements();
                }
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
                {
                    let mut state = self.state.lock();
                    if state.fee_selection.as_ref() != Some(&selection) {
                        state.clear_warning_acknowledgements();
                    }
                    state.fee_selection = Some(selection.clone());
                }
                self.reconcile_coin_control_amount_for_selected_fee();
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
            if state_guard.fee_selection.as_ref() != Some(&fee_selection) {
                state_guard.clear_warning_acknowledgements();
            }
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
    use std::{sync::Arc, time::Duration};

    use cove_types::fees::{
        FeeRateOption, FeeRateOptionWithTotalFee, FeeRateOptionsWithTotalFee, FeeSpeed,
    };
    use cove_types::utxo::{UtxoList, ffi_preview::preview_new_utxo_list};

    use crate::{
        manager::{deferred_sender::SingleOrMany, wallet_manager::RustWalletManager},
        wallet::{Address, balance::Balance, metadata::WalletMetadata},
    };

    fn manager_for_validation() -> Arc<super::RustSendFlowManager> {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();

        let balance = Arc::new(Balance::default());
        let state = super::State::new(WalletMetadata::preview_new(), balance);
        let wallet_manager = Arc::new(RustWalletManager::preview_new_wallet());

        Arc::new(super::RustSendFlowManager {
            app: super::App::global().clone(),
            wallet_manager,
            state: state.into_inner(),
            reconciler: super::ReconcileChannel::new(50),
            fee_check_task: super::DebouncedTask::new(
                "fee_check",
                super::Duration::from_millis(200),
            ),
        })
    }

    fn fee_rate_option_with_total_fee(
        fee_speed: FeeSpeed,
        total_fee_sats: u64,
    ) -> FeeRateOptionWithTotalFee {
        let fee_option = FeeRateOption::new(fee_speed, 1.0);
        FeeRateOptionWithTotalFee::new(fee_option, super::Amount::from_sat(total_fee_sats))
    }

    fn set_selected_fee_total(
        manager: &super::RustSendFlowManager,
        total_fee_sats: u64,
    ) -> Arc<FeeRateOptionWithTotalFee> {
        let selected =
            fee_rate_option_with_total_fee(FeeSpeed::Custom { duration_mins: 10 }, total_fee_sats);
        let options = FeeRateOptionsWithTotalFee {
            fast: fee_rate_option_with_total_fee(FeeSpeed::Fast, total_fee_sats),
            medium: fee_rate_option_with_total_fee(FeeSpeed::Medium, total_fee_sats),
            slow: fee_rate_option_with_total_fee(FeeSpeed::Slow, total_fee_sats),
            custom: Some(selected),
        };
        let selected = Arc::new(selected);

        manager.state.lock().fee_selection =
            Some(super::FeeSelection::new(Arc::new(options), selected.clone()));

        selected
    }

    fn set_selected_fee_without_total(manager: &super::RustSendFlowManager) {
        let base_options = super::FeeRateOptions::_ffi_preview_new();
        let selected = FeeRateOptionWithTotalFee {
            fee_speed: FeeSpeed::Custom { duration_mins: 10 },
            fee_rate: FeeRateOption::new(FeeSpeed::Custom { duration_mins: 10 }, 1.0).fee_rate,
            total_fee: None,
        };
        let options = FeeRateOptionsWithTotalFee::without_totals(base_options);

        let mut state = manager.state.lock();
        state.fee_rate_options_base = Some(Arc::new(base_options));
        state.fee_selection = Some(super::FeeSelection::new(Arc::new(options), Arc::new(selected)));
    }

    fn set_valid_amount_and_address(manager: &super::RustSendFlowManager, amount_sats: u64) {
        let mut state = manager.state.lock();
        state.amount_sats = Some(amount_sats);
        state.unlocked_spendable_sats = Some(50_000);
        state.address = Some(Arc::new(Address::preview_new()));
    }

    fn set_coin_control_mode_with_total(manager: &super::RustSendFlowManager, total_sats: u64) {
        let mut utxos = preview_new_utxo_list(1, 0);
        utxos[0].amount = Arc::new(super::Amount::from_sat(total_sats));
        let utxo_list = Arc::new(UtxoList::from(utxos));

        let mut state = manager.state.lock();
        state.metadata.selected_unit = super::BitcoinUnit::Sat;
        state.mode = super::EnterMode::coin_control_max(utxo_list);
    }

    fn set_coin_control_mode(manager: &super::RustSendFlowManager) {
        set_coin_control_mode_with_total(manager, 10_000);
    }

    fn next_reconcile_message(manager: &super::RustSendFlowManager) -> super::Message {
        let message = manager.reconciler.receiver().try_recv().expect("message is reconciled");
        let SingleOrMany::Single(message) = message else {
            panic!("expected a single reconcile message");
        };

        message
    }

    fn drain_reconcile_messages(manager: &super::RustSendFlowManager) -> Vec<super::Message> {
        let mut messages = Vec::new();
        let receiver = manager.reconciler.receiver();

        while let Ok(message) = receiver.try_recv() {
            match message {
                SingleOrMany::Single(message) => messages.push(message),
                SingleOrMany::Many(batch) => messages.extend(batch),
            }
        }

        messages
    }

    fn pending_warning_kind(
        manager: &super::RustSendFlowManager,
    ) -> Option<super::SendFlowWarningKind> {
        match manager.pending_send_warning()? {
            super::SendFlowAlertState::Warning { kind, .. } => Some(kind),
            _ => None,
        }
    }

    fn warning_message_kind(message: &super::Message) -> Option<super::SendFlowWarningKind> {
        match message {
            super::Message::SetAlert(super::SendFlowAlertState::Warning { kind, .. }) => {
                Some(*kind)
            }
            _ => None,
        }
    }

    #[test]
    fn validate_amount_allows_low_nonzero_amount() {
        let manager = manager_for_validation();
        {
            let mut state = manager.state.lock();
            state.amount_sats = Some(1_000);
            state.unlocked_spendable_sats = Some(50_000);
        }

        assert!(manager.validate_amount(false));
    }

    #[test]
    fn coin_control_amount_change_preserves_below_conservative_dust_amount() {
        let manager = manager_for_validation();
        set_coin_control_mode(&manager);

        assert!(manager.handle_coin_control_amount_changed(300.0).is_some());

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(300));
        assert!(
            matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
        );
    }

    #[test]
    fn focused_coin_control_entry_preserves_below_conservative_dust_amount() {
        let manager = manager_for_validation();
        set_coin_control_mode(&manager);

        assert!(
            manager.handle_coin_control_entered_amount_changed("300".to_string(), true).is_some()
        );

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(300));
        assert!(
            matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
        );
    }

    #[test]
    fn coin_control_amount_change_preserves_amount_when_snap_threshold_unavailable() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 1_500);

        assert!(manager.handle_coin_control_amount_changed(300.0).is_some());

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(300));
        assert!(
            matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
        );
    }

    #[test]
    fn coin_control_amount_change_preserves_amount_when_fallback_max_is_zero() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 900);

        assert!(manager.handle_coin_control_amount_changed(600.0).is_some());

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(600));
        assert!(
            matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
        );
    }

    #[test]
    fn coin_control_amount_change_caps_to_selected_total_when_fee_max_unavailable() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 900);

        assert!(manager.handle_coin_control_amount_changed(1_546.0).is_some());

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(900));
        assert!(
            matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
        );
    }

    #[test]
    fn coin_control_fee_update_caps_preserved_amount_to_real_max() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 900);
        set_selected_fee_without_total(&manager);

        assert!(manager.handle_coin_control_amount_changed(850.0).is_some());

        let selected = fee_rate_option_with_total_fee(FeeSpeed::Custom { duration_mins: 20 }, 100);
        manager.selected_fee_rate_changed(Arc::new(selected));

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(800));
        assert!(matches!(&state.mode, super::EnterMode::CoinControl(mode) if mode.is_max_selected));
    }

    #[test]
    fn coin_control_fee_update_preserves_amount_when_real_max_is_zero() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 900);
        set_selected_fee_without_total(&manager);
        {
            let mut state = manager.state.lock();
            state.address = Some(Arc::new(Address::preview_new()));
            state.unlocked_spendable_sats = Some(50_000);
        }

        assert!(manager.handle_coin_control_amount_changed(850.0).is_some());

        let selected =
            fee_rate_option_with_total_fee(FeeSpeed::Custom { duration_mins: 20 }, 1_000);
        manager.selected_fee_rate_changed(Arc::new(selected));

        {
            let state = manager.state.lock();
            assert_eq!(state.amount_sats, Some(850));
            assert!(
                matches!(&state.mode, super::EnterMode::CoinControl(mode) if !mode.is_max_selected)
            );
        }

        drain_reconcile_messages(&manager);

        manager.finalize_and_go_to_next_screen();

        assert!(matches!(
            next_reconcile_message(&manager),
            super::Message::SetAlert(super::SendFlowAlertState::General { title, .. })
                if title == "Fee Too High!"
        ));
    }

    #[test]
    fn coin_control_fee_update_blocks_when_fee_equals_selected_total() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 900);
        set_selected_fee_without_total(&manager);
        {
            let mut state = manager.state.lock();
            state.address = Some(Arc::new(Address::preview_new()));
            state.unlocked_spendable_sats = Some(50_000);
        }

        assert!(manager.handle_coin_control_amount_changed(900.0).is_some());

        let selected = fee_rate_option_with_total_fee(FeeSpeed::Custom { duration_mins: 20 }, 900);
        manager.selected_fee_rate_changed(Arc::new(selected));
        drain_reconcile_messages(&manager);

        manager.finalize_and_go_to_next_screen();

        assert!(matches!(
            next_reconcile_message(&manager),
            super::Message::SetAlert(super::SendFlowAlertState::General { title, .. })
                if title == "Fee Too High!"
        ));
    }

    #[test]
    fn coin_control_amount_change_snaps_to_max_when_fee_adjusted_max_exceeded() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 1_500);

        assert!(manager.handle_coin_control_amount_changed(600.0).is_some());

        let state = manager.state.lock();
        assert_eq!(state.amount_sats, Some(500));
        assert!(matches!(&state.mode, super::EnterMode::CoinControl(mode) if mode.is_max_selected));
    }

    #[test]
    fn coin_control_max_snap_blocks_when_fee_exceeds_recipient_amount() {
        let manager = manager_for_validation();
        set_coin_control_mode_with_total(&manager, 1_500);
        set_selected_fee_total(&manager, 1_000);
        {
            let mut state = manager.state.lock();
            state.address = Some(Arc::new(Address::preview_new()));
            state.unlocked_spendable_sats = Some(50_000);
        }

        assert!(manager.handle_coin_control_amount_changed(600.0).is_some());
        drain_reconcile_messages(&manager);

        manager.finalize_and_go_to_next_screen();

        assert!(matches!(
            next_reconcile_message(&manager),
            super::Message::SetAlert(super::SendFlowAlertState::General { title, .. })
                if title == "Fee Too High!"
        ));
    }

    #[test]
    fn pending_warning_returns_small_amount_before_high_fee() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 60);

        assert_eq!(pending_warning_kind(&manager), Some(super::SendFlowWarningKind::SmallAmount));
    }

    #[test]
    fn pending_warning_returns_very_high_fee_for_20_percent_or_more() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 10_000);
        set_selected_fee_total(&manager, 2_000);

        assert_eq!(pending_warning_kind(&manager), Some(super::SendFlowWarningKind::VeryHighFee));
    }

    #[test]
    fn pending_warning_returns_high_fee_for_5_to_20_percent() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 10_000);
        set_selected_fee_total(&manager, 500);

        assert_eq!(pending_warning_kind(&manager), Some(super::SendFlowWarningKind::HighFee));
    }

    #[test]
    fn acknowledging_very_high_fee_does_not_reveal_high_fee_for_same_fee() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 10_000);
        set_selected_fee_total(&manager, 2_000);

        assert_eq!(pending_warning_kind(&manager), Some(super::SendFlowWarningKind::VeryHighFee));

        manager.state.lock().acknowledge_warning(super::SendFlowWarningKind::VeryHighFee);

        assert_eq!(pending_warning_kind(&manager), None);
    }

    #[test]
    fn acknowledging_small_amount_through_finalize_reveals_high_fee() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 60);

        manager.finalize_and_go_to_next_screen();

        let message = next_reconcile_message(&manager);
        assert_eq!(warning_message_kind(&message), Some(super::SendFlowWarningKind::SmallAmount));

        manager.clone().dispatch(super::Action::AcknowledgeWarningAndFinalize(
            super::SendFlowWarningKind::SmallAmount,
        ));

        let messages = drain_reconcile_messages(&manager);
        assert!(messages.contains(&super::Message::ClearAlert));
        assert!(
            messages.iter().any(|message| warning_message_kind(message)
                == Some(super::SendFlowWarningKind::HighFee))
        );
    }

    #[test]
    fn acknowledging_final_warning_through_finalize_proceeds_past_warning_gate() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 60);

        manager.finalize_and_go_to_next_screen();
        assert_eq!(
            warning_message_kind(&next_reconcile_message(&manager)),
            Some(super::SendFlowWarningKind::SmallAmount)
        );

        manager.clone().dispatch(super::Action::AcknowledgeWarningAndFinalize(
            super::SendFlowWarningKind::SmallAmount,
        ));
        let messages = drain_reconcile_messages(&manager);
        assert!(
            messages.iter().any(|message| warning_message_kind(message)
                == Some(super::SendFlowWarningKind::HighFee))
        );

        manager.clone().dispatch(super::Action::AcknowledgeWarningAndFinalize(
            super::SendFlowWarningKind::HighFee,
        ));

        let messages = drain_reconcile_messages(&manager);
        let focus_update_index = messages
            .iter()
            .position(|message| matches!(message, super::Message::UpdateFocusField(None)));
        let alert_index =
            messages.iter().position(|message| matches!(message, super::Message::SetAlert(_)));

        assert!(messages.contains(&super::Message::ClearAlert));
        assert!(focus_update_index.is_some());
        assert!(alert_index.is_none_or(|alert_index| {
            focus_update_index.is_some_and(|focus_update_index| focus_update_index < alert_index)
        }));
    }

    #[test]
    fn acknowledged_small_amount_rearms_when_amount_changes_through_dispatch() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 60);

        manager.finalize_and_go_to_next_screen();
        assert_eq!(
            warning_message_kind(&next_reconcile_message(&manager)),
            Some(super::SendFlowWarningKind::SmallAmount)
        );

        manager.clone().dispatch(super::Action::AcknowledgeWarningAndFinalize(
            super::SendFlowWarningKind::SmallAmount,
        ));
        assert!(drain_reconcile_messages(&manager).iter().any(|message| warning_message_kind(
            message
        ) == Some(
            super::SendFlowWarningKind::HighFee
        )));

        manager
            .clone()
            .dispatch(super::Action::NotifyAmountChanged(Arc::new(super::Amount::from_sat(2_000))));

        assert_eq!(pending_warning_kind(&manager), Some(super::SendFlowWarningKind::SmallAmount));
    }

    #[test]
    fn stale_warning_acknowledgement_does_not_record() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 60);

        manager.acknowledge_warning_and_finalize(super::SendFlowWarningKind::HighFee);

        let state = manager.state.lock();
        assert!(!state.has_acknowledged_warning(super::SendFlowWarningKind::SmallAmount));
        assert!(!state.has_acknowledged_warning(super::SendFlowWarningKind::HighFee));
    }

    #[test]
    fn warning_acknowledgements_reset_on_amount_change() {
        let manager = manager_for_validation();
        manager.state.lock().acknowledge_warning(super::SendFlowWarningKind::SmallAmount);

        manager.handle_amount_changed(super::Amount::from_sat(1_000));

        assert!(
            !manager.state.lock().has_acknowledged_warning(super::SendFlowWarningKind::SmallAmount)
        );
    }

    #[test]
    fn warning_acknowledgements_reset_on_address_change() {
        let manager = manager_for_validation();
        manager.state.lock().acknowledge_warning(super::SendFlowWarningKind::SmallAmount);

        manager
            .clone()
            .dispatch(super::Action::NotifyAddressChanged(Arc::new(Address::preview_new())));

        assert!(
            !manager.state.lock().has_acknowledged_warning(super::SendFlowWarningKind::SmallAmount)
        );
    }

    #[test]
    fn warning_acknowledgements_reset_on_fee_change() {
        let manager = manager_for_validation();
        set_selected_fee_total(&manager, 500);
        manager.state.lock().acknowledge_warning(super::SendFlowWarningKind::HighFee);

        let selected = fee_rate_option_with_total_fee(FeeSpeed::Custom { duration_mins: 20 }, 750);
        manager.selected_fee_rate_changed(Arc::new(selected));

        assert!(
            !manager.state.lock().has_acknowledged_warning(super::SendFlowWarningKind::HighFee)
        );
    }

    #[test]
    fn fee_greater_than_amount_hard_blocks_before_warnings() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_total(&manager, 1_001);

        manager.finalize_and_go_to_next_screen();

        assert!(matches!(
            next_reconcile_message(&manager),
            super::Message::SetAlert(super::SendFlowAlertState::General { title, .. })
                if title == "Fee Too High!"
        ));
    }

    #[test]
    fn finalize_refreshes_missing_total_fee_before_warning_detection() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1_000);
        set_selected_fee_without_total(&manager);

        manager.finalize_and_go_to_next_screen();

        assert!(manager.reconciler.receiver().try_recv().is_err());
    }

    #[test]
    fn finalize_reports_dust_when_missing_total_fee_refresh_cannot_build_fee_totals() {
        let manager = manager_for_validation();
        set_valid_amount_and_address(&manager, 1);
        set_selected_fee_without_total(&manager);

        manager.finalize_and_go_to_next_screen();

        let message = manager
            .reconciler
            .receiver()
            .recv_timeout(Duration::from_secs(2))
            .expect("dust alert is reconciled");

        assert!(matches!(
            message,
            SingleOrMany::Single(super::Message::SetAlert(super::SendFlowAlertState::Error(
                super::SendFlowError::SendBelowDustLimit
            )))
        ));
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

        let super::Message::SetAlert(alert) = next_reconcile_message(&manager) else {
            panic!("expected a single lock-state load failure alert");
        };

        assert!(matches!(
            alert,
            super::SendFlowAlertState::General { title, .. } if title.contains("Locked")
        ));
    }
}
