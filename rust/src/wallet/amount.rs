#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Object, derive_more::From, derive_more::Deref,
)]
pub struct Amount(bitcoin_units::Amount);

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

    pub fn to_btc(&self) -> f64 {
        self.0.to_btc()
    }

    pub fn to_sats(&self) -> u64 {
        self.0.to_sat()
    }
}
