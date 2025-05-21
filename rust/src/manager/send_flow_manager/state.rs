use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
};

use super::SetAmountFocusField;
use crate::{
    app::App,
    database::Database,
    fiat::FiatCurrency,
    wallet::{Address, balance::Balance, metadata::WalletMetadata},
};

#[derive(Debug, Clone, derive_more::Deref)]
pub struct State(Arc<Mutex<SendFlowManagerState>>);

#[derive(Clone, Debug, uniffi::Object)]
pub struct SendFlowManagerState {
    // private
    pub(crate) metadata: WalletMetadata,
    pub(crate) fee_rate_options_base: Option<Arc<FeeRateOptions>>,
    pub(crate) btc_price_in_fiat: Option<u64>,
    pub(crate) selected_fiat_currency: FiatCurrency,
    pub(crate) first_address: Option<Arc<Address>>,
    pub(crate) wallet_balance: Option<Arc<Balance>>,
    pub(crate) init_complete: bool,
    pub(crate) enter_type: EnterType,

    // public
    pub entering_btc_amount: String,
    pub entering_fiat_amount: String,
    pub entering_address: String,

    pub amount_sats: Option<u64>,
    pub amount_fiat: Option<f64>,

    pub max_selected: Option<Arc<Amount>>,

    pub address: Option<Arc<Address>>,
    pub focus_field: Option<SetAmountFocusField>,

    pub selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    pub fee_rate_options: Option<Arc<FeeRateOptionsWithTotalFee>>,
}

#[derive(Debug, Default, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum EnterType {
    #[default]
    SetAmount,
    CoinControl(Arc<UtxoTotal>),
}

impl EnterType {
    pub fn is_coin_control(&self) -> bool {
        matches!(self, Self::CoinControl(_))
    }
}

type UtxoTotal = Amount;

/// MARK: State
impl State {
    pub fn new(metadata: WalletMetadata) -> Self {
        Self(Arc::new(Mutex::new(SendFlowManagerState::new(metadata))))
    }

    pub fn into_inner(self) -> Arc<Mutex<SendFlowManagerState>> {
        self.0
    }

    pub fn lock(&self) -> MutexGuard<'_, SendFlowManagerState> {
        self.0.lock()
    }
}

/// MARK: SendFlowManagerState
impl SendFlowManagerState {
    pub fn new(metadata: WalletMetadata) -> Self {
        let selected_fiat_currency =
            Database::global().global_config.fiat_currency().unwrap_or_default();

        let btc_price_in_fiat = App::global().prices().map(|prices| prices.get());

        Self {
            metadata,
            fee_rate_options_base: None,
            entering_btc_amount: String::new(),
            entering_fiat_amount: selected_fiat_currency.symbol().to_string(),
            entering_address: String::new(),
            enter_type: EnterType::SetAmount,
            first_address: None,
            amount_sats: None,
            amount_fiat: None,
            max_selected: None,
            focus_field: None,
            address: None,
            selected_fee_rate: None,
            wallet_balance: None,
            fee_rate_options: None,
            btc_price_in_fiat,
            selected_fiat_currency,
            init_complete: false,
        }
    }
}

impl From<SendFlowManagerState> for State {
    fn from(state: SendFlowManagerState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }
}

impl From<Arc<Mutex<SendFlowManagerState>>> for State {
    fn from(state: Arc<Mutex<SendFlowManagerState>>) -> Self {
        Self(state)
    }
}
