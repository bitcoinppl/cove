use crate::{
    converter::{Converter, ConverterError},
    fiat::{FiatCurrency, client::PriceResponse},
    transaction::Amount,
};

use cove_util::format::NumberFormatter as _;

/// Handles the logic for what happens when the fiat amount onChange is called

#[derive(Debug, Clone)]
pub struct FiatOnChangeHandler {
    prices: PriceResponse,
    selected_currency: FiatCurrency,
    converter: Converter,
}

#[derive(Debug, Clone)]
pub struct FiatOnChangeResult {
    pub fiat_text: Option<String>,
    pub fiat_value: Option<f64>,
    pub btc_amount: Option<Amount>,
}

impl Default for FiatOnChangeResult {
    fn default() -> Self {
        Self::no_change()
    }
}

impl FiatOnChangeResult {
    fn no_change() -> Self {
        Self {
            fiat_text: None,
            fiat_value: None,
            btc_amount: None,
        }
    }

    fn empty_zero(symbol: &str) -> Self {
        Self {
            fiat_text: Some(symbol.to_string()),
            fiat_value: Some(0.0),
            btc_amount: Some(Amount::from_sat(0).into()),
        }
    }
}

pub type Error = SendFlowFiatOnChangeError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, uniffi::Enum, thiserror::Error)]
pub enum SendFlowFiatOnChangeError {
    #[error("invalid fiat amount: {error} ({input})")]
    InvalidFiatAmount { error: String, input: String },

    #[error("converter error: {0}")]
    ConverterError(#[from] ConverterError),
}

impl FiatOnChangeHandler {
    pub fn new(prices: PriceResponse, selected_currency: FiatCurrency) -> Self {
        let converter = Converter::global();

        Self {
            prices,
            selected_currency,
            converter,
        }
    }

    pub fn on_change(&self, old_value: String, new_value: String) -> Result<FiatOnChangeResult> {
        let old_value = old_value.trim();
        let new_value = new_value.trim();

        let symbol = self.selected_currency.symbol();

        let number_of_decimal_points = new_value.chars().filter(|c| *c == '.').count();

        let new_value_raw = new_value
            .chars()
            .filter(|c| c.is_numeric() || *c == '.')
            .collect::<String>();

        let old_value_raw = old_value
            .chars()
            .filter(|c| c.is_numeric() || *c == '.')
            .collect::<String>();

        // if the new value is the symbol, then we don't need to do anything
        if new_value == symbol {
            return Ok(FiatOnChangeResult::empty_zero(symbol));
        }

        // don't allow deleting the fiat amount symbol
        if new_value.is_empty() && !symbol.is_empty() {
            return Ok(FiatOnChangeResult::empty_zero(symbol));
        }

        // if old value is the same as the new value, then we don't need to do anything
        if old_value == new_value {
            return Ok(FiatOnChangeResult::no_change());
        }

        // don't allow adding more than 1 decimal point
        if number_of_decimal_points > 1 {
            return Ok(FiatOnChangeResult {
                fiat_text: Some(old_value.to_string()),
                ..Default::default()
            });
        };

        // if the only change was formatting (adding ,) then we don't need to do anything
        if old_value_raw == new_value_raw {
            return Ok(FiatOnChangeResult::no_change());
        }

        // if its 0.00 (starting state) and they enter an amount auto delete the 0.00
        if old_value_raw == "0.00" && new_value_raw.len() > 3 {
            let mut change = FiatOnChangeResult::no_change();
            let new_value = new_value_raw.trim_start_matches("0.00");

            change.fiat_text = Some(format!("{symbol}{new_value}"));
            change.fiat_value = Some(self.converter.get_fiat_value(old_value).unwrap_or_default());
            return Ok(change);
        }

        // if 0.00 and start deleting, just delete the entire thing
        if old_value_raw == "0.00" && new_value_raw.len() == 3 {
            let mut change = FiatOnChangeResult::no_change();
            change.fiat_text = Some(symbol.to_string());
            change.fiat_value = Some(0.0);
            return Ok(change);
        }

        // convert the fiat amount to btc amount
        let btc_amount = self.converter.convert_from_fiat_string(
            &new_value_raw,
            self.selected_currency,
            self.prices.clone(),
        );

        // get how many decimals there are after the decimal point
        let last_index = new_value_raw.len().saturating_sub(1);
        let int_value_suffix = match memchr::memchr(b'.', new_value_raw.as_bytes()) {
            Some(decimal_index) => {
                let decimals = last_index - decimal_index;

                // get the decimal point and the decimals after it to a max of 2 decimals
                let decimals = decimals.min(2);
                new_value_raw[decimal_index..=decimal_index + decimals].to_string()
            }

            None => "".to_string(),
        };

        // format to thousands
        let fiat_value = self.converter.get_fiat_value(&new_value_raw)?;

        // get the fiat text, taking into account the the decimals might not be complete
        let fiat_value_int = (fiat_value.trunc() as u64).thousands_int();
        let fiat_text = format!("{symbol}{fiat_value_int}{int_value_suffix}");

        let change = FiatOnChangeResult {
            fiat_text: Some(fiat_text),
            fiat_value: Some(fiat_value),
            btc_amount: Some(btc_amount),
        };

        Ok(change)
    }
}

#[uniffi::export]
fn describe_send_flow_fiat_on_change_error(error: SendFlowFiatOnChangeError) -> String {
    error.to_string()
}
