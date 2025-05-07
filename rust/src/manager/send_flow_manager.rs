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
use flume::{Receiver, Sender};
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
    UpdateAddress(Arc<Address>),

    SetMaxSelected(Arc<Amount>),
    UnsetMaxSelected,

    UpdateAmountSats(u64),
    UpdateAmountFiat(f64),

    UpdateFocusField(Option<SetAmountFocusField>),
    UpdateFeeRate(Arc<FeeRateOptionWithTotalFee>),

    UpdateSelectedFeeRate(Arc<FeeRateOptionWithTotalFee>),
    UpdateFeeRateOptions(Arc<FeeRateOptionsWithTotalFee>),

    // side effects
    SetAlert(SendFlowAlertState),
    ClearAlert,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerAction {
    ChangeEnteringBtcAmount(String),
    ChangeEnteringFiatAmount(String),
    ChangeEnteringAddress(String),

    ChangeSetAmountFocusField(Option<SetAmountFocusField>),

    SelectMaxSend,
    ClearSendAmount,

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),

    // front end lets the one of the values were changed
    NotifySelectedUnitedChanged { old: Unit, new: Unit },
    NotifyBtcOrFiatChanged { old: FiatOrBtc, new: FiatOrBtc },
    NotifyScanCodeChanged { old: String, new: String },
    NotifyPricesChanged(Arc<PriceResponse>),
    NotifyFocusFieldChanged { old: Option<SetAmountFocusField>, new: Option<SetAmountFocusField> },

    // starting with an amount and address from scan
    NotifyAddressChanged(Arc<Address>),
    NotifyAmountChanged(Arc<Amount>),

    FinalizeAndGoToNextScreen,
}

impl RustSendFlowManager {
    pub fn new(metadata: WalletMetadata, wallet_manager: Arc<RustWalletManager>) -> Arc<Self> {
        let (sender, receiver) = flume::bounded(100);

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

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
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
    pub async fn wait_for_init(&self) {
        let mut times = 0;
        while !self.state.lock().init_complete {
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
            Unit::Btc => send_amount.as_btc().thousands().to_string(),
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
    pub fn total_spent_btc_string(&self) -> String {
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
    pub fn total_spent_fiat(&self) -> String {
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
            Unit::Btc => format!("{} BTC", total_fee.as_btc()),
            Unit::Sat => format!("{} sats", total_fee.as_sats()),
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
    pub fn validate_address(&self, display_alert: bool) -> bool {
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
    pub fn validate_amount(&self, display_alert: bool) -> bool {
        let Some(amount) = self.state.lock().amount_sats else {
            let msg = Message::SetAlert(SendFlowError::InvalidNumber.into());
            if display_alert {
                self.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }

            return false;
        };

        if amount == 0 {
            let msg = Message::SetAlert(SendFlowError::ZeroAmount.into());
            if display_alert {
                self.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            };
            return false;
        }

        if amount < 5000 {
            let msg = Message::SetAlert(SendFlowError::SendAmountToLow.into());
            if display_alert {
                self.send(msg);
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
            let msg = Message::SetAlert(SendFlowError::InsufficientFunds.into());
            if display_alert {
                self.send(msg);
            } else {
                debug!("validate_amount_failed: {msg:?}");
            }
            return false;
        }

        let selected_fee_rate = self.state.lock().selected_fee_rate.clone();
        if let Some(fee_rate) = &selected_fee_rate {
            let fee = fee_rate.total_fee().to_sat();
            if amount + fee > spendable_balance {
                let msg = Message::SetAlert(SendFlowError::InsufficientFunds.into());
                if display_alert {
                    self.send(msg);
                } else {
                    debug!("validate_amount_failed: {msg:?}");
                };
                return false;
            }
        }

        true
    }

    #[uniffi::method]
    fn sanitize_btc_entering_amount(&self, old_value: &str, new_value: &str) -> Option<String> {
        let on_change_handler = BtcOnChangeHandler::new(self.state.clone());
        let changeset = on_change_handler.on_change(old_value, new_value);
        let entering_amount_btc = changeset.entering_amount_btc?;

        if entering_amount_btc == new_value {
            return None;
        };

        Some(entering_amount_btc)
    }

    #[uniffi::method]
    fn sanitize_fiat_entering_amount(&self, old_value: &str, new_value: &str) -> Option<String> {
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
        match action {
            Action::ChangeEnteringBtcAmount(string) => {
                let old_value = self.state.lock().entering_btc_amount.clone();
                self.btc_field_changed(old_value, string);
            }

            Action::ChangeEnteringFiatAmount(string) => {
                let old_value = self.state.lock().entering_fiat_amount.clone();
                self.fiat_field_changed(old_value, string);
            }

            Action::ChangeEnteringAddress(address) => {
                self.entering_address_changed(address);
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
                self.state.lock().address = Some(address.clone());
                self.state.lock().entering_address = address.to_string();
            }

            Action::NotifyAmountChanged(amount) => {
                self.handle_amount_changed(*amount);
            }

            Action::NotifyFocusFieldChanged { old, new } => {
                self.handle_focus_field_changed(old, new);
            }
        }
    }
}

/// MARK: State mutating impl
impl RustSendFlowManager {
    fn btc_field_changed(self: Arc<Self>, old: String, new: String) -> Option<()> {
        trace!("btc_field_changed {old} --> {new}");
        if old == new {
            return None;
        }

        // update the state
        self.state.lock().entering_btc_amount = new.clone();
        self.send(Message::UpdateEnteringBtcAmount(new.clone()));

        let state: State = self.state.clone().into();
        let me = self.clone();

        let fee_rate_options_base = state.lock().fee_rate_options_base.clone();
        if fee_rate_options_base.is_none() {
            crate::task::spawn(async move {
                me.get_and_update_base_fee_rate_options().await;
            });
        }

        let state: State = self.state.clone().into();
        let handler = BtcOnChangeHandler::new(state.clone());
        let changes = handler.on_change(&old, &new);
        debug!("btc_on_change_handler changes: {changes:?}");

        let btc_on_change::Changeset { entering_amount_btc, max_selected, amount_btc, amount_fiat } =
            changes;

        // mutate the state
        {
            let mut state = state.lock();

            match max_selected {
                Some(Some(max)) => {
                    let max = Arc::new(max);
                    state.max_selected = Some(max.clone());
                    self.send(Message::SetMaxSelected(max));
                }
                Some(None) => {
                    state.max_selected = None;
                    self.send(Message::UnsetMaxSelected);
                }
                None => {}
            }

            if let Some(amount) = amount_btc {
                let amount_sats = amount.to_sat();
                state.amount_sats = Some(amount_sats);
                self.send(Message::UpdateAmountSats(amount_sats));
            }

            if let Some(amount) = amount_fiat {
                state.amount_fiat = Some(amount);
                self.send(Message::UpdateAmountFiat(amount));
            }

            if let Some(entering_amount) = entering_amount_btc {
                state.entering_btc_amount = entering_amount.clone();
                self.send(Message::UpdateEnteringBtcAmount(entering_amount));
            }
        };

        Some(())
    }

    fn fiat_field_changed(&self, old_value: String, new_value: String) -> Option<()> {
        trace!("fiat_field_changed {old_value} --> {new_value}");
        if old_value == new_value {
            return None;
        }

        // update the state
        self.state.lock().entering_fiat_amount = new_value.clone();
        self.send(Message::UpdateEnteringFiatAmount(new_value.clone()));

        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;
        let max_selected = self.state.lock().max_selected.as_deref().copied();

        let handler = FiatOnChangeHandler::new(prices, selected_currency, max_selected);
        let Ok(result) = handler.on_change(&old_value, &new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        trace!("result: {result:?}, old_value: {old_value}, new_value: {new_value}");

        let fiat_on_change::Changeset {
            entering_fiat_amount,
            fiat_value,
            btc_amount,
            max_selected,
        } = result;

        if let Some(entering_fiat_amount) = entering_fiat_amount {
            self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
            self.send(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
        }

        if let Some(amount_fiat) = fiat_value {
            self.state.lock().amount_fiat = Some(amount_fiat);
            self.send(Message::UpdateAmountFiat(amount_fiat));
        }

        if let Some(btc_amount) = btc_amount {
            let btc_amount = btc_amount.as_sats();
            self.state.lock().amount_sats = Some(btc_amount);
            self.send(Message::UpdateAmountSats(btc_amount));
        }

        if let Some(None) = max_selected {
            self.state.lock().max_selected = None;
            self.send(Message::UnsetMaxSelected);
        }

        Some(())
    }

    fn entering_address_changed(self: &Arc<Self>, address: String) {
        // update the state
        self.state.lock().entering_address = address.clone();
        self.send(Message::UpdateEnteringAddress(address.clone()));

        // if the address is valid, then set it in the state
        let address = Address::from_string(&address, self.state.lock().metadata.network).ok();
        let Some(address) = address else { return };

        let address = Arc::new(address);
        self.state.lock().address = Some(address.clone());
        self.send(Message::UpdateAddress(address.clone()));

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

    fn clear_send_amount(&self) {
        let currency = self.state.lock().selected_fiat_currency;
        let mut state = self.state.lock();

        state.amount_sats = Some(0);
        self.send(Message::UpdateAmountSats(0));

        state.amount_fiat = Some(0.0);
        self.send(Message::UpdateAmountFiat(0.0));

        // fiat
        let entering_fiat_amount = currency.symbol().to_string();
        state.entering_fiat_amount = entering_fiat_amount.clone();
        self.send(Message::UpdateEnteringFiatAmount(entering_fiat_amount));

        // btc
        state.entering_btc_amount = String::new();
        self.send(Message::UpdateEnteringBtcAmount(String::new()));

        drop(state);
    }

    fn selected_fee_rate_changed(self: &Arc<Self>, fee_rate: Arc<FeeRateOptionWithTotalFee>) {
        self.state.lock().selected_fee_rate = Some(fee_rate.clone());
        self.send(Message::UpdateFeeRate(fee_rate.clone()));

        // max was selected before, so we need to update it to match the new fee rate
        let max_selected = self.state.lock().max_selected.clone();
        if max_selected.is_some() {
            self.clone().dispatch(Action::SelectMaxSend);
        }
    }

    /// When amount is changed, we will need to update the entering and fiat amounts
    fn handle_amount_changed(&self, amount: Amount) {
        debug!("handle_amount_changed: {amount:?}");

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
                    let amount_fiat = amount.as_btc() * (price as f64);

                    let enterting_amount_fiat = amount_fiat.thousands_fiat();
                    self.state.lock().entering_fiat_amount = enterting_amount_fiat.clone();
                    self.send(Message::UpdateEnteringFiatAmount(enterting_amount_fiat));
                }
            }

            FiatOrBtc::Btc => {
                let amount_string = match unit {
                    Unit::Btc => amount.btc_string(),
                    Unit::Sat => amount.as_sats().thousands_int(),
                };

                self.state.lock().entering_btc_amount = amount_string.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_string));
            }
        }

        let amount_sats = amount.to_sat();
        self.state.lock().amount_sats = Some(amount_sats);
        self.send(Message::UpdateAmountSats(amount_sats));

        if let Some(price) = btc_price_in_fiat {
            let amount_fiat = amount.as_btc() * (price as f64);
            self.state.lock().amount_fiat = Some(amount_fiat);
            self.send(Message::UpdateAmountFiat(amount_fiat));
        }
    }

    fn handle_focus_field_changed(
        &self,
        old: Option<SetAmountFocusField>,
        new: Option<SetAmountFocusField>,
    ) {
        debug!("handle_focus_field_changed: {old:?} --> {new:?}");

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
                self.send(Message::UpdateFocusField(Some(SetAmountFocusField::Amount)));
                return;
            }

            if !self.validate_address(should_show_error) {
                self.state.lock().focus_field = Some(SetAmountFocusField::Address);
                self.send(Message::UpdateFocusField(Some(SetAmountFocusField::Address)));
                return;
            }
        }

        self.state.lock().focus_field = new;
        self.send(Message::UpdateFocusField(new));
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

        // access the mutex once
        let (address, fee_rate_options, selected_fee_rate, selected_fee_rate_base) = {
            let state = self.state.lock();
            let address_string = &state.entering_address;

            let address = Address::from_string(address_string, state.metadata.network)
                .ok()
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
        self.send(Message::SetMaxSelected(total.clone()));
        self.handle_amount_changed(*total);

        let address_is_valid = self.state.lock().address.is_some();
        match address_is_valid {
            true => {
                self.state.lock().focus_field = None;
                self.send(Message::UpdateFocusField(None))
            }
            false => {
                self.state.lock().focus_field = Some(SetAmountFocusField::Address);
                self.send(Message::UpdateFocusField(Some(SetAmountFocusField::Address)))
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

    fn handle_selected_unit_changed(&self, old: Unit, new: Unit) {
        self.state.lock().metadata.selected_unit = new;

        if old == new {
            return;
        }

        // if its already empty clear everythign
        {
            let state = self.state.lock();
            let entering_btc_amount_is_empty = state.entering_btc_amount.is_empty();
            let entering_fiat_amount_is_empty = state.entering_fiat_amount.is_empty();
            drop(state);

            if old == Unit::Btc && entering_btc_amount_is_empty {
                return self.clear_send_amount();
            }

            if old == Unit::Sat && entering_fiat_amount_is_empty {
                return self.clear_send_amount();
            }
        }

        // if we are entering fiat, then we don't need to update the entering field
        if self.state.lock().metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return;
        }

        let amount_sats = self.state.lock().amount_sats.unwrap_or(0);
        match new {
            Unit::Btc => {
                let amount_string = Amount::from_sat(amount_sats).btc_string();
                self.state.lock().entering_btc_amount = amount_string.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_string));
            }
            Unit::Sat => {
                let amount_string = amount_sats.thousands_int();
                self.state.lock().entering_btc_amount = amount_string.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_string));
            }
        }
    }

    fn handle_btc_or_fiat_changed(&self, _old_value: FiatOrBtc, new_value: FiatOrBtc) {
        self.state.lock().metadata.fiat_or_btc = new_value;

        match new_value {
            FiatOrBtc::Btc => {
                let amount_sats = self.state.lock().amount_sats.unwrap_or_default();
                let amount = Amount::from_sat(amount_sats);

                let amount_fmt = match self.state.lock().metadata.selected_unit {
                    Unit::Btc => amount.btc_string(),
                    Unit::Sat => amount.sats_string(),
                };

                self.state.lock().entering_btc_amount = amount_fmt.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_fmt));
            }

            FiatOrBtc::Fiat => {
                let currency = self.state.lock().selected_fiat_currency;
                let fiat_amount = self.state.lock().amount_fiat.unwrap_or_default();
                let fiat_amount_fmt = format!(
                    "{}{}{}",
                    currency.symbol(),
                    fiat_amount.thousands_fiat(),
                    currency.suffix()
                );

                self.state.lock().entering_fiat_amount = fiat_amount_fmt.clone();
                self.send(Message::UpdateEnteringFiatAmount(fiat_amount_fmt));
            }
        }
    }

    fn handle_prices_changed(&self, prices: Arc<PriceResponse>) {
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
        self.send(Message::UpdateAddress(address.clone()));

        self.state.lock().entering_address = address.to_string();
        self.send(Message::UpdateEnteringAddress(address.to_string()));

        // set amount if its valid
        if let Some(amount) = address_with_network.amount {
            self.handle_amount_changed(amount);
        }

        // if amount is invalid, go to amount field
        if !self.validate_amount(false) {
            let focus_field = SetAmountFocusField::Amount;
            self.state.lock().focus_field = Some(focus_field);
            self.send(Message::UpdateFocusField(Some(focus_field)));
        }

        // if both address and amount are valid, then clear the focus field
        if self.validate_amount(false) && self.validate_address(false) {
            self.state.lock().focus_field = None;
            self.send(Message::UpdateFocusField(None));
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
    fn finalize_and_go_to_next_screen(&self) {
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
        let manager = self.wallet_manager.clone();
        let sender = self.reconciler.clone();
        let wallet_type = self.state.lock().metadata.wallet_type;
        let wallet_id = self.state.lock().metadata.id.clone();

        task::spawn(async move {
            let confirm_details =
                manager.confirm_txn(amount, address, selected_fee_rate.fee_rate).await;

            let details = match confirm_details {
                Ok(details) => details,
                Err(error) => {
                    let error = SendFlowError::UnableToBuildTxn(error.to_string());
                    return send_alert(sender, error);
                }
            };

            let details = Arc::new(details);

            // save the unsigned transaction if its a cold wallet
            if matches!(wallet_type, WalletType::Cold | WalletType::XpubOnly) {
                if let Err(e) = manager.save_unsigned_transaction(details.clone()) {
                    let error = SendFlowError::UnableToSaveUnsignedTransaction(e.to_string());
                    send_alert(sender.clone(), error);
                }
            }

            // update the route send the frontend to the proper next screen
            let next_route = match wallet_type {
                WalletType::Hot => RouteFactory::new().send_confirm(wallet_id, details, None, None),
                WalletType::Cold | WalletType::XpubOnly => {
                    RouteFactory::new().send_hardware_export(wallet_id, details)
                }
                WalletType::WatchOnly => {
                    return send_alert(
                        sender,
                        SendFlowError::UnableToBuildTxn("watch only".to_string()),
                    );
                }
            };

            FfiApp::global().dispatch(AppAction::PushRoute(next_route));
        });
    }
}

/// MARK: helper method impls
impl RustSendFlowManager {
    fn send(&self, message: SendFlowManagerReconcileMessage) {
        debug!("send: {message:?}");
        if let Err(err) = self.reconciler.send(message) {
            error!("unable to send message to send flow manager: {err}");
        }
    }

    fn send_alert(&self, alert: impl Into<SendFlowAlertState>) {
        self.send(Message::SetAlert(alert.into()));
    }

    fn total_spent_btc_amount(&self, amount_sats: u64) -> Option<Amount> {
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

    async fn get_or_update_fee_rate_options(self: &Arc<Self>) {
        debug!("get_or_update_fee_rate_options");
        let (address, amount_sats) = {
            let state = self.state.lock();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

        debug!("get_or_update_fee_rate_options: {address:?}, {amount_sats:?}");
        let wallet_actor = self.wallet_actor();
        let sender = self.reconciler.clone();
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

            let address = match (address, first_address) {
                (Some(address), _) => address,
                (None, Some(first_address)) => first_address,
                _ => return,
            };

            address
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
            .updated_custom_fee_option(
                address.clone(),
                amount.clone(),
                fee_rate_options.clone(),
                selected_fee_rate,
            )
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
                    FeeSpeed::Custom { .. } => fee_rate_options.custom().expect("checked above"),
                    FeeSpeed::Fast => fee_rate_options.fast.into(),
                    FeeSpeed::Medium => fee_rate_options.medium.into(),
                    FeeSpeed::Slow => fee_rate_options.slow.into(),
                };

                if new_selected_fee_rate != selected_fee_rate {
                    self.state.lock().selected_fee_rate = Some(new_selected_fee_rate.clone());
                    self.send(Message::UpdateFeeRate(new_selected_fee_rate));
                }
            }
            None => {
                let medium = Arc::new(fee_rate_options_with_total_fee.clone().medium);
                self.state.lock().selected_fee_rate = Some(medium.clone());
                self.send(Message::UpdateFeeRate(medium));
            }
        }

        let _ = sender.send(Message::UpdateFeeRateOptions(fee_rate_options_with_total_fee));
    }

    /// Returns the fee rate options with the updated custom fee
    async fn updated_custom_fee_option(
        self: &Arc<Self>,
        address: Address,
        amount: Amount,
        fee_rate_options: FeeRateOptionsWithTotalFee,
        selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    ) -> Option<FeeRateOptionsWithTotalFee> {
        // nothing to update
        if fee_rate_options.custom().is_none() {
            return None;
        }

        let selected_fee_rate = selected_fee_rate?;
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
                self.send(Message::SetAlert(error.into()));
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

fn send_alert(sender: Sender<Message>, alert: impl Into<SendFlowAlertState>) {
    let message = Message::SetAlert(alert.into());
    if let Err(err) = sender.send(message) {
        error!("unable to send message to send flow manager: {err}");
    }
}
