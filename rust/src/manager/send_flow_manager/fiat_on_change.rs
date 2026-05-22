use cove_util::format::NumberFormatter as _;

use crate::{
    converter::{Converter, ConverterError},
    fiat::{FiatCurrency, client::PriceResponse},
    transaction::Amount,
};

use super::sanitize;

/// Handles the logic for what happens when the fiat amount onChange is called

#[derive(Debug, Clone)]
pub struct FiatOnChangeHandler {
    prices: PriceResponse,
    selected_currency: FiatCurrency,
    converter: Converter,
    max_selected: Option<Amount>,
}

#[derive(Debug, Clone, Default)]
pub struct Changeset {
    pub entering_fiat_amount: Option<String>,
    pub fiat_value: Option<f64>,
    pub btc_amount: Option<Amount>,
    pub max_selected: Option<Option<Amount>>,
}

impl Changeset {
    fn empty_zero(symbol: &str) -> Self {
        Self {
            entering_fiat_amount: Some(symbol.to_string()),
            fiat_value: Some(0.0),
            btc_amount: Some(Amount::from_sat(0)),
            max_selected: None,
        }
    }
}

pub type Error = SendFlowFiatOnChangeError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum SendFlowFiatOnChangeError {
    #[error("invalid fiat amount: {error} ({input})")]
    InvalidFiatAmount { error: String, input: String },

    #[error("converter error: {0}")]
    Converter(#[from] ConverterError),
}

impl FiatOnChangeHandler {
    pub fn new(
        prices: PriceResponse,
        selected_currency: FiatCurrency,
        max_selected: Option<Amount>,
    ) -> Self {
        Self { prices, selected_currency, converter: Converter::new(), max_selected }
    }

    pub fn on_change(&self, old_value: &str, new_value: &str) -> Result<Changeset> {
        let old_value = old_value.trim();
        let new_value_trimmed = new_value.trim();

        // strip fiat tokens from pasted amounts (e.g. "$12.50", "12.50 USD"); rejects BTC/SATS
        let Some(new_value) = sanitize::sanitize_fiat_amount(new_value_trimmed) else {
            return Ok(Changeset {
                entering_fiat_amount: Some(old_value.to_string()),
                ..Default::default()
            });
        };

        // If sanitization stripped tokens (e.g. "100 CHF" → "100"), early exits must still
        // emit the cleaned string so the raw pasted text doesn't stay visible in the UI.
        let sanitization_changed = new_value != new_value_trimmed;

        let symbol = self.selected_currency.symbol();

        let number_of_decimal_points = new_value.chars().filter(|c| *c == '.').count();

        let new_value_raw =
            new_value.chars().filter(|c| c.is_numeric() || *c == '.').collect::<String>();

        let old_value_raw =
            old_value.chars().filter(|c| c.is_numeric() || *c == '.').collect::<String>();

        // if the new value is the symbol, then we don't need to do anything
        if new_value == symbol || new_value.is_empty() {
            return Ok(Changeset::empty_zero(symbol));
        }

        // early exit if same value is passed in
        if old_value == new_value {
            return Ok(Changeset {
                entering_fiat_amount: sanitization_changed.then_some(old_value.to_string()),
                ..Default::default()
            });
        }

        if old_value_raw == new_value_raw {
            return Ok(Changeset {
                entering_fiat_amount: sanitization_changed.then_some(old_value.to_string()),
                ..Default::default()
            });
        }

        // start entering with a period
        if new_value_raw == "." {
            return Ok(Changeset {
                entering_fiat_amount: Some(format!("{symbol}.")),
                ..Default::default()
            });
        }

        // if old value is the same as the new value, then we don't need to do anything
        if old_value == new_value {
            return Ok(Changeset {
                entering_fiat_amount: sanitization_changed.then_some(old_value.to_string()),
                ..Default::default()
            });
        }

        // don't allow adding more than 1 decimal point
        if number_of_decimal_points > 1 {
            return Ok(Changeset {
                entering_fiat_amount: Some(old_value.to_string()),
                ..Default::default()
            });
        }

        // if the only change was formatting (adding ,) then we don't need to do anything
        if old_value_raw == new_value_raw {
            return Ok(Changeset {
                entering_fiat_amount: sanitization_changed.then_some(old_value.to_string()),
                ..Default::default()
            });
        }

        // convert the fiat amount to btc amount
        let btc_amount = self.converter.convert_from_fiat_string(
            &new_value_raw,
            self.selected_currency,
            self.prices,
        );

        // if the amount is too large, don't allow it
        if btc_amount > Amount::MAX_MONEY {
            return Ok(Changeset {
                entering_fiat_amount: Some(old_value.to_string()),
                ..Default::default()
            });
        }

        let mut fiat_value_to_parse = new_value_raw.as_str();

        // if its already 0.00, just start entering dollars
        if old_value_raw == "0.00" {
            fiat_value_to_parse = fiat_value_to_parse.trim_start_matches("0.00");
        }

        // if the old value is 0.00, and we are erasing, erase all of it
        if old_value_raw == "0.00" && new_value_raw == "0.0" {
            fiat_value_to_parse = "";
        }

        // get fiat value as a f64
        let fiat_value = self.converter.parse_fiat_str(fiat_value_to_parse)?;

        // get how many decimals there are after the decimal point
        let (dollars, cents_with_decimal_point) =
            sanitize::seperate_and_limit_dollars_and_cents(fiat_value_to_parse, 2);

        let dollars = dollars.parse::<u64>().ok().unwrap_or_default();
        let dollars_formatted = dollars.thousands_int();

        let mut changes = Changeset {
            fiat_value: Some(fiat_value),
            btc_amount: Some(btc_amount),
            ..Default::default()
        };

        let fiat_text = format!("{symbol}{dollars_formatted}{cents_with_decimal_point}");
        if fiat_text != new_value {
            changes.entering_fiat_amount = Some(fiat_text);
        }

        if let Some(max_selected) = self.max_selected {
            let max_selected = max_selected.as_sats();
            if btc_amount.as_sats() < max_selected {
                changes.max_selected = Some(None);
            }
        }

        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler() -> FiatOnChangeHandler {
        let prices = PriceResponse {
            time: 0,
            fetched_at: 0,
            usd: 100_000,
            eur: 100_000,
            gbp: 100_000,
            cad: 100_000,
            chf: 100_000,
            aud: 100_000,
            jpy: 100_000,
        };
        FiatOnChangeHandler::new(prices, FiatCurrency::Usd, None)
    }

    #[test]
    fn pasting_bech32_bitcoin_address_is_rejected() {
        let h = handler();
        let old = "$100";
        let new = "$100bc1qxyz0123456789abcdef";
        let result = h.on_change(old, new).unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$100"));
        assert!(result.btc_amount.is_none());
        assert!(result.fiat_value.is_none());
    }

    #[test]
    fn pasting_legacy_bitcoin_address_is_rejected() {
        let h = handler();
        let old = "$0";
        let new = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
        let result = h.on_change(old, new).unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$0"));
        assert!(result.btc_amount.is_none());
    }

    #[test]
    fn typing_a_single_letter_is_rejected() {
        let h = handler();
        let old = "$1";
        let new = "$1a";
        let result = h.on_change(old, new).unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$1"));
        assert!(result.btc_amount.is_none());
    }

    #[test]
    fn plain_digits_are_still_accepted() {
        let h = handler();
        let old = "$";
        let new = "$123";
        let result = h.on_change(old, new).unwrap();
        assert!(result.fiat_value.is_some());
        assert!(result.btc_amount.is_some());
    }

    #[test]
    fn decimal_input_still_accepted() {
        let h = handler();
        let old = "$1";
        let new = "$1.5";
        let result = h.on_change(old, new).unwrap();
        assert!(result.fiat_value.is_some());
        assert!(result.btc_amount.is_some());
    }

    #[test]
    fn empty_input_still_works() {
        let h = handler();
        let result = h.on_change("$100", "").unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$"));
        assert_eq!(result.fiat_value, Some(0.0));
    }

    #[test]
    fn pasting_amount_with_chf_suffix_is_accepted() {
        let h = handler();
        let result = h.on_change("$0", "100 CHF").unwrap();
        // CHF is a fiat token — strips to "100", treated as 100 USD
        assert_eq!(result.fiat_value, Some(100.0));
        assert!(result.btc_amount.is_some());
    }

    #[test]
    fn pasting_amount_with_btc_suffix_is_rejected() {
        let h = handler();
        let result = h.on_change("$0", "0.5 BTC").unwrap();
        // BTC is not a fiat token — should revert to old value
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$0"));
        assert!(result.fiat_value.is_none());
        assert!(result.btc_amount.is_none());
    }

    #[test]
    fn pasting_amount_with_sats_suffix_is_rejected() {
        let h = handler();
        let result = h.on_change("$0", "1000 SATS").unwrap();
        // SATS is not a fiat token — should revert to old value
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$0"));
        assert!(result.fiat_value.is_none());
        assert!(result.btc_amount.is_none());
    }

    #[test]
    fn pasting_fiat_code_over_same_numeric_value_rewrites_field() {
        let h = handler();
        // "100 CHF" over "$100" — numeric value unchanged, but display must be rewritten
        let result = h.on_change("$100", "100 CHF").unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$100"));
        assert!(result.fiat_value.is_none()); // amount didn't change, no re-parse needed

        // "USD 100" over "$100" — same case with prefix code
        let result = h.on_change("$100", "USD 100").unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$100"));
        assert!(result.fiat_value.is_none());
    }

    #[test]
    fn pasting_bech32_address_still_rejected_after_suffix_strip() {
        let h = handler();
        let result = h.on_change("$0", "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").unwrap();
        assert_eq!(result.entering_fiat_amount.as_deref(), Some("$0"));
        assert!(result.btc_amount.is_none());
    }
}
