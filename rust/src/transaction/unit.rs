use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum, strum::EnumIter)]
pub enum Unit {
    Btc,
    Sat,
}

mod ffi {
    use super::Unit;
    use strum::IntoEnumIterator;

    #[uniffi::export]
    pub fn all_units() -> Vec<Unit> {
        Unit::iter().collect()
    }

    #[uniffi::export]
    pub fn unit_to_string(unit: Unit) -> String {
        unit.to_string()
    }
}

impl Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unit::Btc => write!(f, "BTC"),
            Unit::Sat => write!(f, "SATS"),
        }
    }
}
