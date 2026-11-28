use cove_util::result_ext::ResultExt as _;
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
#[uniffi::export(Display)]
pub enum ConverterError {
    #[error("Unable to get fiat amount from string: {0}")]
    FiatAmountFromStringError(String),
}

impl Converter {
    pub const fn new() -> Self {
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
                tracing::error!("failed to convert fiat amount: {error} ({fiat_amount})");
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

        // rounded, not floored: f64 division lands just below whole amounts, so $30 at $100,000
        // would otherwise come out as 29,999 sats
        let sat_amount = (fiat_amount * 100_000_000.0 / price).round() as u64;

        Amount::from_sat(sat_amount)
    }
}

#[uniffi::export]
impl Converter {
    #[uniffi::constructor(name = "new")]
    pub const fn global() -> Self {
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

        // keep at most two decimals; flooring `x * 100.0` loses a cent, e.g. 1.15 -> 1.14
        let fiat_amount = match fiat_amount.split_once('.') {
            Some((int, cents)) => format!("{int}.{}", cents.chars().take(2).collect::<String>()),
            None => fiat_amount,
        };

        let fiat_value =
            fiat_amount.parse::<f64>().map_err_str(Error::FiatAmountFromStringError)?;

        Ok(fiat_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fiat_str_keeps_typed_cents() {
        let converter = Converter::new();
        assert_eq!(converter.parse_fiat_str("$0.29").unwrap(), 0.29);
        assert_eq!(converter.parse_fiat_str("$1.15").unwrap(), 1.15);
        assert_eq!(converter.parse_fiat_str("$2.01").unwrap(), 2.01);
        assert_eq!(converter.parse_fiat_str("$1,234.567").unwrap(), 1234.56);
        assert_eq!(converter.parse_fiat_str("$12.").unwrap(), 12.0);
    }

    #[test]
    fn fiat_amounts_convert_to_the_nearest_sat() {
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

        let converter = Converter::new();
        let sats = |fiat: &str| {
            converter.convert_from_fiat_string(fiat, FiatCurrency::Usd, prices).as_sats()
        };

        assert_eq!(sats("$30"), 30_000);
        assert_eq!(sats("$20.00"), 20_000);
        assert_eq!(sats("$0.29"), 290);
        assert_eq!(sats("$1,234.56"), 1_234_560);
    }
}
