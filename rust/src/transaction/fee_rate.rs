use bdk_wallet::bitcoin::FeeRate as BdkFeeRate;

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
pub struct FeeRate(BdkFeeRate);

#[uniffi::export]
impl FeeRate {}
