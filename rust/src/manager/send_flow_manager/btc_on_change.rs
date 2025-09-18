use std::sync::Arc;

use crate::wallet::metadata::{FiatOrBtc, WalletMetadata};
use cove_types::{amount::Amount, unit::Unit};
use cove_util::format::{self, NumberFormatter as _};
use tracing::debug;

use super::state::State;

#[derive(Debug, Clone)]
pub struct BtcOnChangeHandler {
    metadata: WalletMetadata,
    max_selected: Option<Arc<Amount>>,
    btc_price_in_fiat: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Changeset {
    pub entering_amount_btc: Option<String>,
    pub amount_btc: Option<Amount>,
    pub amount_fiat: Option<f64>,
    pub max_selected: Option<Option<Amount>>,
}

impl BtcOnChangeHandler {
    pub fn new(state: impl Into<State>) -> Self {
        let state = state.into();
        let state = state.lock();

        let metadata = state.metadata.clone();
        let max_selected = state.max_selected.clone();
        let btc_price_in_fiat = state.btc_price_in_fiat;

        Self { metadata, max_selected, btc_price_in_fiat }
    }

    pub fn on_change(&self, old_value: &str, new_value: &str) -> Changeset {
        // ---------------------------------------------------------------------
        // 1. early exits and sanitization
        // ---------------------------------------------------------------------
        if self.metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return Changeset::default();
        }

        let old = old_value.trim();
        let new = new_value.trim();

        // early exit if nothing changed
        if old == new {
            return Changeset::default();
        }

        if new == "00" {
            return Changeset {
                entering_amount_btc: Some("0".into()),
                amount_btc: Some(Amount::from_sat(0)),
                amount_fiat: Some(0.0),
                ..Default::default()
            };
        }

        if new.is_empty() {
            return Changeset {
                amount_btc: Some(Amount::from_sat(0)),
                amount_fiat: Some(0.0),
                ..Default::default()
            };
        }

        let unit = self.metadata.selected_unit;

        // decimal points `.` count
        let number_of_periods = new.chars().filter(|c| *c == '.').count();

        if unit == Unit::Sat && number_of_periods > 0 {
            return Changeset { entering_amount_btc: Some(old.to_string()), ..Default::default() };
        }

        if number_of_periods > 1 {
            return Changeset { entering_amount_btc: Some(old.to_string()), ..Default::default() };
        }

        // starting to enter decimal point values, no changes for now
        if new.ends_with('.') {
            return Changeset::default();
        }

        // ---------------------------------------------------------------------
        // 2. normalize for parsing
        // ---------------------------------------------------------------------
        let mut changeset = Changeset::default();
        let mut unformatted = new.replace(',', "");

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
            Unit::Btc => unformatted.parse::<f64>().ok().and_then(|v| Amount::from_btc(v).ok()),
        };

        let amount = match amount {
            Some(a) => a,
            None => {
                debug!("unable to parse amount: {unformatted}");
                return Changeset { entering_amount_btc: Some(old.into()), ..Default::default() };
            }
        };

        // check if its over the max
        if amount > Amount::MAX_MONEY {
            return Changeset { entering_amount_btc: Some(old.into()), ..Default::default() };
        }

        // ---------------------------------------------------------------------
        // 4. apply rules
        // ---------------------------------------------------------------------
        if let Some(max) = &self.max_selected
            && &amount < max
        {
            // clear the max selected
            changeset.max_selected = Some(None);
        }

        // set the amount
        changeset.amount_btc = Some(amount);

        // set the fiat amount display
        if let Some(price) = self.btc_price_in_fiat {
            changeset.amount_fiat = Some(amount.as_btc() * (price as f64));
        }

        let entering_amount_btc = match self.metadata.selected_unit {
            Unit::Sat => Some(amount.as_sats().thousands_int()),
            Unit::Btc => format::btc_typing(&unformatted),
        };

        if let Some(entering_amount_btc) = entering_amount_btc
            && entering_amount_btc != new_value
        {
            changeset.entering_amount_btc = Some(entering_amount_btc);
        }

        changeset
    }
}
