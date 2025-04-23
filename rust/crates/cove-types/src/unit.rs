use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum, strum::EnumIter,
)]
pub enum Unit {
    Btc,
    Sat,
}

impl Default for Unit {
    fn default() -> Self {
        Self::Btc
    }
}

use strum::IntoEnumIterator;

#[uniffi::export]
fn all_units() -> Vec<Unit> {
    Unit::iter().collect()
}

#[uniffi::export]
fn unit_to_string(unit: Unit) -> String {
    unit.to_string()
}

impl Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unit::Btc => write!(f, "BTC"),
            Unit::Sat => write!(f, "SATS"),
        }
    }
}
