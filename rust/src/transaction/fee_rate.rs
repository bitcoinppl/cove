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

impl FeeRate {
    pub fn preview_new() -> Self {
        let fee_rate = BdkFeeRate::from_sat_per_vb(1).expect("fee rate");
        Self(fee_rate)
    }
}

#[uniffi::export]
impl FeeRate {}
