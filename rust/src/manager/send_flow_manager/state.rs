use std::sync::Arc;

use parking_lot::Mutex;

use cove_types::{
    Network, WalletId,
    amount::Amount,
    fees::{FeeRate, FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
    unit::BitcoinUnit,
    utxo::UtxoList,
};
use tracing::warn;

use super::{SendFlowError, SendFlowWarningKind, SetAmountFocusField};
use crate::{
    app::App,
    database::Database,
    fiat::FiatCurrency,
    manager::wallet_manager::live_metadata::WalletDisplayPreferences,
    transfer_destination::TransferRecipient,
    wallet::{
        Address,
        balance::Balance,
        metadata::{FiatOrBtc, WalletMetadata, WalletType},
    },
};

#[derive(Debug, Clone, derive_more::Deref)]
pub struct State(Arc<Mutex<SendFlowManagerState>>);

#[derive(Clone, Debug, uniffi::Object)]
pub struct SendFlowManagerState {
    // private
    pub(crate) wallet: SendFlowWallet,
    pub(crate) amount_entry: AmountEntry,
    pub(crate) fee_rate_options_base: Option<Arc<FeeRateOptions>>,
    pub(crate) btc_price_in_fiat: Option<u64>,
    pub(crate) selected_fiat_currency: FiatCurrency,
    pub(crate) first_address: Option<Arc<Address>>,
    pub(crate) wallet_balance: Option<Arc<Balance>>,
    pub(crate) unlocked_spendable_sats: Option<u64>,
    pub(crate) lock_state_load_failed: bool,
    pub(crate) acknowledged_warnings: Vec<SendFlowWarningKind>,
    /// True once we have base fee rates (either from cache or network)
    /// UI can show immediately once this is true (total fees pending calculation)
    pub(crate) has_base_fees: bool,
    pub(crate) mode: EnterMode,
    /// Last fee update failure, kept so recalculations on every edit log only when it changes
    pub(crate) last_fee_failure: Option<String>,
    /// The amount, in sats, of an alerting check that found no fee total and runs again once one
    /// lands, see `AmountAlert::ShownOnceFeeLoads`
    pub(crate) amount_check_awaiting_fee: Option<u64>,
    /// The picked wallet this send pays, kept only while the address is the one it handed out
    pub(crate) transfer_recipient: Option<TransferRecipient>,
    /// Counts the addresses put in the send, so a picked wallet's address that lands after a
    /// newer one is dropped
    pub(crate) address_generation: u64,

    // public
    pub entering_address: String,

    pub amount_sats: Option<u64>,
    pub amount_fiat: Option<f64>,

    pub max_selected: Option<MaxSelection>,

    pub address: Option<Arc<Address>>,
    pub focus_field: Option<SetAmountFocusField>,

    pub fee_selection: Option<FeeSelection>,

    pub payjoin_endpoint: Option<String>,
}

/// The wallet a send flow spends from, fixed for the flow's lifetime
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendFlowWallet {
    pub id: WalletId,
    pub network: Network,
    pub wallet_type: WalletType,
}

/// The amount as the user enters it: the unit and field it is typed in, and the text in each field
///
/// Parsing and rendering both read it, and a display preference change rewrites the text in the
/// same step as the mode. An edit names the mode it was typed in and is checked, parsed and
/// committed in one step too, so the text is never parsed in a unit it was not written in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmountEntry {
    pub mode: AmountEntryMode,
    pub btc_text: String,
    pub fiat_text: String,
}

/// A Max amount and the drain it was built from
#[derive(Debug, Clone, PartialEq)]
pub struct MaxSelection {
    pub amount: Arc<Amount>,

    /// The fee the drain pays, which with `amount` spends every coin it drew on
    pub fee: Amount,
    pub request: MaxSendRequest,
}

/// What a Max amount depends on: a drain of the wallet to this address at this fee rate
#[derive(Debug, Clone, PartialEq)]
pub struct MaxSendRequest {
    pub address: Address,
    pub fee_rate: FeeRate,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct FeeSelection {
    pub options: Arc<FeeRateOptionsWithTotalFee>,
    pub selected: Arc<FeeRateOptionWithTotalFee>,
}

// define types this way so uniffi generates unique named types but the rust code just uses the abbreviated name
pub use internal::SendFlowAmountEntryMode as AmountEntryMode;
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

    /// The unit bitcoin amounts are shown in and whether the amount is typed in fiat or bitcoin
    #[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
    pub struct SendFlowAmountEntryMode {
        pub unit: BitcoinUnit,
        pub fiat_or_btc: FiatOrBtc,
    }

    #[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
    pub struct SendFlowCoinControlMode {
        pub utxo_list: Arc<UtxoList>,
        pub is_max_selected: bool,
    }
}

impl EnterMode {
    pub fn coin_control_max(utxos: impl Into<Arc<UtxoList>>) -> Self {
        Self::CoinControl(CoinControlMode::new(utxos, true))
    }

    pub const fn is_coin_control(&self) -> bool {
        matches!(self, Self::CoinControl(_))
    }
}

impl From<WalletDisplayPreferences> for AmountEntryMode {
    fn from(preferences: WalletDisplayPreferences) -> Self {
        Self { unit: preferences.selected_unit, fiat_or_btc: preferences.fiat_or_btc }
    }
}

impl From<&WalletMetadata> for SendFlowWallet {
    fn from(metadata: &WalletMetadata) -> Self {
        Self {
            id: metadata.id.clone(),
            network: metadata.network,
            wallet_type: metadata.wallet_type,
        }
    }
}

impl AmountEntry {
    fn new(mode: AmountEntryMode, fiat_text: String) -> Self {
        Self { mode, btc_text: String::new(), fiat_text }
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
    pub fn new(wallet: SendFlowWallet, mode: AmountEntryMode, balance: Arc<Balance>) -> Self {
        Self(Arc::new(Mutex::new(SendFlowManagerState::new(wallet, mode, balance))))
    }

    pub fn into_inner(self) -> Arc<Mutex<SendFlowManagerState>> {
        self.0
    }
}

/// MARK: SendFlowManagerState
impl SendFlowManagerState {
    pub fn new(wallet: SendFlowWallet, mode: AmountEntryMode, balance: Arc<Balance>) -> Self {
        let selected_fiat_currency = Database::global().global_config.selected_fiat_currency();

        let btc_price_in_fiat = App::global().prices().map(|prices| prices.get());

        Self {
            wallet,
            amount_entry: AmountEntry::new(mode, selected_fiat_currency.symbol().to_string()),
            fee_rate_options_base: None,
            entering_address: String::new(),
            mode: EnterMode::SetAmount,
            first_address: None,
            amount_sats: None,
            amount_fiat: None,
            max_selected: None,
            focus_field: None,
            address: None,
            wallet_balance: Some(balance),
            unlocked_spendable_sats: None,
            lock_state_load_failed: false,
            acknowledged_warnings: Vec::new(),
            fee_selection: None,
            btc_price_in_fiat,
            selected_fiat_currency,
            has_base_fees: false,
            last_fee_failure: None,
            amount_check_awaiting_fee: None,
            transfer_recipient: None,
            address_generation: 0,
            payjoin_endpoint: None,
        }
    }

    /// The drain a Max amount needs now: the entered address, or the wallet's first address
    /// before one is entered, at the selected fee rate
    pub(crate) fn max_send_request(&self) -> Result<MaxSendRequest, SendFlowError> {
        let address = self
            .address
            .as_deref()
            .cloned()
            .or_else(|| Address::from_string(&self.entering_address, self.wallet.network).ok())
            .or_else(|| self.first_address.as_deref().cloned())
            .ok_or_else(|| SendFlowError::InvalidAddress(self.entering_address.clone()))?;

        // the selected rate, or the medium base rate, or a default of 50 sat/vB
        let fee_rate = self
            .fee_selection
            .as_ref()
            .map(|selection| selection.selected.fee_rate)
            .or_else(|| self.fee_rate_options_base.as_ref().map(|base| base.medium.fee_rate))
            .unwrap_or_else(|| {
                warn!(
                    "unable to get selected fee rate or base fee rate using default of 50 sat/vb"
                );
                FeeRate::from_sat_per_vb(50.0)
            });

        Ok(MaxSendRequest { address, fee_rate })
    }

    /// Whether Max is selected but its amount was drained for another address or fee rate
    pub(crate) fn max_is_stale(&self) -> bool {
        let Some(max) = &self.max_selected else {
            return false;
        };

        // an address that no longer resolves is for address validation to report
        self.max_send_request().is_ok_and(|request| request != max.request)
    }

    /// The fee of the drain behind the amount, while the amount is a selected Max whose drain
    /// still answers the current address and fee rate
    ///
    /// Balance checks price such an amount with this fee rather than the fee selection's total:
    /// committing a drain schedules a fee update, and until it lands the selection can still hold
    /// a total priced for the earlier amount, which would put Max over the balance it was drawn from
    pub(crate) fn current_max_fee(&self) -> Option<Amount> {
        let max = self.max_selected.as_ref()?;
        if self.amount_sats != Some(max.amount.to_sat()) {
            return None;
        }

        let is_current = self.max_send_request().is_ok_and(|request| request == max.request);
        is_current.then_some(max.fee)
    }

    /// The total fee to check the amount against the balance with: a current Max's drain fee, or
    /// else the selected fee rate's total, which is zero before any fee rate is selected and
    /// missing while it is priced
    pub(crate) fn balance_check_fee_sats(&self) -> Option<u64> {
        if let Some(fee) = self.current_max_fee() {
            return Some(fee.to_sat());
        }

        match self.fee_selection.as_ref() {
            Some(selection) => selection.selected.total_fee.map(|fee| fee.as_sats()),
            None => Some(0),
        }
    }

    pub(crate) fn acknowledge_warning(&mut self, kind: SendFlowWarningKind) {
        if !self.has_acknowledged_warning(kind) {
            self.acknowledged_warnings.push(kind);
        }
    }

    pub(crate) fn has_acknowledged_warning(&self, kind: SendFlowWarningKind) -> bool {
        self.acknowledged_warnings.contains(&kind)
    }

    pub(crate) fn clear_warning_acknowledgements(&mut self) {
        self.acknowledged_warnings.clear();
    }

    /// Sets the send's recipient address, and drops a picked wallet that did not hand it out
    ///
    /// Every change of the address goes through here, so a picked wallet never outlives its
    /// address. Returns whether a picked wallet was dropped, so the caller tells the apps
    #[must_use]
    pub(crate) fn set_address(
        &mut self,
        address: Option<Arc<Address>>,
        payjoin_endpoint: Option<String>,
    ) -> RecipientDropped {
        if self.address != address {
            self.clear_warning_acknowledgements();
        }

        self.address = address;
        self.payjoin_endpoint = payjoin_endpoint;

        let still_paid = self
            .transfer_recipient
            .as_ref()
            .is_some_and(|recipient| self.address.as_ref() == Some(&recipient.address));

        if still_paid || self.transfer_recipient.is_none() {
            return RecipientDropped::No;
        }

        self.transfer_recipient = None;
        RecipientDropped::Yes
    }
}

/// Whether setting the address dropped the picked wallet, which the apps then need to hear
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecipientDropped {
    Yes,
    No,
}

impl FeeSelection {
    pub fn new(
        options: impl Into<Arc<FeeRateOptionsWithTotalFee>>,
        selected: impl Into<Arc<FeeRateOptionWithTotalFee>>,
    ) -> Self {
        Self { options: options.into(), selected: selected.into() }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bitcoin::Amount as BdkAmount;

    use super::*;

    #[test]
    fn new_starts_with_unknown_unlocked_spendable_balance() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::init_test_database();

        let metadata = WalletMetadata::preview_new();
        let balance = Arc::new(Balance(bdk_wallet::Balance {
            confirmed: BdkAmount::from_sat(50_000),
            ..Default::default()
        }));

        let mode =
            AmountEntryMode { unit: metadata.selected_unit, fiat_or_btc: metadata.fiat_or_btc };
        let state = SendFlowManagerState::new(SendFlowWallet::from(&metadata), mode, balance);

        assert_eq!(state.unlocked_spendable_sats, None);
        assert!(!state.lock_state_load_failed);
    }
}
