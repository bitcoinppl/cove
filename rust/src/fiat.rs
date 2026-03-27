pub mod amount;
pub mod client;
pub mod historical;

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
#[uniffi::export(Display)]
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

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Usd | Self::Cad | Self::Aud => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy => "¥",
            Self::Chf => "",
        }
    }

    pub const fn emoji(self) -> &'static str {
        match self {
            Self::Usd => "🇺🇸",
            Self::Cad => "🇨🇦",
            Self::Aud => "🇦🇺",
            Self::Eur => "🇪🇺",
            Self::Gbp => "🇬🇧",
            Self::Chf => "🇨🇭",
            Self::Jpy => "🇯🇵",
        }
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Cad => "CAD",
            Self::Aud => "AUD",
            Self::Chf => "CHF",
            Self::Usd | Self::Eur | Self::Gbp | Self::Jpy => "",
        }
    }
}

impl Display for FiatCurrency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: &'static str = self.into();
        write!(f, "{s}")
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
            "USD" => Ok(Self::Usd),
            "CAD" => Ok(Self::Cad),
            "AUD" => Ok(Self::Aud),
            "EUR" => Ok(Self::Eur),
            "GBP" => Ok(Self::Gbp),
            "CHF" => Ok(Self::Chf),
            "JPY" => Ok(Self::Jpy),
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
impl FiatCurrency {
    #[uniffi::method(name = "symbolString")]
    fn ffi_symbol_string(&self) -> String {
        self.symbol().to_string()
    }

    #[uniffi::method(name = "emojiString")]
    fn ffi_emoji_string(&self) -> String {
        self.emoji().to_string()
    }

    #[uniffi::method(name = "suffixString")]
    fn ffi_suffix_string(&self) -> String {
        self.suffix().to_string()
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
