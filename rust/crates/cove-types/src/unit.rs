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
    pub fn toggle(self) -> Self {
        match self {
            BitcoinUnit::Btc => BitcoinUnit::Sat,
            BitcoinUnit::Sat => BitcoinUnit::Btc,
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
            BitcoinUnit::Btc => write!(f, "BTC"),
            BitcoinUnit::Sat => write!(f, "SATS"),
        }
    }
}
