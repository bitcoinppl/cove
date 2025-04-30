pub mod alert_state;
pub mod btc_on_change;
pub mod error;
pub mod fiat_on_change;
pub mod state;

use std::sync::Arc;

use crate::{
    app::{App, reconcile::AppStateReconcileMessage},
    fee_client::FEE_CLIENT,
    task,
    transaction::FeeRate,
    wallet::{
        Address,
        metadata::{FiatOrBtc, WalletMetadata},
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
use parking_lot::RwLock;
use state::{SendFlowManagerState, State};
use tokio::task::JoinHandle;
use tracing::error;

use super::wallet::{RustWalletManager, WalletManagerReconcileMessage, actor::WalletActor};

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
    state_listeners: Vec<JoinHandle<()>>,
    pub state: Arc<RwLock<SendFlowManagerState>>,

    reconciler: Sender<Message>,
    reconcile_receiver: Arc<Receiver<Message>>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerReconcileMessage {
    // reconcile state with swift
    UpdateEnteringBtcAmount(String),
    UpdateEnteringFiatAmount(String),
    UpdateEnteringAddress(String),

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

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),

    NotifySelectedUnitedChanged { old: Unit, new: Unit },
    NotifyScanCodeChanged { old: String, new: String },

    SelectMaxSend,
}

impl RustSendFlowManager {
    pub fn new(
        metadata: WalletMetadata,
        wallet_manager: Arc<RustWalletManager>,
        wallet_manager_receiver: Arc<Receiver<WalletManagerReconcileMessage>>,
    ) -> Arc<Self> {
        let (sender, receiver) = flume::bounded(100);

        let state = State::new(metadata);

        let manager_listeners = {
            let wallet_manager_listener =
                start_wallet_manager_listener(state.clone(), wallet_manager_receiver);

            let app_listener = start_app_manager_listener(state.clone(), App::global().receiver());

            vec![wallet_manager_listener, app_listener]
        };

        let me: Arc<Self> = Self {
            app: App::global().clone(),
            state: state.into_inner(),
            wallet_manager,
            state_listeners: manager_listeners,
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
        .into();

        // in background run init tasks
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
        self.state.read().amount_sats.unwrap_or(0)
    }

    #[uniffi::method]
    pub fn total_spent_btc_string(&self) -> String {
        let Some(total_spent) = self.total_spent_btc_amount() else {
            return "---".to_string();
        };

        match self.state.read().metadata.selected_unit {
            Unit::Btc => format!("{} BTC", total_spent.as_btc()),
            Unit::Sat => format!("{} sats", total_spent.as_sats()),
        }
    }

    #[uniffi::method]
    pub fn total_spent_fiat(&self) -> String {
        let Some(total_spent) = self.total_spent_btc_amount() else {
            return "---".to_string();
        };

        let Some(btc_price_in_fiat) = self.state.read().btc_price_in_fiat else {
            return "---".to_string();
        };

        let total_spent_in_fiat = total_spent.as_btc() * (btc_price_in_fiat as f64);
        format!("≈ {}", self.display_fiat_amount(total_spent_in_fiat, true))
    }

    #[uniffi::method]
    pub fn total_fee_string(&self) -> String {
        let Some(selected_fee_rate) = &self.state.read().selected_fee_rate else {
            return "---".to_string();
        };

        let total_fee = selected_fee_rate.total_fee();
        match self.state.read().metadata.selected_unit {
            Unit::Btc => format!("{} BTC", total_fee.as_btc()),
            Unit::Sat => format!("{} sats", total_fee.as_sats()),
        }
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount(&self, amount: f64, with_suffix: bool) -> String {
        {
            let sensitive_visible = self.state.read().metadata.sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let fiat = amount.thousands_fiat();
        let currency = self.state.read().selected_fiat_currency;

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
        if self.state.read().address.is_none() {
            if display_alert {
                let error =
                    SendFlowError::InvalidAddress(self.state.read().entering_address.clone());
                self.send(Message::SetAlert(error.into()));
            }

            return false;
        }

        true
    }

    #[uniffi::method(default(display_alert = false))]
    pub fn validate_amount(&self, display_alert: bool) -> bool {
        let Some(amount) = self.state.read().amount_sats else {
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
            .read()
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

        if let Some(fee_rate) = &self.state.read().selected_fee_rate {
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

    // MARK: Action handler
    /// action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(self: Arc<Self>, action: Action) {
        match action {
            Action::ChangeEnteringBtcAmount(string) => {
                self.state.write().entering_btc_amount = string.clone();
                let old_value = self.state.read().entering_btc_amount.clone();
                self.btc_field_changed(old_value, string);
            }

            Action::ChangeEnteringFiatAmount(string) => {
                self.state.write().entering_fiat_amount = string.clone();
                let old_value = self.state.read().entering_fiat_amount.clone();
                self.fiat_field_changed(old_value, string);
            }

            Action::ChangeEnteringAddress(address) => {
                self.entering_address_changed(address);
            }

            Action::ChangeSetAmountFocusField(set_amount_focus_field) => {
                self.state.write().focus_field = set_amount_focus_field;
                self.send(Message::UpdateFocusField(set_amount_focus_field));
            }

            Action::SelectFeeRate(fee_rate) => {
                self.state.write().selected_fee_rate = Some(fee_rate.clone());
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

            Action::NotifySelectedUnitedChanged { old, new } => {
                self.handle_selected_unit_changed(old, new);
            }

            Action::NotifyScanCodeChanged { old, new } => {
                self.handle_scan_code_changed(old, new);
            }
        }
    }
}

/// MARK: State mutating impl
impl RustSendFlowManager {
    fn btc_field_changed(self: Arc<Self>, old_value: String, new_value: String) -> Option<()> {
        let state: State = self.state.clone().into();

        let me = self.clone();
        if state.read().fee_rate_options.is_none() {
            crate::task::spawn(async move {
                me.get_fee_rate_options().await;
            });
        }

        let state: State = self.state.clone().into();
        let handler = BtcOnChangeHandler::new(state.clone());
        let changes: btc_on_change::Changeset = handler.on_change(&old_value, &new_value);

        let mut state = state.write();

        match changes.max_selected {
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

        if let Some(amount) = changes.amount_btc {
            let amount_sats = amount.to_sat();
            state.amount_sats = Some(amount_sats);
            self.send(Message::UpdateAmountSats(amount_sats));
        }

        if let Some(amount) = changes.amount_fiat {
            state.amount_fiat = Some(amount);
            self.send(Message::UpdateAmountFiat(amount));
        }

        Some(())
    }

    fn fiat_field_changed(&self, old_value: String, new_value: String) -> Option<()> {
        let prices = self.app.prices()?;
        let selected_currency = self.state.read().selected_fiat_currency;

        let handler = FiatOnChangeHandler::new(prices, selected_currency);
        let Ok(result) = handler.on_change(old_value, new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        if let Some(fiat_text) = result.fiat_text {
            self.state.write().entering_fiat_amount = fiat_text.clone();
            self.send(Message::UpdateEnteringFiatAmount(fiat_text));
        }

        if let Some(amount_fiat) = result.fiat_value {
            self.state.write().amount_fiat = Some(amount_fiat);
            self.send(Message::UpdateAmountFiat(amount_fiat));
        }

        if let Some(btc_amount) = result.btc_amount {
            let btc_amount = btc_amount.as_sats();
            self.state.write().amount_sats = Some(btc_amount);
            self.send(Message::UpdateAmountSats(btc_amount));
        }

        Some(())
    }

    fn entering_address_changed(&self, address: String) {
        self.state.write().entering_address = address.clone();
        self.send(Message::UpdateEnteringAddress(address.clone()));

        // if the address is valid, then set it in the state
        let address = Address::from_string(&address, self.state.read().metadata.network).ok();
        if let Some(address) = address {
            self.state.write().address = Some(Arc::new(address));
        }
    }

    async fn select_max_send(self: &Arc<Self>) -> Result<()> {
        let address = {
            let state = self.state.read();
            let address_string = &state.entering_address;

            let address = Address::from_string(&address_string, state.metadata.network)
                .ok()
                .or_else(|| state.first_address.clone().map(Arc::unwrap_or_clone));

            address.ok_or(Error::InvalidAddress(address_string.to_string()))?
        };

        let fee_rate_options = self.state.read().fee_rate_options.clone();
        if fee_rate_options.is_none() {
            self.get_fee_rate_options().await;
        }

        let wallet_actor = self.wallet_actor();

        // use the selected fee rate if we have have
        // or the medium base fee rate
        // or a default of 50 sat/vb
        let fee_rate = self
            .state
            .read()
            .selected_fee_rate
            .clone()
            .map(|selected| selected.fee_rate)
            .or_else(|| {
                self.state.read().fee_rate_options_base.clone().map(|base| base.medium.fee_rate)
            })
            .unwrap_or_else(|| FeeRate::from_sat_per_vb(50.0));

        let psbt: Psbt = call!(wallet_actor.build_ephemeral_drain_tx(address, fee_rate))
            .await
            .unwrap()
            .map_err(|error| Error::UnableToGetMaxSend(error.to_string()))?
            .into();

        let total = psbt.output_total_amount();
        self.send(Message::SetMaxSelected(total.into()));

        Ok(())
    }

    async fn get_and_update_base_fee_rate_options(self: &Arc<Self>) -> Option<Arc<FeeRateOptions>> {
        let fee_response = FEE_CLIENT.fetch_and_get_fees().await.ok()?;
        let fees = Arc::new(FeeRateOptions::from(fee_response));

        self.state.write().fee_rate_options_base = Some(fees.clone());

        Some(fees)
    }

    fn handle_selected_unit_changed(&self, old: Unit, new: Unit) {
        if old == new {
            return;
        }

        // if we are entering fiat, then we don't need to update the entering field
        if self.state.read().metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return;
        }

        let amount_sats = match self.state.read().amount_sats {
            Some(amount_sats) => amount_sats,
            None => {
                self.state.write().entering_btc_amount = String::new();
                self.send(Message::UpdateEnteringBtcAmount(String::new()));
                return;
            }
        };

        match new {
            Unit::Btc => {
                let amount_string = Amount::from_sat(amount_sats).btc_string();
                self.state.write().entering_btc_amount = amount_string.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_string));
            }
            Unit::Sat => {
                let amount_string = amount_sats.thousands_int();
                self.state.write().entering_btc_amount = amount_string.clone();
                self.send(Message::UpdateEnteringBtcAmount(amount_string));
            }
        }
    }

    fn handle_scan_code_changed(&self, _old_value: String, new_value: String) {
        let network = self.state.read().metadata.network;
        let address_with_network = match AddressWithNetwork::try_new(&new_value) {
            Ok(address_with_network) => address_with_network,
            Err(err) => {
                let error = SendFlowError::from_address_error(err, new_value);
                return self.send_alert(error);
            }
        };

        if address_with_network.network != network {
            let error = SendFlowError::WrongNetwork {
                address: address_with_network.address.to_string(),
                valid_for: address_with_network.network,
                current: network,
            };
            return self.send_alert(error);
        }

        // set address
        let address = Arc::new(address_with_network.address);
        self.state.write().address = Some(address.clone());
        self.state.write().entering_address = address.to_string();
        self.send(Message::UpdateEnteringAddress(new_value));

        // set amount if its valid
        if let Some(amount) = address_with_network.amount {
            self.state.write().amount_sats = Some(amount.to_sat());
            self.send(Message::UpdateAmountSats(amount.to_sat()));
        }

        // if amount is invalid, go to amount field
        if !self.validate_amount(false) {
            let focus_field = SetAmountFocusField::Amount;
            self.state.write().focus_field = Some(focus_field);
            self.send(Message::UpdateFocusField(Some(focus_field)));
        }

        // if both address and amount are valid, then clear the focus field
        if self.validate_amount(false) && self.validate_address(false) {
            self.state.write().focus_field = None;
            self.send(Message::UpdateFocusField(None));
        }
    }
}

/// MARK: helper method impls
impl RustSendFlowManager {
    fn send(&self, message: SendFlowManagerReconcileMessage) {
        if let Err(err) = self.reconciler.send(message) {
            error!("unable to send message to send flow manager: {err}");
        }
    }

    fn send_alert(&self, alert: impl Into<SendFlowAlertState>) {
        self.send(Message::SetAlert(alert.into()));
    }

    fn total_spent_btc_amount(&self) -> Option<Amount> {
        let selected_fee_rate = self.state.read().selected_fee_rate.as_ref()?.clone();
        let amount_sats = self.state.read().amount_sats?;

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
            self.state.write().first_address = Some(Arc::new(address));
        }
    }

    async fn get_fee_rate_options(self: &Arc<Self>) {
        let (address, amount_sats) = {
            let state = self.state.read();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

        let wallet_actor = self.wallet_actor();
        let sender = self.reconciler.clone();
        let state = self.state.clone();

        let fee_rate_options_base = {
            let fee_rate_options_base = state.read().fee_rate_options_base.clone();
            let fee_rate_options_base = match fee_rate_options_base {
                Some(fee_rate_options_base) => Some(fee_rate_options_base),
                None => self.get_and_update_base_fee_rate_options().await,
            };

            match fee_rate_options_base {
                Some(fee_rate_options_base) => Arc::unwrap_or_clone(fee_rate_options_base),
                None => return,
            }
        };

        let first_address = state.read().first_address.clone();
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
        let selected_fee_rate = state.read().selected_fee_rate.clone();
        if fee_rate_options.custom().is_none() {
            if let Some(selected) = &selected_fee_rate {
                if let FeeSpeed::Custom { .. } = selected.fee_speed() {
                    fee_rate_options = fee_rate_options.add_custom_fee_rate(selected.clone());
                }
            }
        };

        // update the state
        let fee_rate_options_with_total_fee = Arc::new(fee_rate_options);
        state.write().fee_rate_options = Some(fee_rate_options_with_total_fee.clone());

        // if no fee rate is selected, then set the default to medium
        if self.state.read().selected_fee_rate.is_none() {
            let medium = Arc::new(fee_rate_options_with_total_fee.clone().medium);
            self.state.write().selected_fee_rate = Some(medium.clone());
            self.send(Message::UpdateFeeRate(medium));
        }

        let _ = sender.send(Message::UpdateFeeRateOptions(fee_rate_options_with_total_fee));
    }

    async fn get_wallet_balance(self: &Arc<Self>) {
        let balance = self.wallet_manager.balance().await;
        let wallet_balance = Arc::new(balance);
        self.state.write().wallet_balance = Some(wallet_balance.clone());
    }
}

// Listens for updates from the wallet manager
fn start_wallet_manager_listener(
    state: State,
    receiver: Arc<Receiver<WalletManagerReconcileMessage>>,
) -> JoinHandle<()> {
    type Message = WalletManagerReconcileMessage;

    task::spawn(async move {
        while let Ok(message) = receiver.recv_async().await {
            match message {
                Message::WalletMetadataChanged(metadata) => {
                    let mut state = state.write();
                    state.metadata = metadata;
                }
                Message::WalletBalanceChanged(balance) => {
                    let mut state = state.write();
                    state.wallet_balance = Some(balance);
                }
                _ => (),
            }
        }
    })
}

// Listens for updates from the app manager
fn start_app_manager_listener(
    state: State,
    receiver: Arc<Receiver<AppStateReconcileMessage>>,
) -> JoinHandle<()> {
    type Message = AppStateReconcileMessage;

    task::spawn(async move {
        while let Ok(message) = receiver.recv_async().await {
            match message {
                Message::FiatCurrencyChanged(currency) => {
                    let mut state = state.write();
                    state.selected_fiat_currency = currency;
                }
                Message::FiatPricesChanged(prices) => {
                    let fiat_currency = state.read().selected_fiat_currency;
                    state.write().btc_price_in_fiat = Some(prices.get_for_currency(fiat_currency));
                }
                _ => {}
            }
        }
    })
}

// on drop, stop all listeners
impl Drop for RustSendFlowManager {
    fn drop(&mut self) {
        self.state_listeners.drain(..).for_each(|handle| handle.abort());
    }
}
