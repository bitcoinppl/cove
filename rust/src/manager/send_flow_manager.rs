pub mod alert_state;
pub mod btc_on_change;
pub mod error;
pub mod fiat_on_change;
mod sanitize;
pub mod state;

use std::sync::Arc;

use crate::{
    app::{App, AppAction, FfiApp},
    fee_client::FEE_CLIENT,
    fiat::client::PriceResponse,
    router::RouteFactory,
    task,
    transaction::FeeRate,
    wallet::{
        Address,
        metadata::{FiatOrBtc, WalletMetadata, WalletType},
    },
};
use act_zero::{WeakAddr, call};
use alert_state::SendFlowAlertState;
use btc_on_change::BtcOnChangeHandler;
use cove_types::{
    address::AddressWithNetwork,
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
    psbt::Psbt,
    unit::Unit,
};
use cove_util::format::NumberFormatter as _;
use error::SendFlowError;
use fiat_on_change::FiatOnChangeHandler;
use flume::{Receiver, Sender, TrySendError};
use parking_lot::Mutex;
use state::{SendFlowManagerState, State};
use tracing::{debug, error, trace, warn};

use super::wallet::{RustWalletManager, actor::WalletActor};

pub type Error = error::SendFlowError;
type Result<T, E = Error> = std::result::Result<T, E>;

type Action = SendFlowManagerAction;
type Message = SendFlowManagerReconcileMessage;
type Reconciler = dyn SendFlowManagerReconciler;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SetAmountFocusField {
    Amount,
    Address,
}

#[uniffi::export(callback_interface)]
pub trait SendFlowManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
}

#[derive(Debug, uniffi::Object)]

pub struct RustSendFlowManager {
    app: App,

    wallet_manager: Arc<RustWalletManager>,
    pub state: Arc<Mutex<SendFlowManagerState>>,

    reconciler: Sender<Message>,
    reconcile_receiver: Arc<Receiver<Message>>,
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

    UpdateSelectedFeeRate(Arc<FeeRateOptionWithTotalFee>),
    UpdateFeeRateOptions(Arc<FeeRateOptionsWithTotalFee>),

    // side effects
    SetAlert(SendFlowAlertState),
    ClearAlert,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerAction {
    ChangeEnteringAddress(String),

    ChangeSetAmountFocusField(Option<SetAmountFocusField>),

    SelectMaxSend,
    ClearSendAmount,
    ClearAddress,

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),

    // front end changing text fields
    NotifyEnteringBtcAmountChanged(String),
    NotifyEnteringFiatAmountChanged(String),
    NotifyEnteringAddressChanged(String),

    // front end lets the one of the values were changed
    NotifySelectedUnitedChanged { old: Unit, new: Unit },
    NotifyBtcOrFiatChanged { old: FiatOrBtc, new: FiatOrBtc },
    NotifyScanCodeChanged { old: String, new: String },
    NotifyPricesChanged(Arc<PriceResponse>),
    NotifyFocusFieldChanged { old: Option<SetAmountFocusField>, new: Option<SetAmountFocusField> },

    // starting with an amount and address from scan
    NotifyAddressChanged(Arc<Address>),
    NotifyAmountChanged(Arc<Amount>),

    // custom fee selection
    ChangeFeeRateOptions(Arc<FeeRateOptionsWithTotalFee>),

    FinalizeAndGoToNextScreen,
}

impl RustSendFlowManager {
    pub fn new(metadata: WalletMetadata, wallet_manager: Arc<RustWalletManager>) -> Arc<Self> {
        let (sender, receiver) = flume::bounded(50);

        let state = State::new(metadata);

        let me: Arc<Self> = Self {
            app: App::global().clone(),
            state: state.into_inner(),
            wallet_manager,
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
        .into();

        // in background run init tasks and setup
        me.background_init_tasks();
        me
    }

    fn wallet_actor(&self) -> WeakAddr<WalletActor> {
        self.wallet_manager.actor.downgrade()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RustSendFlowManager {
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        task::spawn(async move {
            while let Ok(field) = reconcile_receiver.recv_async().await {
                trace!("reconcile_receiver: {field:?}");
                // call the reconcile method on the frontend
                reconciler.reconcile(field);
            }
        });
    }

    // MARK: read only methods
    #[uniffi::method]
    pub fn amount(&self) -> Arc<Amount> {
        Arc::new(Amount::from_sat(self.amount_sats()))
    }

    #[uniffi::method]
    pub fn entering_fiat_amount(&self) -> String {
        self.state.lock().entering_fiat_amount.clone()
    }

    #[uniffi::method]
    pub async fn wait_for_init(&self) {
        let mut times = 0;
        loop {
            if self.state.lock().init_complete {
                break;
            }

            debug!("waiting for init {times}");
            let wait_time = (33 + times * 10).min(200);
            tokio::time::sleep(std::time::Duration::from_millis(wait_time)).await;
            times += 1;
        }
    }

    #[uniffi::method]
    pub fn amount_sats(&self) -> u64 {
        self.state.lock().amount_sats.unwrap_or(0)
    }

    #[uniffi::method]
    pub fn send_amount_btc(&self) -> String {
        let amount_sats = self.amount_sats();
        let send_amount = Amount::from_sat(amount_sats);
        match self.state.lock().metadata.selected_unit {
            Unit::Btc => {
                let string = send_amount.as_btc().thousands();
                if string.contains("e") { send_amount.btc_string() } else { string.to_string() }
            }
            Unit::Sat => send_amount.as_sats().thousands_int().to_string(),
        }
    }

    #[uniffi::method]
    pub fn send_amount_fiat(&self) -> String {
        let Some(btc_price_in_fiat) = self.state.lock().btc_price_in_fiat else {
            return "---".to_string();
        };

        let amount_sats = self.amount_sats();
        let send_amount_in_fiat = self.state.lock().amount_fiat.unwrap_or_else(|| {
            let send_amount = Amount::from_sat(amount_sats);
            send_amount.as_btc().ceil() * (btc_price_in_fiat as f64)
        });

        self.display_fiat_amount(send_amount_in_fiat, true).to_string()
    }

    #[uniffi::method]
    pub fn total_spent_in_btc(self: &Arc<Self>) -> String {
        let Some(amount_sats) = self.state.lock().amount_sats else {
            return "---".to_string();
        };

        let Some(total_spent) = self.total_spent_btc_amount(amount_sats) else {
            return "---".to_string();
        };

        match self.state.lock().metadata.selected_unit {
            Unit::Btc => format!("{} BTC", total_spent.as_btc().thousands()),
            Unit::Sat => format!("{} sats", total_spent.as_sats().thousands_int()),
        }
    }

    #[uniffi::method]
    pub fn total_spent_in_fiat(self: &Arc<Self>) -> String {
        let Some(amount_sats) = self.state.lock().amount_sats else {
            return "---".to_string();
        };

        let Some(total_spent) = self.total_spent_btc_amount(amount_sats) else {
            return "---".to_string();
        };

        let Some(btc_price_in_fiat) = self.state.lock().btc_price_in_fiat else {
            return "---".to_string();
        };

        let total_spent_in_fiat = total_spent.as_btc() * (btc_price_in_fiat as f64);
        format!("≈ {}", self.display_fiat_amount(total_spent_in_fiat, true))
    }

    #[uniffi::method]
    pub fn total_fee_string(&self) -> String {
        let Some(selected_fee_rate) = &self.state.lock().selected_fee_rate.clone() else {
            return "---".to_string();
        };

        let total_fee = selected_fee_rate.total_fee();
        match self.state.lock().metadata.selected_unit {
            Unit::Btc => format!("{} BTC", total_fee.as_btc().thousands()),
            Unit::Sat => format!("{} sats", total_fee.as_sats().thousands_int()),
        }
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount(&self, amount: f64, with_suffix: bool) -> String {
        {
            let sensitive_visible = self.state.lock().metadata.sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let fiat = amount.thousands_fiat();
        let currency = self.state.lock().selected_fiat_currency;

        let symbol = currency.symbol();
        let suffix = currency.suffix();

        if with_suffix && !suffix.is_empty() {
            return format!("{symbol}{fiat} {suffix}");
        }

        format!("{symbol}{fiat}")
    }

    // MARK: Validators
    #[uniffi::method(default(display_alert = false))]
    pub fn validate_address(self: &Arc<Self>, display_alert: bool) -> bool {
        if self.state.lock().address.is_none() {
            if display_alert {
                let error =
                    SendFlowError::InvalidAddress(self.state.lock().entering_address.clone());
                self.send(Message::SetAlert(error.into()));
            }

            return false;
        }

        true
    }

    #[uniffi::method(default(display_alert = false))]
    pub fn validate_amount(self: &Arc<Self>, display_alert: bool) -> bool {
        let mut sender = DeferredSender::new(self.clone());
        let Some(amount) = self.state.lock().amount_sats else {
            let msg = Message::SetAlert(SendFlowError::InvalidNumber.into());
            if display_alert {
                sender.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }

            return false;
        };

        if amount == 0 {
            let msg = Message::SetAlert(SendFlowError::ZeroAmount.into());
            if display_alert {
                sender.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            };
            return false;
        }

        if amount < 5000 {
            let msg = Message::SetAlert(SendFlowError::SendAmountToLow.into());
            if display_alert {
                sender.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        let spendable_balance = self
            .state
            .lock()
            .wallet_balance
            .clone()
            .unwrap_or_default()
            .trusted_spendable()
            .to_sat();

        if spendable_balance < amount {
            let is_max_selected = self.state.lock().max_selected.is_some();
            if is_max_selected {
                let me = self.clone();
                task::spawn(async move { me.select_max_send_report_error().await });
                return false;
            }

            let msg = Message::SetAlert(SendFlowError::InsufficientFunds.into());
            if display_alert {
                sender.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        true
    }

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
        };

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
                self.send(Message::UpdateFocusField(set_amount_focus_field));
            }

            Action::SelectFeeRate(fee_rate) => {
                self.selected_fee_rate_changed(fee_rate);
            }

            Action::SelectMaxSend => {
                let me = self.clone();
                task::spawn(async move { me.select_max_send_report_error().await });
            }

            Action::ClearSendAmount => self.clear_send_amount(),
            Action::ClearAddress => self.clear_address(),

            Action::NotifySelectedUnitedChanged { old, new } => {
                self.handle_selected_unit_changed(old, new)
            }

            Action::NotifyScanCodeChanged { old, new } => {
                self.handle_scan_code_changed(old, new);
            }

            Action::NotifyBtcOrFiatChanged { old, new } => {
                self.handle_btc_or_fiat_changed(old, new);
            }

            Action::NotifyPricesChanged(prices) => {
                self.handle_prices_changed(prices);
            }

            Action::FinalizeAndGoToNextScreen => {
                self.finalize_and_go_to_next_screen();
            }

            Action::NotifyAddressChanged(address) => {
                let mut state = self.state.lock();
                state.address = Some(address.clone());
                state.entering_address = address.to_string();
            }

            Action::NotifyAmountChanged(amount) => {
                self.handle_amount_changed(*amount);
            }

            Action::NotifyFocusFieldChanged { old, new } => {
                self.handle_focus_field_changed(old, new);
            }

            Action::ChangeFeeRateOptions(fee_options) => {
                self.state.lock().fee_rate_options = Some(fee_options.clone());
                self.send(Message::UpdateFeeRateOptions(fee_options));
            }

            Action::ChangeEnteringAddress(string) => {
                self.send(Message::UpdateEnteringAddress(string.clone()));
                self.handle_entering_address_changed(string);
            }
        }
    }
}

/// MARK: State mutating impl
impl RustSendFlowManager {
    fn handle_btc_field_changed(self: Arc<Self>, old: String, new: String) -> Option<()> {
        trace!("btc_field_changed {old} --> {new}");
        if old == new {
            return None;
        }

        // update the state
        let mut sender = DeferredSender::new(self.clone());
        self.state.lock().entering_btc_amount = new.clone();

        let state: State = self.state.clone().into();
        let me = self.clone();

        let needs_fee_rate_options_base = self.state.lock().fee_rate_options_base.is_none();
        if needs_fee_rate_options_base {
            crate::task::spawn(async move {
                me.get_and_update_base_fee_rate_options().await;
            });
        }

        let handler = BtcOnChangeHandler::new(state.clone());
        let changes = handler.on_change(&old, &new);
        debug!("btc_on_change_handler changes: {changes:?}");

        let btc_on_change::Changeset { entering_amount_btc, max_selected, amount_btc, amount_fiat } =
            changes;

        match max_selected {
            Some(Some(max)) => {
                let max = Arc::new(max);
                self.state.lock().max_selected = Some(max.clone());
                sender.send(Message::SetMaxSelected(max));
            }
            Some(None) => {
                let was_max_selected = self.state.lock().max_selected.take().is_some();
                if was_max_selected {
                    sender.send(Message::UnsetMaxSelected);
                }
            }
            None => {}
        }

        if let Some(amount) = amount_btc {
            let current_amount_sats = self.state.lock().amount_sats;
            let amount_sats = amount.to_sat();
            self.state.lock().amount_sats = Some(amount_sats);

            if current_amount_sats != Some(amount_sats) {
                sender.send(Message::UpdateAmountSats(amount_sats));
                self.sync_wrap_get_or_update_fee_rate_options();
            }
        }

        if let Some(amount) = amount_fiat {
            self.state.lock().amount_fiat = Some(amount);
            sender.send(Message::UpdateAmountFiat(amount));
        }

        if let Some(entering_amount) = entering_amount_btc {
            self.set_and_send_entering_btc_amount(entering_amount, &mut sender);
        }

        Some(())
    }

    fn handle_fiat_field_changed(
        self: &Arc<Self>,
        old_value: String,
        new_value: String,
    ) -> Option<()> {
        debug!("fiat_field_changed {old_value} --> {new_value}");
        if old_value == new_value {
            return None;
        }

        let mut sender = DeferredSender::new(self.clone());

        // update the state
        self.state.lock().entering_fiat_amount = new_value.clone();

        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;
        let max_selected = self.state.lock().max_selected.as_deref().copied();

        let handler = FiatOnChangeHandler::new(prices, selected_currency, max_selected);
        let Ok(result) = handler.on_change(&old_value, &new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        debug!("result: {result:?}, old_value: {old_value}, new_value: {new_value}");
        let fiat_on_change::Changeset {
            entering_fiat_amount,
            fiat_value,
            btc_amount,
            max_selected,
        } = result;

        if let Some(entering_fiat_amount) = entering_fiat_amount {
            self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
            sender.send(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
        }

        if let Some(amount_fiat) = fiat_value {
            self.state.lock().amount_fiat = Some(amount_fiat);
            sender.send(Message::UpdateAmountFiat(amount_fiat));
        }

        if let Some(btc_amount) = btc_amount {
            let btc_amount = btc_amount.as_sats();
            self.state.lock().amount_sats = Some(btc_amount);
            sender.send(Message::UpdateAmountSats(btc_amount));
            self.sync_wrap_get_or_update_fee_rate_options();
        }

        if let Some(None) = max_selected {
            let was_max_selected = self.state.lock().max_selected.take().is_some();
            if was_max_selected {
                sender.send(Message::UnsetMaxSelected);
            }
        }

        Some(())
    }

    fn handle_entering_address_changed(self: &Arc<Self>, address: String) {
        debug!("handle_entering_address_changed: {address}");

        let mut sender = DeferredSender::new(self.clone());

        // update the state
        self.state.lock().entering_address = address.clone();

        // if the address is valid, then set it in the state
        let address = Address::from_string(&address, self.state.lock().metadata.network).ok();
        let address = address.map(Arc::new);
        self.state.lock().address = address.clone();
        sender.send(Message::UpdateAddress(address.clone()));

        // when we have a valid address, use that to get the fee rate options
        let me = self.clone();
        let is_max_selected = self.state.lock().max_selected.is_some();
        task::spawn(async move {
            me.get_or_update_fee_rate_options().await;

            if is_max_selected {
                me.select_max_send_report_error().await;
            }
        });
    }

    fn clear_send_amount(self: &Arc<Self>) {
        {
            let mut state = self.state.lock();
            state.amount_sats = None;
            state.amount_fiat = None;
        }

        let mut sender = DeferredSender::new(self.clone());
        sender.send(Message::UpdateAmountFiat(0.0));
        sender.send(Message::UpdateAmountSats(0));
        self.sync_wrap_get_or_update_fee_rate_options();

        // fiat
        let currency = self.state.lock().selected_fiat_currency;
        let entering_fiat_amount = currency.symbol().to_string();
        self.set_and_send_entering_fiat_amount(entering_fiat_amount, &mut sender);

        // btc
        self.set_and_send_entering_btc_amount(String::new(), &mut sender);

        let was_max_selected = self.state.lock().max_selected.take().is_some();
        if was_max_selected {
            sender.send(Message::UnsetMaxSelected);
        }
    }

    fn clear_address(self: &Arc<Self>) {
        let mut sender = DeferredSender::new(self.clone());
        self.state.lock().address = None;
        sender.send(Message::UpdateAddress(None));

        self.state.lock().entering_address = String::new();
        sender.send(Message::UpdateEnteringAddress(String::new()));
    }

    fn selected_fee_rate_changed(self: &Arc<Self>, fee_rate: Arc<FeeRateOptionWithTotalFee>) {
        let mut sender = DeferredSender::new(self.clone());
        self.state.lock().selected_fee_rate = Some(fee_rate.clone());
        sender.send(Message::UpdateSelectedFeeRate(fee_rate.clone()));

        // max was selected before, so we need to update it to match the new fee rate
        let max_selected = self.state.lock().max_selected.clone();
        if max_selected.is_some() {
            self.clone().dispatch(Action::SelectMaxSend);
        }

        if self.validate_amount(false) && self.validate_address(false) {
            self.state.lock().focus_field = None;
            sender.send(Message::UpdateFocusField(None));
        }
    }

    /// When amount is changed, we will need to update the entering and fiat amounts
    fn handle_amount_changed(self: &Arc<Self>, amount: Amount) {
        debug!("handle_amount_changed: {amount:?}");

        let mut sender = DeferredSender::new(self.clone());
        let (unit, fiat_or_btc, btc_price_in_fiat) = {
            let state = self.state.lock();

            let unit = state.metadata.selected_unit;
            let fiat_or_btc = state.metadata.fiat_or_btc;
            let btc_price_in_fiat = state.btc_price_in_fiat;

            (unit, fiat_or_btc, btc_price_in_fiat)
        };

        match fiat_or_btc {
            FiatOrBtc::Fiat => {
                if let Some(price) = btc_price_in_fiat {
                    let currency = self.state.lock().selected_fiat_currency;
                    let amount_fiat = amount.as_btc() * (price as f64);

                    let enterting_amount_fiat =
                        format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat());

                    self.set_and_send_entering_fiat_amount(enterting_amount_fiat, &mut sender);
                }
            }

            FiatOrBtc::Btc => {
                let amount_string = match unit {
                    Unit::Btc => amount.btc_string(),
                    Unit::Sat => amount.as_sats().thousands_int(),
                };

                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
        }

        let old_amount_sats = self.state.lock().amount_sats;
        let amount_sats = amount.to_sat();
        self.state.lock().amount_sats = Some(amount_sats);

        if old_amount_sats != Some(amount_sats) {
            sender.send(Message::UpdateAmountSats(amount_sats));
            self.sync_wrap_get_or_update_fee_rate_options();
        }

        if let Some(price) = btc_price_in_fiat {
            let amount_fiat = amount.as_btc() * (price as f64);
            self.state.lock().amount_fiat = Some(amount_fiat);
            sender.send(Message::UpdateAmountFiat(amount_fiat));
        }
    }

    fn handle_focus_field_changed(
        self: &Arc<Self>,
        old: Option<SetAmountFocusField>,
        new: Option<SetAmountFocusField>,
    ) {
        debug!("handle_focus_field_changed: {old:?} --> {new:?}");

        let mut sender = DeferredSender::new(self.clone());

        // most likely the first load, so ignore for now let front end handle it
        if old.is_none() && new.is_some() && self.state.lock().focus_field.is_none() {
            return;
        }

        // make sure having no focus field is only possible is address and amount are valid
        if new.is_none() {
            // hacky way of finding out if this is the initial load
            let should_show_error = {
                let state = self.state.lock();
                state.address.is_some()
                    && state.amount_sats.is_some()
                    && state.amount_sats.unwrap_or_default() != 0
            };

            if !self.validate_amount(should_show_error) {
                self.state.lock().focus_field = Some(SetAmountFocusField::Amount);
                sender.send(Message::UpdateFocusField(Some(SetAmountFocusField::Amount)));
                return;
            }

            if !self.validate_address(should_show_error) {
                self.state.lock().focus_field = Some(SetAmountFocusField::Address);
                sender.send(Message::UpdateFocusField(Some(SetAmountFocusField::Address)));
                return;
            }
        }

        // format on blur
        if old == Some(SetAmountFocusField::Amount) {
            let amount = self.state.lock().amount_sats.map(Amount::from_sat);
            let amount_fiat = self.state.lock().amount_fiat;

            if let Some(amount_fiat) = amount_fiat {
                let currency = self.state.lock().selected_fiat_currency;
                let entering_fiat_amount =
                    format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat());

                self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
                sender.send(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
            }

            let unit = self.state.lock().metadata.selected_unit;
            match (amount, unit) {
                (Some(amount), Unit::Sat) => {
                    let entering_btc_amount = amount.as_sats().thousands_int().to_string();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                (Some(amount_sats), Unit::Btc) => {
                    let entering_btc_amount = amount_sats.as_btc().thousands().to_string();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                _ => {}
            }
        };

        self.state.lock().focus_field = new;
        sender.send(Message::UpdateFocusField(new));
    }

    async fn select_max_send_report_error(self: &Arc<Self>) {
        match self.select_max_send().await {
            Ok(_) => {}
            Err(error) => {
                let error = SendFlowError::UnableToGetMaxSend(error.to_string());
                self.send(Message::SetAlert(error.into()));
            }
        }
    }

    async fn select_max_send(self: &Arc<Self>) -> Result<()> {
        debug!("select_max_send");

        let mut sender = DeferredSender::new(self.clone());

        // access the mutex once
        let (address, fee_rate_options, selected_fee_rate, selected_fee_rate_base) = {
            let state = self.state.lock();

            let address = state.address.clone();
            let address_string = &state.entering_address;

            let address = address
                .map(Arc::unwrap_or_clone)
                .or_else(|| Address::from_string(address_string, state.metadata.network).ok())
                .or_else(|| state.first_address.clone().map(Arc::unwrap_or_clone));

            let selected_fee_rate_base = state.fee_rate_options_base.clone();
            let fee_rate_options = state.fee_rate_options.clone();
            let selected_fee_rate = state.selected_fee_rate.clone();
            let address = address.ok_or(Error::InvalidAddress(address_string.to_string()))?;

            (address, fee_rate_options, selected_fee_rate, selected_fee_rate_base)
        };

        if fee_rate_options.is_none() {
            self.get_or_update_fee_rate_options().await;
        }

        let wallet_actor = self.wallet_actor();

        // use the selected fee rate if we have have
        // or the medium base fee rate
        // or a default of 50 sat/vb
        let fee_rate = selected_fee_rate
            .map(|selected| selected.fee_rate)
            .or_else(|| selected_fee_rate_base.map(|base| base.medium.fee_rate));

        if fee_rate.is_none() {
            warn!("unable to get selected fee rate or base fee rate using default of 50 sat/vb");
        }

        let fee_rate = fee_rate.unwrap_or_else(|| FeeRate::from_sat_per_vb(50.0));
        let psbt: Psbt = call!(wallet_actor.build_ephemeral_drain_tx(address, fee_rate))
            .await
            .unwrap()
            .map_err(|error| Error::UnableToGetMaxSend(error.to_string()))?
            .into();

        let total = Arc::new(psbt.output_total_amount());
        trace!("psbt: {psbt:?}, total: {total:?}, fee_rate: {fee_rate:?}");

        self.state.lock().max_selected = Some(total.clone());
        sender.send(Message::SetMaxSelected(total.clone()));
        self.handle_amount_changed(*total);

        let address_is_valid = self.state.lock().address.is_some();
        match address_is_valid {
            true => {
                self.state.lock().focus_field = None;
                sender.send(Message::UpdateFocusField(None))
            }
            false => {
                self.state.lock().focus_field = Some(SetAmountFocusField::Address);
                sender.send(Message::UpdateFocusField(Some(SetAmountFocusField::Address)))
            }
        }

        Ok(())
    }

    async fn get_and_update_base_fee_rate_options(self: &Arc<Self>) -> Option<Arc<FeeRateOptions>> {
        let fee_response = FEE_CLIENT.fetch_and_get_fees().await.ok()?;
        let fees = Arc::new(FeeRateOptions::from(fee_response));
        self.state.lock().fee_rate_options_base = Some(fees.clone());
        Some(fees)
    }

    fn handle_selected_unit_changed(self: &Arc<Self>, old: Unit, new: Unit) {
        let mut sender = DeferredSender::new(self.clone());
        self.state.lock().metadata.selected_unit = new;

        if old == new {
            return;
        }

        // if its already empty clear everything
        {
            let state = self.state.lock();
            let amount_is_empty = state.amount_sats.is_none();
            let entering_btc_amount_is_empty = state.entering_btc_amount.is_empty();
            drop(state);

            if entering_btc_amount_is_empty || amount_is_empty {
                return self.clear_send_amount();
            }
        }

        // if we are entering fiat, then we don't need to update the entering field
        if self.state.lock().metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return;
        }

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        match new {
            Unit::Btc => {
                let amount_string = Amount::from_sat(amount_sats).btc_string();
                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
            Unit::Sat => {
                let amount_string = amount_sats.thousands_int();
                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
        }
    }

    fn handle_btc_or_fiat_changed(self: &Arc<Self>, _old_value: FiatOrBtc, new_value: FiatOrBtc) {
        let mut sender = DeferredSender::new(self.clone());
        self.state.lock().metadata.fiat_or_btc = new_value;

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        match new_value {
            FiatOrBtc::Btc => {
                let amount = Amount::from_sat(amount_sats);

                let amount_fmt = match self.state.lock().metadata.selected_unit {
                    Unit::Btc => amount.btc_string(),
                    Unit::Sat => amount.sats_string(),
                };

                self.set_and_send_entering_btc_amount(amount_fmt.clone(), &mut sender);
            }

            FiatOrBtc::Fiat => {
                let currency = self.state.lock().selected_fiat_currency;
                let fiat_amount = self.state.lock().amount_fiat.unwrap_or_default();
                let fiat_amount_fmt =
                    format!("{}{}", currency.symbol(), fiat_amount.thousands_fiat(),);

                self.set_and_send_entering_fiat_amount(fiat_amount_fmt.clone(), &mut sender);
            }
        }
    }

    fn handle_prices_changed(self: &Arc<Self>, prices: Arc<PriceResponse>) {
        let selected_currency = self.state.lock().selected_fiat_currency;
        let btc_price_in_fiat = prices.get_for_currency(selected_currency);

        self.state.lock().btc_price_in_fiat = Some(btc_price_in_fiat);

        let Some(amount) = self.state.lock().amount_sats else {
            return;
        };

        let amount_fiat = Amount::from_sat(amount).as_btc() * (btc_price_in_fiat as f64);
        self.state.lock().amount_fiat = Some(amount_fiat);
        self.send(Message::UpdateAmountFiat(amount_fiat));
    }

    fn handle_scan_code_changed(self: &Arc<Self>, _old_value: String, new_value: String) {
        debug!("handle_scan_code_changed {new_value}");
        let mut sender = DeferredSender::new(self.clone());

        let network = self.state.lock().metadata.network;
        let address_with_network = {
            let new_value_moved = new_value;
            match AddressWithNetwork::try_new(&new_value_moved) {
                Ok(address_with_network) => address_with_network,
                Err(err) => {
                    let error = SendFlowError::from_address_error(err, new_value_moved);
                    return self.send_alert(error);
                }
            }
        };

        if !address_with_network.is_valid_for_network(network) {
            let error = SendFlowError::WrongNetwork {
                address: address_with_network.address.to_string(),
                valid_for: address_with_network.network,
                current: network,
            };
            return self.send_alert(error);
        }

        // set address
        let address = Arc::new(address_with_network.address);

        self.state.lock().address = Some(address.clone());
        sender.send(Message::UpdateAddress(Some(address.clone())));

        self.state.lock().entering_address = address.to_string();
        sender.send(Message::UpdateEnteringAddress(address.to_string()));

        let mut should_show_amount_error = false;

        // set amount if its valid
        if let Some(amount) = address_with_network.amount {
            let max_was_selected = self.state.lock().max_selected.take().is_some();
            if max_was_selected {
                sender.send(Message::UnsetMaxSelected)
            }

            should_show_amount_error = true;
            self.handle_amount_changed(amount);
        }

        // if amount is invalid, go to amount field
        if !self.validate_amount(should_show_amount_error) {
            let focus_field = SetAmountFocusField::Amount;
            self.state.lock().focus_field = Some(focus_field);
            sender.send(Message::UpdateFocusField(Some(focus_field)));
        }

        // if both address and amount are valid, then clear the focus field
        if self.validate_amount(false) && self.validate_address(false) {
            self.state.lock().focus_field = None;
            sender.send(Message::UpdateFocusField(None));
        }

        // the address or amount might have changed
        // lets update the fee rate options if its needed
        let me = self.clone();
        let is_max_selected = self.state.lock().max_selected.is_some();
        task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
            if is_max_selected {
                me.select_max_send_report_error().await;
            }
        });
    }

    /// Create the PSBT and everything is valid go to the next screen
    fn finalize_and_go_to_next_screen(self: &Arc<Self>) {
        if !self.validate_amount(true) || !self.validate_address(true) {
            return;
        };

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return self.send_alert(SendFlowError::InvalidNumber);
        };

        let amount = Amount::from_sat(amount_sats);

        let Some(address) = self.state.lock().address.clone() else {
            let invalid_address = self.state.lock().entering_address.clone();
            return self.send_alert(SendFlowError::InvalidAddress(invalid_address));
        };

        let Some(selected_fee_rate) = self.state.lock().selected_fee_rate.clone() else {
            return self.send_alert(SendFlowError::UnableToGetFeeRate);
        };

        self.send(Message::UpdateFocusField(None));

        let (wallet_type, wallet_id) = {
            let state = self.state.lock();
            (state.metadata.wallet_type, state.metadata.id.clone())
        };

        let me = self.clone();
        let manager = self.wallet_manager.clone();

        task::spawn(async move {
            let confirm_details =
                manager.confirm_txn(amount, address, selected_fee_rate.fee_rate).await;

            let details = match confirm_details {
                Ok(details) => details,
                Err(error) => {
                    let error = SendFlowError::UnableToBuildTxn(error.to_string());
                    return me.send_alert_async(error).await;
                }
            };

            let details = Arc::new(details);

            // save the unsigned transaction if its a cold wallet
            if matches!(wallet_type, WalletType::Cold | WalletType::XpubOnly) {
                if let Err(e) = manager.save_unsigned_transaction(details.clone()) {
                    let error = SendFlowError::UnableToSaveUnsignedTransaction(e.to_string());
                    me.send_alert_async(error).await;
                }
            }

            // update the route send the frontend to the proper next screen
            let next_route = match wallet_type {
                WalletType::Hot => RouteFactory::new().send_confirm(wallet_id, details, None, None),
                WalletType::Cold | WalletType::XpubOnly => {
                    RouteFactory::new().send_hardware_export(wallet_id, details)
                }
                WalletType::WatchOnly => {
                    return me
                        .send_alert_async(SendFlowError::UnableToBuildTxn("watch only".to_string()))
                        .await;
                }
            };

            FfiApp::global().dispatch(AppAction::PushRoute(next_route));
        });
    }
}

/// MARK: helper method impls
impl RustSendFlowManager {
    fn send(self: &Arc<Self>, message: SendFlowManagerReconcileMessage) {
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

    fn set_and_send_entering_btc_amount(
        self: &Arc<Self>,
        new_entering_btc_amount: String,
        deffered_sender: &mut DeferredSender,
    ) {
        let is_changed = {
            let mut state = self.state.lock();
            let current = std::mem::take(&mut state.entering_btc_amount);
            state.entering_btc_amount = new_entering_btc_amount.clone();
            current != new_entering_btc_amount
        };

        if is_changed {
            deffered_sender.send(Message::UpdateEnteringBtcAmount(new_entering_btc_amount));
        }
    }

    fn set_and_send_entering_fiat_amount(
        self: &Arc<Self>,
        new_entering_fiat_amount: String,
        deferred_sender: &mut DeferredSender,
    ) {
        let is_changed = {
            let mut state = self.state.lock();
            let current = std::mem::take(&mut state.entering_fiat_amount);
            state.entering_fiat_amount = new_entering_fiat_amount.clone();
            current != new_entering_fiat_amount
        };

        if is_changed {
            deferred_sender.send(Message::UpdateEnteringFiatAmount(new_entering_fiat_amount));
        }
    }

    async fn send_async(self: &Arc<Self>, message: SendFlowManagerReconcileMessage) {
        debug!("send_async: {message:?}");
        if let Err(err) = self.reconciler.send_async(message).await {
            error!("unable to send message to send flow manager: {err}");
        }
    }

    fn send_alert(self: &Arc<Self>, alert: impl Into<SendFlowAlertState>) {
        self.send(Message::SetAlert(alert.into()));
    }

    async fn send_alert_async(self: &Arc<Self>, alert: impl Into<SendFlowAlertState>) {
        self.send_async(Message::SetAlert(alert.into())).await;
    }

    fn total_spent_btc_amount(self: &Arc<Self>, amount_sats: u64) -> Option<Amount> {
        let selected_fee_rate = self.state.lock().selected_fee_rate.as_ref()?.clone();

        let amount = Amount::from_sat(amount_sats);
        let total_fee = selected_fee_rate.total_fee();

        Some(amount + total_fee)
    }

    // Get the first address for the wallet
    // Get the fee rate options
    fn background_init_tasks(self: &Arc<Self>) {
        let me = self.clone();
        let state = self.state.clone();

        task::spawn(async move {
            // get and save first address
            me.get_first_address().await;

            // get fee rate options
            me.get_or_update_fee_rate_options().await;

            me.get_wallet_balance().await;

            state.lock().init_complete = true;
        });
    }

    async fn get_first_address(self: &Arc<Self>) {
        if let Ok(first_address) = self.wallet_manager.first_address().await {
            let address = first_address.address.clone().into();
            self.state.lock().first_address = Some(Arc::new(address));
        }
    }

    fn sync_wrap_get_or_update_fee_rate_options(self: &Arc<Self>) {
        let me = self.clone();
        task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
        });
    }

    async fn get_or_update_fee_rate_options(self: &Arc<Self>) {
        debug!("get_or_update_fee_rate_options");

        let mut sender = DeferredSender::new(self.clone());

        let (address, amount_sats) = {
            let state = self.state.lock();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

        debug!("get_or_update_fee_rate_options: {address:?}, {amount_sats:?}");
        let wallet_actor = self.wallet_actor();
        let state = self.state.clone();

        let fee_rate_options_base = {
            let fee_rate_options_base = state.lock().fee_rate_options_base.clone();
            let fee_rate_options_base = match fee_rate_options_base {
                Some(fee_rate_options_base) => Some(fee_rate_options_base),
                None => self.get_and_update_base_fee_rate_options().await,
            };

            match fee_rate_options_base {
                Some(fee_rate_options_base) => Arc::unwrap_or_clone(fee_rate_options_base),
                None => return,
            }
        };

        let address = {
            let first_address = state.lock().first_address.clone();
            if first_address.is_none() {
                let _ = self.get_first_address().await;
            }

            match (address, first_address) {
                (Some(address), _) => address,
                (None, Some(first_address)) => first_address,
                _ => return,
            }
        };

        let address = Arc::unwrap_or_clone(address);
        let amount_sats = amount_sats.unwrap_or(10_000);
        let amount = Amount::from_sat(amount_sats);

        let max_selected = self.state.lock().max_selected.clone();

        let new_fee_rate_options = match max_selected {
            Some(_) => {
                call!(wallet_actor.fee_rate_options_with_total_fee_for_drain(
                    fee_rate_options_base,
                    address.clone()
                ))
            }
            None => {
                call!(wallet_actor.fee_rate_options_with_total_fee(
                    fee_rate_options_base,
                    amount.into(),
                    address.clone()
                ))
            }
        }
        .await
        .unwrap();

        let mut fee_rate_options = match new_fee_rate_options {
            Ok(fee_rate_options) => fee_rate_options,
            Err(_) => return,
        };

        // if user had a custom speed selected, re-apply it
        let selected_fee_rate = state.lock().selected_fee_rate.clone();
        if let Some(updated_options) = self
            .updated_custom_fee_option(address.clone(), amount, fee_rate_options, selected_fee_rate)
            .await
        {
            fee_rate_options = updated_options;
        }

        // update the state
        let fee_rate_options_with_total_fee = Arc::new(fee_rate_options);
        state.lock().fee_rate_options = Some(fee_rate_options_with_total_fee.clone());

        // if no fee rate is selected, then set the default to medium
        let selected_fee_rate = self.state.lock().selected_fee_rate.clone();
        match selected_fee_rate {
            Some(selected_fee_rate) => {
                let new_selected_fee_rate = match selected_fee_rate.fee_speed {
                    FeeSpeed::Custom { .. } => {
                        fee_rate_options.custom().unwrap_or_else(|| fee_rate_options.medium.into())
                    }
                    FeeSpeed::Fast => fee_rate_options.fast.into(),
                    FeeSpeed::Medium => fee_rate_options.medium.into(),
                    FeeSpeed::Slow => fee_rate_options.slow.into(),
                };

                if new_selected_fee_rate != selected_fee_rate {
                    self.state.lock().selected_fee_rate = Some(new_selected_fee_rate.clone());
                    sender.send(Message::UpdateSelectedFeeRate(new_selected_fee_rate));
                }
            }
            None => {
                let medium = Arc::new(fee_rate_options_with_total_fee.clone().medium);
                self.state.lock().selected_fee_rate = Some(medium.clone());
                sender.send(Message::UpdateSelectedFeeRate(medium));
            }
        }

        sender.send(Message::UpdateFeeRateOptions(fee_rate_options_with_total_fee));
    }

    /// Returns the fee rate options with the updated custom fee
    async fn updated_custom_fee_option(
        self: &Arc<Self>,
        address: Address,
        amount: Amount,
        fee_rate_options: FeeRateOptionsWithTotalFee,
        selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    ) -> Option<FeeRateOptionsWithTotalFee> {
        // only update if the selected fee rate is custom
        let selected_fee_rate = selected_fee_rate?;
        if !matches!(selected_fee_rate.fee_speed, FeeSpeed::Custom { .. }) {
            return None;
        }

        let wallet_actor = self.wallet_actor();
        let old_fee_rate = selected_fee_rate.fee_rate;
        let max_selected = self.state.lock().max_selected.clone();

        let psbt = match max_selected {
            Some(_) => {
                call!(wallet_actor.build_ephemeral_drain_tx(address, old_fee_rate))
            }
            None => {
                call!(wallet_actor.build_ephemeral_tx(amount.into(), address, old_fee_rate.into()))
            }
        }
        .await
        .unwrap();

        let total_fee = psbt
            .map_err(|error| error.to_string())
            .and_then(|psbt| psbt.fee().map_err(|error| error.to_string()));

        let total_fee = match total_fee {
            Ok(total_fee) => total_fee.into(),
            Err(error) => {
                let error = SendFlowError::UnableToGetMaxSend(error.to_string());
                self.send_async(Message::SetAlert(error.into())).await;
                return None;
            }
        };

        let mut new_custom_with_fee = Arc::unwrap_or_clone(selected_fee_rate.clone());
        new_custom_with_fee.total_fee = total_fee;

        let fee_rate_options = fee_rate_options.add_custom_fee_rate(new_custom_with_fee.into());
        Some(fee_rate_options)
    }

    async fn get_wallet_balance(self: &Arc<Self>) {
        let balance = self.wallet_manager.balance().await;
        let wallet_balance = Arc::new(balance);
        self.state.lock().wallet_balance = Some(wallet_balance.clone());
    }
}

#[derive(Debug, Clone)]
struct DeferredSender {
    manager: Arc<RustSendFlowManager>,
    messages: Vec<Message>,
}

impl DeferredSender {
    fn new(manager: Arc<RustSendFlowManager>) -> Self {
        Self { manager, messages: vec![] }
    }

    fn send(&mut self, message: Message) {
        self.messages.push(message);
    }
}

impl Drop for DeferredSender {
    fn drop(&mut self) {
        let messages = std::mem::take(&mut self.messages);

        if !messages.is_empty() {
            let manager = self.manager.clone();

            for message in messages {
                manager.send(message);
            }
        }
    }
}
