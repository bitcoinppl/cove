use std::sync::Arc;

use crate::{
    database::Database,
    fiat::FiatCurrency,
    wallet::metadata::{FiatOrBtc, WalletMetadata},
};
use cove_types::{amount::Amount, fees::FeeRateOptionsWithTotalFee, unit::Unit};
use cove_util::format::NumberFormatter as _;
use parking_lot::RwLock;

use super::State;

#[derive(Debug, Clone)]
pub struct BtcOnChangeHandler {
    metadata: WalletMetadata,
    fee_rate_options: Option<FeeRateOptionsWithTotalFee>,
    max_selected: Option<Amount>,
    btc_price_in_fiat: Option<f64>,
    selected_fiat_currency: FiatCurrency,
}

#[derive(Debug, Default)]
pub struct Changeset {
    pub entering_amount_btc: Option<String>,
    pub amount_btc: Option<Amount>,
    pub amount_fiat: Option<String>,
    pub max_selected: Option<Option<Amount>>,
}

impl BtcOnChangeHandler {
    pub fn new(state: Arc<RwLock<State>>) -> Self {
        let state = state.read();

        let metadata = state.metadata.clone();
        let fee_rate_options = state.fee_rate_options.clone();
        let max_selected = state.max_selected.as_deref().copied();

        let btc_price_in_fiat = state.btc_price_in_fiat.clone();
        let selected_fiat_currency = state.selected_fiat_currency.clone();
        let fee_rate_options = fee_rate_options.as_deref().copied();

        Self {
            metadata,
            fee_rate_options,
            max_selected,
            btc_price_in_fiat,
            selected_fiat_currency,
        }
    }

    pub fn on_change(&self, old_value: &str, new_value: &str) -> Changeset {
        // ---------------------------------------------------------------------
        // 1. early exits / sanitation
        // ---------------------------------------------------------------------
        if self.metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return Changeset::default();
        }

        let old = old_value.trim();
        let new = new_value.trim();

        if new.is_empty() {
            return Changeset {
                amount_fiat: Some("0".into()),
                ..Default::default()
            };
        }

        if new.starts_with("00") {
            return Changeset {
                entering_amount_btc: Some("0".into()),
                ..Default::default()
            };
        }

        if new.len() == 2 && new.starts_with('0') && new != "0." {
            return Changeset {
                entering_amount_btc: Some(new.trim_start_matches('0').into()),
                ..Default::default()
            };
        }

        // ---------------------------------------------------------------------
        // 2. normalize for parsing
        // ---------------------------------------------------------------------
        let mut changeset = Changeset::default();
        let mut unformatted = new.replace(',', "");

        if self.metadata.selected_unit == Unit::Sat {
            unformatted = unformatted.replace('.', "");
        }

        if unformatted == "." {
            unformatted = "0.".into();
            changeset.entering_amount_btc = Some("0.".into());
        }

        // if the unformatted is the same as the old value, then we don't need to do anything
        if old.replace(',', "") == unformatted {
            return Changeset::default();
        }

        // ---------------------------------------------------------------------
        // 3. parse to `Amount`
        // ---------------------------------------------------------------------
        let amount = match self.metadata.selected_unit {
            Unit::Sat => unformatted.parse::<u64>().ok().map(Amount::from_sat),
            Unit::Btc => unformatted
                .parse::<f64>()
                .ok()
                .and_then(|v| Amount::from_btc(v).ok()),
        };

        let amount = match amount {
            Some(a) => a,
            None => {
                return Changeset {
                    entering_amount_btc: Some(old.into()),
                    ..Default::default()
                };
            }
        };

        // ---------------------------------------------------------------------
        // 4. apply rules
        // ---------------------------------------------------------------------
        if let Some(max) = &self.max_selected {
            if amount < *max {
                // clear the max selected
                changeset.max_selected = Some(None);
            }
        }

        // set the amount
        changeset.amount_btc = Some(amount);

        // set the fiat amount display
        if let Some(price) = self.btc_price_in_fiat {
            let fiat_val = amount.as_btc() * price;
            changeset.amount_fiat = Some(self.display_fiat_amount(fiat_val));
        }

        // if its sat add thousands formatting
        if self.metadata.selected_unit == Unit::Sat {
            changeset.entering_amount_btc = Some(amount.as_sats().thousands_int());
        }

        changeset
    }

    pub fn display_fiat_amount(&self, amount: f64) -> String {
        {
            let sensitive_visible = self.metadata.sensitive_visible;
            if !sensitive_visible {
                return "**************".to_string();
            }
        }

        let fiat = amount.thousands_fiat();

        let selected_fiat_currency = Database::global().global_config().fiat_currency();
        let currency = selected_fiat_currency.unwrap_or_default();
        let symbol = currency.symbol();
        let suffix = currency.suffix();

        if !suffix.is_empty() {
            return format!("{symbol}{fiat} {suffix}");
        }

        format!("{symbol}{fiat}")
    }
}
