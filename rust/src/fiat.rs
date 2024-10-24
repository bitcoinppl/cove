pub mod client;

use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FiatCurrency {
    Usd,
    Eur,
    Gbp,
    Cad,
    Chf,
    Aud,
    Jpy,
}

impl Display for FiatCurrency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: &'static str = self.into();
        write!(f, "{}", s)
    }
}

impl Into<&'static str> for FiatCurrency {
    fn into(self) -> &'static str {
        match self {
            FiatCurrency::Usd => "USD",
            FiatCurrency::Eur => "EUR",
            FiatCurrency::Gbp => "GBP",
            FiatCurrency::Cad => "CAD",
            FiatCurrency::Chf => "CHF",
            FiatCurrency::Aud => "AUD",
            FiatCurrency::Jpy => "JPY",
        }
    }
}

impl TryFrom<&str> for FiatCurrency {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "USD" => Ok(FiatCurrency::Usd),
            "EUR" => Ok(FiatCurrency::Eur),
            "GBP" => Ok(FiatCurrency::Gbp),
            "CAD" => Ok(FiatCurrency::Cad),
            "CHF" => Ok(FiatCurrency::Chf),
            "AUD" => Ok(FiatCurrency::Aud),
            "JPY" => Ok(FiatCurrency::Jpy),
            _ => Err(format!("unknown fiat currency: {value}")),
        }
    }
}

impl TryFrom<String> for FiatCurrency {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.to_ascii_uppercase().as_str().try_into()
    }
}

impl Into<&'static str> for &FiatCurrency {
    fn into(self) -> &'static str {
        let me: FiatCurrency = *self;
        me.into()
    }
}
