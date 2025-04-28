pub mod btc_on_change;
pub mod fiat_on_change;

use std::sync::Arc;

use crate::{
    app::{App, reconcile::AppStateReconcileMessage},
    database::Database,
    fee_client::FEE_CLIENT,
    fiat::FiatCurrency,
    task,
    wallet::{Address, metadata::WalletMetadata},
};
use act_zero::{WeakAddr, call};
use btc_on_change::BtcOnChangeHandler;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
};
use fiat_on_change::FiatOnChangeHandler;
use flume::{Receiver, Sender};
use parking_lot::RwLock;
use tokio::task::JoinHandle;
use tracing::error;

use super::wallet::{WalletManagerReconcileMessage, actor::WalletActor};

type Action = SendFlowManagerAction;
type Message = SendFlowManagerReconcileMessage;
type State = SendFlowManagerState;
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
    wallet_actor: WeakAddr<WalletActor>,

    state_listeners: Vec<JoinHandle<()>>,
    pub state: Arc<RwLock<State>>,

    reconciler: Sender<Message>,
    reconcile_receiver: Arc<Receiver<Message>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SendFlowManagerState {
    // private
    metadata: WalletMetadata,
    fee_rate_options_base: Option<Arc<FeeRateOptions>>,
    btc_price_in_fiat: Option<f64>,
    selected_fiat_currency: FiatCurrency,
    first_address: Option<Arc<Address>>,

    // public
    pub entering_btc_amount: String,
    pub entering_fiat_amount: String,

    pub amount_sats: Option<u64>,
    pub amount_fiat: Option<f64>,

    pub max_selected: Option<Arc<Amount>>,

    pub address: Option<String>,
    pub focus_field: Option<SetAmountFocusField>,

    pub selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    pub fee_rate_options: Option<Arc<FeeRateOptionsWithTotalFee>>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerReconcileMessage {
    // reconcile state with swift
    UpdateEnteringBtcAmount(String),
    UpdateEnteringFiatAmount(String),
    UpdateMaxSelected(Option<Arc<Amount>>),

    UpdateAmountSats(u64),
    UpdateAmountFiat(f64),

    UpdateFocusField(Option<SetAmountFocusField>),
    UpdateFeeRate(Arc<FeeRateOptionWithTotalFee>),

    UpdateSelectedFeeRate(Arc<FeeRateOptionWithTotalFee>),
    UpdateFeeRateOptions(Arc<FeeRateOptionsWithTotalFee>),

    // side effects
    SetAlert { title: String, message: String },
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerAction {
    ChangeEnteringBtcAmount(String),
    ChangeEnteringFiatAmount(String),
    ChangeSetAmountFocusField(Option<SetAmountFocusField>),

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),
    ChangeAddress(String),
}

impl RustSendFlowManager {
    pub fn new(
        metadata: WalletMetadata,
        wallet_actor: WeakAddr<WalletActor>,
        wallet_manager_receiver: Arc<Receiver<WalletManagerReconcileMessage>>,
    ) -> Self {
        let (sender, receiver) = flume::bounded(100);

        let state = Arc::new(RwLock::new(State::new(metadata)));

        let manager_listeners = {
            let wallet_manager_listener =
                start_wallet_manager_listener(state.clone(), wallet_manager_receiver);

            let app_listener = start_app_manager_listener(state.clone(), App::global().receiver());

            vec![wallet_manager_listener, app_listener]
        };

        // in background run init tasks
        background_init_tasks(state.clone(), wallet_actor.clone(), sender.clone());

        Self {
            app: App::global().clone(),
            state,
            wallet_actor,
            state_listeners: manager_listeners,
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
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

    #[uniffi::method]
    pub fn amount(&self) -> Arc<Amount> {
        Arc::new(Amount::from_sat(self.amount_sats()))
    }

    #[uniffi::method]
    pub fn amount_sats(&self) -> u64 {
        self.state.read().amount_sats.unwrap_or(0)
    }

    /// action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) {
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
            Action::ChangeSetAmountFocusField(set_amount_focus_field) => {
                self.state.write().focus_field = set_amount_focus_field;
            }
            Action::SelectFeeRate(fee_rate) => {
                self.state.write().selected_fee_rate = Some(fee_rate);
            }
            Action::ChangeAddress(address) => {
                self.state.write().address = Some(address);
            }
        }
    }
}

impl RustSendFlowManager {
    fn btc_field_changed(&self, old_value: String, new_value: String) -> Option<()> {
        let wallet_actor = self.wallet_actor.clone();
        let state = self.state.clone();
        let sender = self.reconciler.clone();

        if self.state.read().fee_rate_options.is_none() {
            crate::task::spawn(async move {
                get_fee_rate_options(state, wallet_actor, sender).await;
            });
        }

        let state = self.state.clone();
        let handler = BtcOnChangeHandler::new(state.clone());
        let changes: btc_on_change::Changeset = handler.on_change(&old_value, &new_value);

        let mut state = state.write();

        if let Some(max) = changes.max_selected {
            state.max_selected = max.map(Arc::new);
            self.send(Message::UpdateMaxSelected(state.max_selected.clone()));
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

    fn send(&self, message: SendFlowManagerReconcileMessage) {
        if let Err(err) = self.reconciler.send(message) {
            error!("unable to send message to send flow manager: {err}");
        }
    }
}

async fn get_fee_rate_options(
    state: Arc<RwLock<State>>,
    wallet_actor: WeakAddr<WalletActor>,
    sender: Sender<SendFlowManagerReconcileMessage>,
) {
    let address = state.read().address.clone();
    let network = state.read().metadata.network;
    let amount_sats = state.read().amount_sats;

    let fee_rate_options_base = {
        let fee_rate_options_base = state.read().fee_rate_options_base.clone();
        let fee_rate_options_base = match fee_rate_options_base {
            Some(fee_rate_options_base) => Some(fee_rate_options_base),
            None => get_and_update_base_fee_rate_options(state.clone()).await,
        };

        match fee_rate_options_base {
            Some(fee_rate_options_base) => Arc::unwrap_or_clone(fee_rate_options_base),
            None => return,
        }
    };

    let first_address = state.read().first_address.clone();
    if first_address.is_none() {
        let _ = get_first_address(state.clone(), wallet_actor.clone()).await;
    }

    let address = address.and_then(|addr| Address::from_string(addr, network).ok());

    let address = match (address, first_address) {
        (Some(address), _) => address,
        (None, Some(first_address)) => Arc::unwrap_or_clone(first_address),
        _ => return,
    };

    let amount_sats = amount_sats.unwrap_or(10_000);
    let amount = Amount::from_sat(amount_sats);

    let fee_rate_options = call!(wallet_actor.fee_rate_options_with_total_fee(
        fee_rate_options_base,
        amount.into(),
        address
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
    let _ = sender.send(Message::UpdateFeeRateOptions(
        fee_rate_options_with_total_fee,
    ));
}

async fn get_and_update_base_fee_rate_options(
    state: Arc<RwLock<State>>,
) -> Option<Arc<FeeRateOptions>> {
    let fee_response = FEE_CLIENT.fetch_and_get_fees().await.ok()?;
    let fees = Arc::new(FeeRateOptions::from(fee_response));

    state.write().fee_rate_options_base = Some(fees.clone());

    Some(fees)
}

// Listens for updates from the wallet manager
fn start_wallet_manager_listener(
    state: Arc<RwLock<State>>,
    receiver: Arc<Receiver<WalletManagerReconcileMessage>>,
) -> JoinHandle<()> {
    type Message = WalletManagerReconcileMessage;

    task::spawn(async move {
        while let Ok(message) = receiver.recv_async().await {
            if let Message::WalletMetadataChanged(metadata) = message {
                let mut state = state.write();
                state.metadata = metadata;
            }
        }
    })
}

// Listens for updates from the app manager
fn start_app_manager_listener(
    state: Arc<RwLock<State>>,
    receiver: Arc<Receiver<AppStateReconcileMessage>>,
) -> JoinHandle<()> {
    type Message = AppStateReconcileMessage;

    task::spawn(async move {
        while let Ok(message) = receiver.recv_async().await {
            if let Message::FiatCurrencyChanged(currency) = message {
                let mut state = state.write();
                state.selected_fiat_currency = currency;
            }
        }
    })
}

// Get the first address for the wallet
// Get the fee rate options
fn background_init_tasks(
    state: Arc<RwLock<State>>,
    actor: WeakAddr<WalletActor>,
    sender: Sender<Message>,
) {
    task::spawn(async move {
        // get and save first address
        get_first_address(state.clone(), actor.clone()).await;

        // get fee rate options
        get_fee_rate_options(state, actor, sender).await;
    });
}

async fn get_first_address(state: Arc<RwLock<State>>, actor: WeakAddr<WalletActor>) {
    if let Ok(first_address) = call!(actor.address_at(0)).await {
        let address = first_address.address.clone().into();
        state.write().first_address = Some(Arc::new(address));
    }
}

impl State {
    pub fn new(metadata: WalletMetadata) -> Self {
        Self {
            metadata,
            fee_rate_options_base: None,
            entering_btc_amount: String::new(),
            entering_fiat_amount: String::new(),
            first_address: None,
            amount_sats: None,
            amount_fiat: None,
            max_selected: None,
            focus_field: None,
            address: None,
            selected_fee_rate: None,
            fee_rate_options: None,
            btc_price_in_fiat: None,
            selected_fiat_currency: Database::global()
                .global_config
                .fiat_currency()
                .unwrap_or_default(),
        }
    }
}

// on drop, stop all listeners
impl Drop for RustSendFlowManager {
    fn drop(&mut self) {
        self.state_listeners
            .drain(..)
            .for_each(|handle| handle.abort());
    }
}
