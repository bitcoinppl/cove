use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum, strum::EnumIter,
)]
#[uniffi::export(Display)]
pub enum BitcoinUnit {
    Btc,
    Sat,
}

impl Default for BitcoinUnit {
    fn default() -> Self {
        Self::Btc
    }
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

#[uniffi::export]
fn unit_to_string(unit: BitcoinUnit) -> String {
    unit.to_string()
}

impl Display for BitcoinUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BitcoinUnit::Btc => write!(f, "BTC"),
            BitcoinUnit::Sat => write!(f, "SATS"),
        }
    }
}
