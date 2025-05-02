use tap::TapFallible as _;

use crate::{
    fiat::{FiatCurrency, client::PriceResponse},
    transaction::Amount,
};

#[derive(Debug, Clone, uniffi::Object)]
pub struct Converter;

type Error = ConverterError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum ConverterError {
    #[error("Unable to get fiat amount from string: {0}")]
    FiatAmountFromStringError(String),
}

impl Converter {
    pub fn new() -> Self {
        Self
    }

    pub fn convert_from_fiat_string(
        &self,
        fiat_amount: &str,
        currency: FiatCurrency,
        prices: PriceResponse,
    ) -> Amount {
        if fiat_amount.len() == 1 && FiatCurrency::is_symbol(fiat_amount) {
            return Amount::from_sat(0);
        }

        if fiat_amount.is_empty() {
            return Amount::from_sat(0);
        }

        let fiat_value = self
            .parse_fiat_str(fiat_amount)
            .tap_err(|error| {
                tracing::error!("failed to convert fiat amount: {error} ({fiat_amount})")
            })
            .unwrap_or_default();

        self.convert_from_fiat(fiat_value, currency, prices)
    }

    pub fn convert_from_fiat(
        &self,
        fiat_amount: f64,
        currency: FiatCurrency,
        prices: PriceResponse,
    ) -> Amount {
        let price = prices.get_for_currency(currency) as f64;
        let btc_amount = fiat_amount / price;
        let sat_amount = (btc_amount * 100_000_000.0).floor() as u64;

        Amount::from_sat(sat_amount)
    }
}

#[uniffi::export]
impl Converter {
    #[uniffi::constructor(name = "new")]
    pub fn global() -> Self {
        Self::new()
    }

    pub fn parse_fiat_str(&self, fiat_amount: &str) -> Result<f64> {
        if fiat_amount.is_empty() {
            return Ok(0.0);
        }

        let fiat_amount =
            fiat_amount.chars().filter(|c| c.is_numeric() || *c == '.').collect::<String>();

        if fiat_amount.is_empty() || FiatCurrency::is_symbol(&fiat_amount) {
            return Ok(0.0);
        }

        let fiat_value = fiat_amount
            .parse::<f64>()
            .map_err(|e| Error::FiatAmountFromStringError(e.to_string()))?;

        let fiat_value = (fiat_value * 100.0).floor() / 100.0;
        Ok(fiat_value)
    }

    pub fn remove_fiat_suffix(&self, fiat_amount: &str) -> String {
        let currency_prefixes = FiatCurrency::all_symbols_as_chars();

        fiat_amount
            .chars()
            .filter(|c| c.is_numeric() || *c == '.' || currency_prefixes.contains(c))
            .collect::<String>()
    }
}
