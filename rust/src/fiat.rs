pub mod amount;
pub mod client;

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};

pub type FiatAmount = amount::FiatAmount;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum, Serialize, Deserialize,
)]
#[serde(rename_all = "UPPERCASE")]
pub enum FiatCurrency {
    #[default]
    Usd,
    Cad,
    Aud,
    Eur,
    Gbp,
    Chf,
    Jpy,
}

impl FiatCurrency {
    pub fn symbol(&self) -> &'static str {
        match self {
            FiatCurrency::Usd => "$",
            FiatCurrency::Cad => "$",
            FiatCurrency::Aud => "$",
            FiatCurrency::Eur => "€",
            FiatCurrency::Gbp => "£",
            FiatCurrency::Chf => "Fr",
            FiatCurrency::Jpy => "¥",
        }
    }
}

impl Display for FiatCurrency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: &'static str = self.into();
        write!(f, "{}", s)
    }
}

impl From<FiatCurrency> for &'static str {
    fn from(val: FiatCurrency) -> Self {
        match val {
            FiatCurrency::Usd => "USD",
            FiatCurrency::Cad => "CAD",
            FiatCurrency::Aud => "AUD",
            FiatCurrency::Eur => "EUR",
            FiatCurrency::Gbp => "GBP",
            FiatCurrency::Chf => "CHF",
            FiatCurrency::Jpy => "JPY",
        }
    }
}

impl FromStr for FiatCurrency {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "USD" => Ok(FiatCurrency::Usd),
            "CAD" => Ok(FiatCurrency::Cad),
            "AUD" => Ok(FiatCurrency::Aud),
            "EUR" => Ok(FiatCurrency::Eur),
            "GBP" => Ok(FiatCurrency::Gbp),
            "CHF" => Ok(FiatCurrency::Chf),
            "JPY" => Ok(FiatCurrency::Jpy),
            _ => Err(format!("unknown fiat currency: {s}")),
        }
    }
}

impl TryFrom<String> for FiatCurrency {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.to_ascii_uppercase().as_str().parse()
    }
}

impl From<&FiatCurrency> for &'static str {
    fn from(val: &FiatCurrency) -> Self {
        let me: FiatCurrency = *val;
        me.into()
    }
}
