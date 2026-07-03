use std::sync::Arc;

use cove_types::{WalletId, amount::Amount, unit::BitcoinUnit, utxo::Utxo};
use cove_util::format::NumberFormatter as _;
use tracing::{debug, warn};

use crate::app::AppAction;
use crate::manager::deferred_dispatch::DeferredDispatch;

use super::{EnterMode, Message, RustSendFlowManager, SendFlowAlertState, validation};

#[uniffi::export(async_runtime = "tokio")]
impl RustSendFlowManager {
    #[uniffi::method]
    pub fn wallet_id(&self) -> WalletId {
        self.state.lock().metadata.id.clone()
    }

    #[uniffi::method]
    pub fn amount(&self) -> Arc<Amount> {
        Arc::new(Amount::from_sat(self.amount_sats()))
    }

    #[uniffi::method]
    pub fn entering_fiat_amount(&self) -> String {
        self.state.lock().entering_fiat_amount.clone()
    }

    #[uniffi::method(name = "maxSendMinusFees")]
    pub fn ffi_max_send_minus_fees(&self) -> Option<Arc<Amount>> {
        self.max_send_minus_fees().map(Arc::new)
    }

    #[uniffi::method(name = "maxSendMinusFeesAndSmallUtxo")]
    pub fn ffi_max_send_minus_fees_and_small_utxo(&self) -> Option<Arc<Amount>> {
        self.max_send_minus_fees_and_small_utxo().map(Arc::new)
    }

    #[uniffi::method]
    pub fn utxos(&self) -> Option<Vec<Utxo>> {
        let mode = self.state.lock().mode.clone();
        match mode {
            EnterMode::CoinControl(cc) => Some(cc.utxo_list.utxos.clone()),
            _ => None,
        }
    }

    /// Wait until we have base fee rates, returns false if timeout
    /// Returns immediately if we already have cached fees
    /// Only blocks if no cached fees exist (first launch, network needed)
    /// On timeout: shows alert and pops route
    #[uniffi::method]
    pub async fn wait_for_init(self: &Arc<Self>) -> bool {
        let mut times = 0;
        const MAX_WAIT_MS: u64 = 20_000;
        let mut total_waited: u64 = 0;

        loop {
            if self.state.lock().has_base_fees {
                return true;
            }

            if total_waited >= MAX_WAIT_MS {
                warn!("wait_for_init timed out after {MAX_WAIT_MS}ms");

                self.reconciler.send(Message::SetAlert(SendFlowAlertState::General {
                    title: "Unable to Load Fees".to_string(),
                    message: "Cannot create a transaction without fee information. Please check your internet connection and try again.".to_string(),
                }));

                let mut deferred = DeferredDispatch::<AppAction>::new();
                deferred.queue(AppAction::PopRoute);

                return false;
            }

            debug!("waiting for base fees {times}");
            let wait_time = (33 + times * 10).min(200);
            tokio::time::sleep(std::time::Duration::from_millis(wait_time)).await;
            total_waited += wait_time;
            times += 1;
        }
    }

    #[uniffi::method]
    pub fn amount_sats(&self) -> u64 {
        self.state.lock().amount_sats.unwrap_or(0)
    }

    #[uniffi::method]
    pub fn amount_exceeds_balance(&self) -> bool {
        let state = self.state.lock();
        let total_fee_sats = state
            .fee_selection
            .as_ref()
            .and_then(|selection| selection.selected.total_fee.map(|fee| fee.as_sats()));

        validation::amount_exceeds_spendable_balance(
            state.amount_sats,
            total_fee_sats,
            state.unlocked_spendable_sats,
        )
    }

    #[uniffi::method]
    pub fn send_amount_btc(&self) -> String {
        let selected_unit = self.state.lock().metadata.selected_unit;
        let send_amount = self.send_amount().unwrap_or(Amount::ZERO);

        match selected_unit {
            BitcoinUnit::Btc => {
                let string = send_amount.as_btc().thousands();
                if string.contains('e') { send_amount.btc_string() } else { string.to_string() }
            }
            BitcoinUnit::Sat => send_amount.as_sats().thousands_int().to_string(),
        }
    }

    #[uniffi::method]
    pub fn send_amount_fiat(&self) -> String {
        let Some(btc_price_in_fiat) = self.state.lock().btc_price_in_fiat else {
            return "---".to_string();
        };

        let send_amount = self.send_amount().unwrap_or(Amount::ZERO);
        let send_amount_in_fiat = self
            .state
            .lock()
            .amount_fiat
            .unwrap_or_else(|| send_amount.as_btc().ceil() * (btc_price_in_fiat as f64));

        self.display_fiat_amount(send_amount_in_fiat, true).to_string()
    }

    #[uniffi::method]
    pub fn total_spent_in_btc(self: &Arc<Self>) -> String {
        if self.state.lock().amount_sats.is_none() {
            return "---".to_string();
        }

        let Some(total_spent) = self.total_spent_btc_amount() else {
            return "---".to_string();
        };

        match self.state.lock().metadata.selected_unit {
            BitcoinUnit::Btc => format!("{} BTC", total_spent.as_btc().thousands()),
            BitcoinUnit::Sat => format!("{} sats", total_spent.as_sats().thousands_int()),
        }
    }

    #[uniffi::method]
    pub fn total_spent_in_fiat(self: &Arc<Self>) -> String {
        if self.state.lock().amount_sats.is_none() {
            return "---".to_string();
        }

        let Some(total_spent) = self.total_spent_btc_amount() else {
            return "---".to_string();
        };

        let Some(btc_price_in_fiat) = self.state.lock().btc_price_in_fiat else {
            return "---".to_string();
        };

        let total_spent_in_fiat = total_spent.as_btc() * (btc_price_in_fiat as f64);
        format!("≈ {}", self.display_fiat_amount(total_spent_in_fiat, true))
    }

    #[uniffi::method]
    pub fn total_fee_string(&self) -> Option<String> {
        let selected_fee_rate = self.selected_fee_rate()?;
        let total_fee = selected_fee_rate.total_fee()?;

        let string = match self.state.lock().metadata.selected_unit {
            BitcoinUnit::Btc => format!("{} BTC", total_fee.as_btc().thousands()),
            BitcoinUnit::Sat => format!("{} sats", total_fee.as_sats().thousands_int()),
        };

        Some(string)
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
}
