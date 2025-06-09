use crate::transaction::Amount;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Default,
    uniffi::Object,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    derive_more::AsRef,
)]
pub struct Balance(pub bdk_wallet::Balance);

#[uniffi::export]
impl Balance {
    #[uniffi::constructor]
    pub fn zero() -> Self {
        Balance::default()
    }

    #[uniffi::method]
    pub fn spendable(&self) -> Amount {
        self.0.trusted_spendable().into()
    }
}
