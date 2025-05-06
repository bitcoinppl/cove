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
use tracing::{debug, error, trace};

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

#[uniffi::export]
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
    pub fn amount_sats(&self) -> u64 {
        self.state.lock().amount_sats.unwrap_or(0)
    }

    #[uniffi::method]
    pub fn send_amount_btc(&self, amount_sats: Option<u64>) -> String {
        let send_amount = amount_sats.unwrap_or(0);
        let send_amount = Amount::from_sat(send_amount);
        match self.state.lock().metadata.selected_unit {
            Unit::Btc => send_amount.as_btc().thousands().to_string(),
            Unit::Sat => send_amount.as_sats().thousands_int().to_string(),
        }
    }

    #[uniffi::method]
    pub fn send_amount_fiat(&self, amount_sats: Option<u64>) -> String {
        let Some(btc_price_in_fiat) = self.state.lock().btc_price_in_fiat else {
            return "---".to_string();
        };

        let send_amount_in_fiat = self.state.lock().amount_fiat.unwrap_or_else(|| {
            let amount_sats = amount_sats.unwrap_or(0);
            let send_amount = Amount::from_sat(amount_sats);

            send_amount.as_btc().ceil() * (btc_price_in_fiat as f64)
        });

        self.display_fiat_amount(send_amount_in_fiat, true).to_string()
    }

    #[uniffi::method]
    pub fn total_spent_btc_string(&self, amount_sats: Option<u64>) -> String {
        let Some(amount_sats) = amount_sats else {
            return "---".to_string();
        };

        let Some(total_spent) = self.total_spent_btc_amount(amount_sats) else {
            return "---".to_string();
        };

        match self.state.lock().metadata.selected_unit {
            Unit::Btc => format!("{} BTC", total_spent.as_btc()),
            Unit::Sat => format!("{} sats", total_spent.as_sats()),
        }
    }

    #[uniffi::method]
    pub fn total_spent_fiat(&self, amount_sats: Option<u64>) -> String {
        let Some(amount_sats) = amount_sats else {
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
            if display_alert {
                self.send(Message::SetAlert(SendFlowError::InvalidNumber.into()))
            }
            return false;
        };

        if amount == 0 {
            if display_alert {
                self.send(Message::SetAlert(SendFlowError::ZeroAmount.into()));
            }
            return false;
        }

        if amount < 5000 {
            if display_alert {
                self.send(Message::SetAlert(SendFlowError::SendAmountToLow.into()));
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
            if display_alert {
                self.send(Message::SetAlert(SendFlowError::InsufficientFunds.into()));
            }
            return false;
        }

        let selected_fee_rate = self.state.lock().selected_fee_rate.clone();
        if let Some(fee_rate) = &selected_fee_rate {
            let fee = fee_rate.total_fee().to_sat();
            if amount + fee > spendable_balance {
                if display_alert {
                    self.send(Message::SetAlert(SendFlowError::InsufficientFunds.into()));
                }
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

        let handler = FiatOnChangeHandler::new(prices, selected_currency);
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
                self.state.lock().selected_fee_rate = Some(fee_rate.clone());
                self.send(Message::UpdateFeeRate(fee_rate));
            }

            Action::SelectMaxSend => {
                let me = self.clone();
                task::spawn(async move {
                    match me.select_max_send().await {
                        Ok(_) => {}
                        Err(error) => {
                            let alert = SendFlowAlertState::from(error);
                            me.send(Message::SetAlert(alert));
                        }
                    }
                });
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

        let state: State = self.state.clone().into();
        let me = self.clone();
        let fee_rate_options_base = state.lock().fee_rate_options_base.clone();

        if fee_rate_options_base.is_none() {
            crate::task::spawn(async move {
                me.get_fee_rate_options().await;
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

        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;

        let handler = FiatOnChangeHandler::new(prices, selected_currency);
        let Ok(result) = handler.on_change(&old_value, &new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        trace!("result: {result:?}, old_value: {old_value}, new_value: {new_value}");

        let fiat_on_change::Changeset { entering_fiat_amount, fiat_value, btc_amount } = result;

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

        Some(())
    }

    fn entering_address_changed(&self, address: String) {
        self.state.lock().entering_address = address.clone();
        self.send(Message::UpdateEnteringAddress(address.clone()));

        // if the address is valid, then set it in the state
        let address = Address::from_string(&address, self.state.lock().metadata.network).ok();
        if let Some(address) = address {
            self.state.lock().address = Some(Arc::new(address));
        }
    }

    fn clear_send_amount(&self) {
        let currency = self.state.lock().selected_fiat_currency;
        let mut state = self.state.lock();

        state.amount_sats = None;
        self.send(Message::UpdateAmountSats(0));

        state.amount_fiat = None;
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
            self.get_fee_rate_options().await;
        }

        let wallet_actor = self.wallet_actor();

        // use the selected fee rate if we have have
        // or the medium base fee rate
        // or a default of 50 sat/vb
        let fee_rate = selected_fee_rate
            .map(|selected| selected.fee_rate)
            .or_else(|| selected_fee_rate_base.map(|base| base.medium.fee_rate))
            .unwrap_or_else(|| FeeRate::from_sat_per_vb(50.0));

        let psbt: Psbt = call!(wallet_actor.build_ephemeral_drain_tx(address, fee_rate))
            .await
            .unwrap()
            .map_err(|error| Error::UnableToGetMaxSend(error.to_string()))?
            .into();

        let total = Arc::new(psbt.output_total_amount());

        self.state.lock().max_selected = Some(total.clone());
        self.send(Message::SetMaxSelected(total.clone()));

        self.handle_amount_changed(*total);

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

        let amount_sats = match self.state.lock().amount_sats {
            Some(amount_sats) => amount_sats,
            None => return,
        };

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

    fn handle_scan_code_changed(&self, _old_value: String, new_value: String) {
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

        task::spawn(async move {
            // get and save first address
            me.get_first_address().await;

            // get fee rate options
            me.get_fee_rate_options().await;

            me.get_wallet_balance().await;
        });
    }

    async fn get_first_address(self: &Arc<Self>) {
        if let Ok(first_address) = self.wallet_manager.first_address().await {
            let address = first_address.address.clone().into();
            self.state.lock().first_address = Some(Arc::new(address));
        }
    }

    async fn get_fee_rate_options(self: &Arc<Self>) {
        let (address, amount_sats) = {
            let state = self.state.lock();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

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

        let first_address = state.lock().first_address.clone();
        if first_address.is_none() {
            let _ = self.get_first_address().await;
        }

        let address = match (address, first_address) {
            (Some(address), _) => address,
            (None, Some(first_address)) => first_address,
            _ => return,
        };

        let amount_sats = amount_sats.unwrap_or(10_000);
        let amount = Amount::from_sat(amount_sats);

        let fee_rate_options = call!(wallet_actor.fee_rate_options_with_total_fee(
            fee_rate_options_base,
            amount.into(),
            Arc::unwrap_or_clone(address)
        ))
        .await
        .unwrap();

        let mut fee_rate_options = match fee_rate_options {
            Ok(fee_rate_options) => fee_rate_options,
            Err(_) => return,
        };

        // if user had a custom speed selected, re-apply it
        let selected_fee_rate = state.lock().selected_fee_rate.clone();
        if fee_rate_options.custom().is_none() {
            if let Some(selected) = &selected_fee_rate {
                if let FeeSpeed::Custom { .. } = selected.fee_speed() {
                    fee_rate_options = fee_rate_options.add_custom_fee_rate(selected.clone());
                }
            }
        };

        // update the state
        let fee_rate_options_with_total_fee = Arc::new(fee_rate_options);
        state.lock().fee_rate_options = Some(fee_rate_options_with_total_fee.clone());

        // if no fee rate is selected, then set the default to medium
        let selected_fee_rate = self.state.lock().selected_fee_rate.clone();
        if selected_fee_rate.is_none() {
            let medium = Arc::new(fee_rate_options_with_total_fee.clone().medium);
            self.state.lock().selected_fee_rate = Some(medium.clone());
            self.send(Message::UpdateFeeRate(medium));
        }

        let _ = sender.send(Message::UpdateFeeRateOptions(fee_rate_options_with_total_fee));
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
