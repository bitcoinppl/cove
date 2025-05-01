use crate::wallet::metadata::{FiatOrBtc, WalletMetadata};
use cove_types::{amount::Amount, unit::Unit};
use cove_util::format::{self, NumberFormatter as _};

use super::state::State;

#[derive(Debug, Clone)]
pub struct BtcOnChangeHandler {
    metadata: WalletMetadata,
    max_selected: Option<Amount>,
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
    pub fn new(state: State) -> Self {
        let state = state.read();

        let metadata = state.metadata.clone();
        let max_selected = state.max_selected.as_deref().copied();
        let btc_price_in_fiat = state.btc_price_in_fiat;

        Self { metadata, max_selected, btc_price_in_fiat }
    }

    pub fn on_change(&self, old_value: &str, new_value: &str) -> Changeset {
        println!("btc_on_change_handler old: {old_value}, new: {new_value}");
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
                entering_amount_btc: Some("".into()),
                amount_btc: Some(Amount::from_sat(0)),
                amount_fiat: Some(0.0),
                ..Default::default()
            };
        }

        if new == "00" {
            return Changeset { entering_amount_btc: Some("0".into()), ..Default::default() };
        }

        if new.ends_with("..") {
            return Changeset { entering_amount_btc: Some(old.into()), ..Default::default() };
        }

        // don't allow adding . to sats
        if new.ends_with('.') && self.metadata.selected_unit == Unit::Sat {
            return Changeset { entering_amount_btc: Some(old.into()), ..Default::default() };
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
            Unit::Btc => unformatted.parse::<f64>().ok().and_then(|v| Amount::from_btc(v).ok()),
        };

        let amount = match amount {
            Some(a) => a,
            None => {
                return Changeset { entering_amount_btc: Some(old.into()), ..Default::default() };
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
            changeset.amount_fiat = Some(amount.as_btc() * (price as f64));
        }

        match self.metadata.selected_unit {
            Unit::Sat => changeset.entering_amount_btc = Some(amount.as_sats().thousands_int()),
            Unit::Btc => changeset.entering_amount_btc = format::btc_typing(&unformatted),
        };

        changeset
    }
}
