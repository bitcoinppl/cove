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
#[uniffi::export(Eq)]
pub struct Balance(pub bdk_wallet::Balance);

#[uniffi::export]
impl Balance {
    #[uniffi::constructor]
    pub fn zero() -> Self {
        Self::default()
    }

    #[uniffi::method]
    pub fn spendable(&self) -> Amount {
        self.0.trusted_spendable().into()
    }
}
