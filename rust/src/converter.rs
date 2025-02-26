/// Functions that help display and convert different units
/// Maybe later we can move this into a seperate folder called presenters
///
use std::sync::{Arc, LazyLock};

pub static CONVERTER: LazyLock<Arc<Converter>> = LazyLock::new(|| Arc::new(Converter));

#[derive(Debug, Clone, uniffi::Object)]
pub struct Converter;

type Error = ConverterError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum ConverterError {
    #[error("Unable to get fiat amount from string: {0}")]
    FiatAmountFromStringError(String),
}

#[uniffi::export]
impl Converter {
    #[uniffi::constructor(name = "new")]
    pub fn global() -> Self {
        CONVERTER.as_ref().clone()
    }

    pub fn get_fiat_value(&self, fiat_amount: String) -> Result<f64> {
        if fiat_amount.is_empty() {
            return Ok(0.0);
        }

        let fiat_amount = fiat_amount
            .chars()
            .filter(|c| c.is_numeric() || *c == '.')
            .collect::<String>();

        let fiat_value = fiat_amount
            .parse::<f64>()
            .map_err(|e| Error::FiatAmountFromStringError(e.to_string()))?;

        let fiat_value = (fiat_value * 100.0).floor() / 100.0;
        Ok(fiat_value)
    }

    pub fn remove_fiat_suffix(&self, fiat_amount: String) -> String {
        let currency_prefixes = ['$', '€', '£', '¥'];

        fiat_amount
            .chars()
            .filter(|c| c.is_numeric() || *c == '.' || currency_prefixes.contains(c))
            .collect::<String>()
    }
}
