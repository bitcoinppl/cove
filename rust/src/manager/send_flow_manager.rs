use std::sync::Arc;

use act_zero::WeakAddr;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions},
    unit::Unit,
};
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{fiat::FiatCurrency, wallet::metadata::FiatOrBtc};

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
    wallet_actor: WeakAddr<WalletActor>,
    pub state: Arc<RwLock<State>>,

    pub reconciler: Sender<Message>,
    pub reconcile_receiver: Arc<Receiver<Message>>,
    pub wallet_manager_listener: Arc<Receiver<WalletManagerReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SendFlowManagerState {
    // private
    mode: FiatOrBtc,
    unit: Unit,
    fiat_currency: FiatCurrency,
    fee_rate_options_base: Option<Arc<FeeRateOptions>>,

    // public
    pub entering_btc_amount: String,
    pub entering_fiat_amount: String,

    pub amount_sats: u64,
    pub amount_fiat: f64,

    pub max_selected: Option<Arc<Amount>>,
    pub set_amount_focus_field: Option<SetAmountFocusField>,

    pub selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    pub fee_rate_options: Option<Arc<FeeRateOptionWithTotalFee>>,
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
        wallet_manager: WeakAddr<WalletActor>,
        wallet_manager_listener: Arc<Receiver<WalletManagerReconcileMessage>>,
    ) -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            wallet_actor: wallet_manager,
            state: Arc::new(RwLock::new(SendFlowManagerState::new())),
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
                self.state.write().entering_btc_amount = string;
            }
            Action::ChangeEnteringFiatAmount(string) => {
                self.state.write().entering_fiat_amount = string;
            }
            Action::ChangeSetAmountFocusField(set_amount_focus_field) => {
                self.state.write().set_amount_focus_field = set_amount_focus_field;
            }
            Action::SelectFeeRate(fee_rate) => {
                self.state.write().selected_fee_rate = Some(fee_rate);
            }
        }
    }
}

impl State {
    pub fn new() -> Self {
        todo!()
        // Self {}
    }
}
