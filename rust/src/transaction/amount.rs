use bdk_wallet::bitcoin::Amount as BdkAmount;
use numfmt::{Formatter, Precision};

use super::Unit;
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    uniffi::Object,
    derive_more::From,
    derive_more::Deref,
)]
pub struct Amount(pub BdkAmount);

#[uniffi::export]
impl Amount {
    #[uniffi::constructor]
    pub fn one_btc() -> Self {
        Self(bitcoin_units::Amount::ONE_BTC)
    }

    #[uniffi::constructor]
    pub fn one_sat() -> Self {
        Self(bitcoin_units::Amount::ONE_SAT)
    }

    #[uniffi::constructor]
    pub fn from_sat(sats: u64) -> Self {
        Self(bitcoin_units::Amount::from_sat(sats))
    }

    pub fn as_btc(&self) -> f64 {
        self.0.to_btc()
    }

    pub fn fmt_string_with_unit(&self, unit: Unit) -> String {
        match unit {
            Unit::Btc => self.btc_string_with_unit(),
            Unit::Sat => self.sats_string_with_unit(),
        }
    }

    pub fn as_sats(&self) -> u64 {
        self.0.to_sat()
    }

    pub fn btc_string(&self) -> String {
        format!("{:.8}", self.as_btc())
    }

    pub fn btc_string_with_unit(&self) -> String {
        format!("{:.8} BTC", self.as_btc())
    }

    pub fn sats_string(&self) -> String {
        let mut f = Formatter::new()
            .separator(',')
            .unwrap()
            .precision(Precision::Decimals(0));

        f.fmt2(self.as_sats() as f64).to_string()
    }

    pub fn sats_string_with_unit(&self) -> String {
        format!("{} SATS", self.sats_string())
    }
}
