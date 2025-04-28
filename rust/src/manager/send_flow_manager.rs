pub mod btc_on_change;
pub mod fiat_on_change;

use std::sync::Arc;

use crate::{
    app::{App, reconcile::AppStateReconcileMessage},
    database::Database,
    fiat::FiatCurrency,
    task,
    wallet::metadata::WalletMetadata,
};
use act_zero::WeakAddr;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
};
use fiat_on_change::FiatOnChangeHandler;
use flume::{Receiver, Sender};
use parking_lot::RwLock;
use tokio::task::JoinHandle;

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

    // public
    pub entering_btc_amount: String,
    pub entering_fiat_amount: String,

    pub amount_sats: u64,
    pub amount_fiat: f64,

    pub max_selected: Option<Arc<Amount>>,
    pub focus_field: Option<SetAmountFocusField>,

    pub selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    pub fee_rate_options: Option<Arc<FeeRateOptionsWithTotalFee>>,
}

#[derive(Debug, Copy, Clone, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerReconcileMessage {
    UpdateAmountSats(u64),
    UpdateAmountFiat(f64),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowManagerAction {
    ChangeEnteringBtcAmount(String),
    ChangeEnteringFiatAmount(String),
    ChangeSetAmountFocusField(Option<SetAmountFocusField>),

    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),
}

impl RustSendFlowManager {
    pub fn new(
        metadata: WalletMetadata,
        wallet_manager: WeakAddr<WalletActor>,
        wallet_manager_receiver: Arc<Receiver<WalletManagerReconcileMessage>>,
    ) -> Self {
        let (sender, receiver) = flume::bounded(1000);

        let state = Arc::new(RwLock::new(State::new(metadata)));

        let manager_listeners = {
            let wallet_manager_listener =
                start_wallet_manager_listener(state.clone(), wallet_manager_receiver);

            let app_listener = start_app_manager_listener(state.clone(), App::global().receiver());

            vec![wallet_manager_listener, app_listener]
        };

        Self {
            app: App::global().clone(),
            state,
            wallet_actor: wallet_manager,
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
        let amount = self.state.read().amount_sats;
        Arc::new(Amount::from_sat(amount))
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
        }
    }
}

impl RustSendFlowManager {
    fn btc_field_changed(&self, old_value: String, new_value: String) -> Option<()> {
        let state = self.state.clone();
        if self.state.read().fee_rate_options.is_none() {
            crate::task::spawn(async move {
                get_fee_rate_options(state).await;
            });
        }

        todo!()
    }

    async fn get_fee_rate_options(&self) {
        todo!()
    }

    fn fiat_field_changed(&self, old_value: String, new_value: String) -> Option<()> {
        let prices = self.app.prices()?;
        let selected_currency = Database::global()
            .global_config
            .fiat_currency()
            .unwrap_or_default();

        let handler = FiatOnChangeHandler::new(prices, selected_currency);
        let Ok(result) = handler.on_change(old_value, new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        if let Some(fiat_text) = result.fiat_text {
            self.state.write().entering_fiat_amount = fiat_text;
        }

        if let Some(fiat_value) = result.fiat_value {
            self.state.write().amount_fiat = fiat_value;
        }

        if let Some(btc_amount) = result.btc_amount {
            self.state.write().amount_sats = btc_amount.as_sats();
        }

        Some(())
    }
}

async fn get_fee_rate_options(state: Arc<RwLock<State>>) {
    todo!()
}

// Listens for updates from the wallet manager
fn start_wallet_manager_listener(
    state: Arc<RwLock<State>>,
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

                _ => {}
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
            match message {
                Message::FiatCurrencyChanged(currency) => {
                    let mut state = state.write();
                    state.selected_fiat_currency = currency;
                }

                _ => {}
            }
        }
    })
}

impl State {
    pub fn new(metadata: WalletMetadata) -> Self {
        Self {
            metadata,
            fee_rate_options_base: None,
            entering_btc_amount: String::new(),
            entering_fiat_amount: String::new(),
            amount_sats: 0,
            amount_fiat: 0.0,
            max_selected: None,
            focus_field: None,
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
