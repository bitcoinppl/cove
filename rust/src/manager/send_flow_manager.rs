pub mod fiat_on_change;

use std::sync::Arc;

use crate::{app::App, wallet::metadata::WalletMetadata};
use act_zero::WeakAddr;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
};
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

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

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustSendFlowManager {
    app: App,
    wallet_actor: WeakAddr<WalletActor>,

    pub state: Arc<RwLock<State>>,

    reconciler: Sender<Message>,
    reconcile_receiver: Arc<Receiver<Message>>,
    wallet_manager_listener: Arc<Receiver<WalletManagerReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SendFlowManagerState {
    // private
    metadata: WalletMetadata,

    fee_rate_options_base: Option<Arc<FeeRateOptions>>,

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
        wallet_manager_listener: Arc<Receiver<WalletManagerReconcileMessage>>,
    ) -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            app: App::global().clone(),
            wallet_actor: wallet_manager,
            state: Arc::new(RwLock::new(State::new(metadata))),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            wallet_manager_listener,
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
                self.btc_field_changed(string);
            }
            Action::ChangeEnteringFiatAmount(string) => {
                self.state.write().entering_fiat_amount = string;
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
    fn btc_field_changed(&self, amount: String) -> Option<()> {
        let prices = self.app.prices();
        None
    }
}

fn handle_wallet_manager_updates(
    state: Arc<RwLock<State>>,
    receiver: Receiver<WalletManagerReconcileMessage>,
) {
    type Message = WalletManagerReconcileMessage;

    std::thread::spawn(move || {
        while let Ok(message) = receiver.recv() {
            match message {
                Message::WalletMetadataChanged(metadata) => {
                    let mut state = state.write();
                    state.metadata = metadata;
                }

                _ => {}
            }
        }
    });
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
        }
    }
}
