use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
    utxo::UtxoList,
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
    pub(crate) mode: EnterMode,

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

// define types this way so uniffi generates unique named types but the rust code just uses the abbreviated name
pub use internal::SendFlowCoinControlMode as CoinControlMode;
pub use internal::SendFlowEnterMode as EnterMode;

mod internal {
    use super::*;

    #[derive(Debug, Default, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
    pub enum SendFlowEnterMode {
        #[default]
        SetAmount,
        CoinControl(SendFlowCoinControlMode),
    }

    #[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
    pub struct SendFlowCoinControlMode {
        pub utxo_list: Arc<UtxoList>,
        pub is_max_selected: bool,
    }
}

impl EnterMode {
    pub fn coin_control(utxos: impl Into<Arc<UtxoList>>) -> Self {
        Self::CoinControl(CoinControlMode::new(utxos, true))
    }

    pub fn is_coin_control(&self) -> bool {
        matches!(self, Self::CoinControl(_))
    }
}

impl CoinControlMode {
    pub fn new(utxo_list: impl Into<Arc<UtxoList>>, is_max_selected: bool) -> Self {
        Self { utxo_list: utxo_list.into(), is_max_selected }
    }

    pub fn max_send(&self) -> Amount {
        self.utxo_list.total
    }

    pub fn utxo_list(&self) -> Arc<UtxoList> {
        self.utxo_list.clone()
    }

    pub fn outpoints(&self) -> Vec<bitcoin::OutPoint> {
        self.utxo_list.outpoints()
    }
}

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
            mode: EnterMode::SetAmount,
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
