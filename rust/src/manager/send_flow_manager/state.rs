use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
};

use super::SetAmountFocusField;
use crate::{
    database::Database,
    fiat::FiatCurrency,
    wallet::{Address, metadata::WalletMetadata},
};

#[derive(Debug, Clone, derive_more::Deref)]
pub struct State(Arc<RwLock<SendFlowManagerState>>);

#[derive(Clone, Debug, uniffi::Record)]
pub struct SendFlowManagerState {
    // private
    pub(crate) metadata: WalletMetadata,
    pub(crate) fee_rate_options_base: Option<Arc<FeeRateOptions>>,
    pub(crate) btc_price_in_fiat: Option<f64>,
    pub(crate) selected_fiat_currency: FiatCurrency,
    pub(crate) first_address: Option<Arc<Address>>,

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

/// MARK: State
impl State {
    pub fn new(metadata: WalletMetadata) -> Self {
        Self(Arc::new(RwLock::new(SendFlowManagerState::new(metadata))))
    }

    pub fn into_inner(self) -> Arc<RwLock<SendFlowManagerState>> {
        self.0
    }

    pub fn read(&self) -> RwLockReadGuard<'_, SendFlowManagerState> {
        self.0.read()
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, SendFlowManagerState> {
        self.0.write()
    }
}

/// MARK: SendFlowManagerState
impl SendFlowManagerState {
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

impl From<SendFlowManagerState> for State {
    fn from(state: SendFlowManagerState) -> Self {
        Self(Arc::new(RwLock::new(state)))
    }
}

impl From<Arc<RwLock<SendFlowManagerState>>> for State {
    fn from(state: Arc<RwLock<SendFlowManagerState>>) -> Self {
        Self(state)
    }
}
