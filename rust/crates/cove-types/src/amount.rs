use bdk_wallet::bitcoin::Amount as BdkAmount;
use numfmt::{Formatter, Precision};
use serde::{Deserialize, Serialize};

use crate::unit::BitcoinUnit;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    uniffi::Object,
    derive_more::Add,
    derive_more::Sub,
    derive_more::Mul,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
)]
pub struct Amount(pub BdkAmount);

// rust only
impl Amount {
    pub const ZERO: Amount = Amount(BdkAmount::ZERO);
    pub const ONE_SAT: Amount = Amount(BdkAmount::ONE_SAT);
    pub const ONE_BTC: Amount = Amount(BdkAmount::ONE_BTC);
    pub const MAX_MONEY: Amount = Amount(BdkAmount::MAX_MONEY);
    pub const SIZE: usize = BdkAmount::SIZE; // Serialized length of a u64.

    pub fn from_btc(btc: f64) -> Result<Self, eyre::Report> {
        Ok(Self(bitcoin::Amount::from_btc(btc)?))
    }
}

#[uniffi::export]
impl Amount {
    #[uniffi::constructor]
    pub const fn from_sat(sats: u64) -> Self {
        Self(bitcoin::Amount::from_sat(sats))
    }

    #[uniffi::constructor]
    pub const fn one_btc() -> Self {
        Self(bitcoin::Amount::ONE_BTC)
    }

    #[uniffi::constructor]
    pub const fn one_sat() -> Self {
        Self(bitcoin::Amount::ONE_SAT)
    }

    pub fn as_btc(&self) -> f64 {
        self.0.to_btc()
    }

    pub fn fmt_string(&self, unit: BitcoinUnit) -> String {
        match unit {
            BitcoinUnit::Btc => self.btc_string(),
            BitcoinUnit::Sat => self.sats_string(),
        }
    }

    pub fn fmt_string_with_unit(&self, unit: BitcoinUnit) -> String {
        match unit {
            BitcoinUnit::Btc => self.btc_string_with_unit(),
            BitcoinUnit::Sat => self.sats_string_with_unit(),
        }
    }

    pub fn as_sats(&self) -> u64 {
        self.0.to_sat()
    }

    pub fn btc_string(&self) -> String {
        let mut f = Formatter::new().separator(',').unwrap().precision(Precision::Decimals(8));
        f.fmt(self.as_btc()).to_string()
    }

    pub fn btc_string_with_unit(&self) -> String {
        let mut f = Formatter::new().separator(',').unwrap().precision(Precision::Decimals(8));
        format!("{} BTC", f.fmt(self.as_btc()))
    }

    pub fn sats_string(&self) -> String {
        let mut f = Formatter::new().separator(',').unwrap().precision(Precision::Decimals(0));
        f.fmt(self.as_sats() as f64).to_string()
    }

    pub fn sats_string_with_unit(&self) -> String {
        format!("{} SATS", self.sats_string())
    }
}
