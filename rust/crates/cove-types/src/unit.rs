use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    uniffi::Enum,
    strum::EnumIter,
)]
#[uniffi::export(Display)]
pub enum BitcoinUnit {
    #[default]
    Btc,
    Sat,
}

use strum::IntoEnumIterator;

impl BitcoinUnit {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Btc => Self::Sat,
            Self::Sat => Self::Btc,
        }
    }
}

#[uniffi::export]
fn all_units() -> Vec<BitcoinUnit> {
    BitcoinUnit::iter().collect()
}

impl Display for BitcoinUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Btc => write!(f, "BTC"),
            Self::Sat => write!(f, "SATS"),
        }
    }
}
