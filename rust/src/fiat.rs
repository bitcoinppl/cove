pub mod amount;
pub mod client;

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator as _;

pub type FiatAmount = amount::FiatAmount;

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    uniffi::Enum,
    Serialize,
    Deserialize,
    strum::EnumIter,
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
    pub const fn all_symbols() -> &'static [&'static str] {
        &["$", "€", "£", "¥"]
    }

    pub const fn all_symbols_as_chars() -> &'static [char] {
        &['$', '€', '£', '¥']
    }

    pub fn is_symbol(symbol: &str) -> bool {
        matches!(symbol, "$" | "€" | "£" | "¥")
    }

    pub const fn symbol(&self) -> &'static str {
        use FiatCurrency as F;

        match self {
            F::Usd | F::Cad | F::Aud => "$",
            F::Eur => "€",
            F::Gbp => "£",
            F::Jpy => "¥",
            F::Chf => "",
        }
    }

    pub const fn emoji(&self) -> &'static str {
        match self {
            FiatCurrency::Usd => "🇺🇸",
            FiatCurrency::Cad => "🇨🇦",
            FiatCurrency::Aud => "🇦🇺",
            FiatCurrency::Eur => "🇪🇺",
            FiatCurrency::Gbp => "🇬🇧",
            FiatCurrency::Chf => "🇨🇭",
            FiatCurrency::Jpy => "🇯🇵",
        }
    }

    pub const fn suffix(&self) -> &'static str {
        match self {
            FiatCurrency::Usd => "",
            FiatCurrency::Cad => "CAD",
            FiatCurrency::Aud => "AUD",
            FiatCurrency::Eur => "",
            FiatCurrency::Gbp => "",
            FiatCurrency::Chf => "CHF",
            FiatCurrency::Jpy => "",
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

#[uniffi::export]
fn all_fiat_currencies() -> Vec<FiatCurrency> {
    FiatCurrency::iter().collect()
}

#[uniffi::export]
fn is_fiat_currency_symbol(symbol: &str) -> bool {
    FiatCurrency::is_symbol(symbol)
}

#[uniffi::export]
fn fiat_currency_to_string(fiat_currency: FiatCurrency) -> String {
    fiat_currency.to_string()
}

#[uniffi::export]
fn fiat_currency_symbol(fiat_currency: FiatCurrency) -> String {
    fiat_currency.symbol().to_string()
}

#[uniffi::export]
fn fiat_currency_emoji(fiat_currency: FiatCurrency) -> String {
    fiat_currency.emoji().to_string()
}

#[uniffi::export]
fn fiat_currency_suffix(fiat_currency: FiatCurrency) -> String {
    fiat_currency.suffix().to_string()
}
