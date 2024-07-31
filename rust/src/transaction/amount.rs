use bdk_wallet::bitcoin::Amount as BdkAmount;

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
pub struct Amount(BdkAmount);

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

    pub fn btc_string(&self) -> String {
        format!("{:.8} BTC", self.as_btc())
    }

    pub fn sats_string(&self) -> String {
        format!("{} SATS", self.as_sats())
    }

    pub fn as_sats(&self) -> u64 {
        self.0.to_sat()
    }
}
