pub mod client;

use std::sync::Arc;

use crate::{color::FfiColor, transaction::Amount};
use bdk_wallet::bitcoin::FeeRate as BdkFeeRate;
use derive_more::{AsRef, Deref, Display, From, Into};

// MARK: FeeRate
//
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
    From,
    Deref,
    AsRef,
    Into,
)]
pub struct FeeRate(BdkFeeRate);

impl FeeRate {
    pub fn preview_new() -> Self {
        let fee_rate = BdkFeeRate::from_sat_per_vb(1).expect("fee rate");
        Self(fee_rate)
    }
}

#[uniffi::export]
impl FeeRate {
    #[uniffi::constructor()]
    pub fn from_sat_per_vb(sat_per_vb: u64) -> Self {
        let fee_rate = BdkFeeRate::from_sat_per_vb(sat_per_vb).expect("fee rate");
        Self(fee_rate)
    }

    pub fn sat_per_vb(&self) -> f64 {
        (self.0.to_sat_per_kwu() as f64) / 250.0
    }
}

// MARK: FeeRateOptions
//

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptions {
    pub fast: FeeRateOption,
    pub medium: FeeRateOption,
    pub slow: FeeRateOption,
}

mod fee_rate_options_ffi {
    use super::*;

    #[uniffi::export]
    impl FeeRateOptions {
        pub fn fast(&self) -> FeeRateOption {
            self.fast
        }

        pub fn medium(&self) -> FeeRateOption {
            self.medium
        }

        pub fn slow(&self) -> FeeRateOption {
            self.slow
        }
    }
}

mod preview_ffi {
    use super::*;

    #[uniffi::export]
    impl FeeRateOptions {
        #[uniffi::constructor]
        fn preview_new() -> Self {
            Self {
                fast: FeeRateOption::new(FeeSpeed::Fast, 10),
                medium: FeeRateOption::new(FeeSpeed::Medium, 7),
                slow: FeeRateOption::new(FeeSpeed::Slow, 2),
            }
        }
    }
}

// MARK: FeeRateOption

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOption {
    pub fee_speed: FeeSpeed,
    pub fee_rate: FeeRate,
}

#[uniffi::export]
impl FeeRateOption {
    #[uniffi::constructor]
    pub fn new(fee_speed: FeeSpeed, fee_rate: u64) -> Self {
        Self {
            fee_speed,
            fee_rate: FeeRate::from_sat_per_vb(fee_rate),
        }
    }

    pub fn sat_per_vb(&self) -> f64 {
        self.fee_rate.sat_per_vb()
    }

    pub fn total_fee(&self, txn_size: u64) -> Option<Arc<Amount>> {
        let amount = self.fee_rate.fee_vb(txn_size)?.into();
        Some(Arc::new(amount))
    }

    pub fn duration(&self) -> String {
        self.fee_speed.duration()
    }

    pub fn speed(&self) -> FeeSpeed {
        self.fee_speed
    }

    pub fn rate(&self) -> FeeRate {
        self.fee_rate
    }
}

// MARK: FeeSpeed
//
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Enum, Display)]
pub enum FeeSpeed {
    Fast,
    Medium,
    Slow,
}

impl FeeSpeed {
    pub fn circle_color(&self) -> FfiColor {
        match self {
            FeeSpeed::Fast => FfiColor::Green(Default::default()),
            FeeSpeed::Medium => FfiColor::Yellow(Default::default()),
            FeeSpeed::Slow => FfiColor::Orange(Default::default()),
        }
    }

    pub fn duration(&self) -> String {
        match self {
            FeeSpeed::Fast => "15 minutes".to_string(),
            FeeSpeed::Medium => "30 minutes".to_string(),
            FeeSpeed::Slow => "1+ hour".to_string(),
        }
    }
}

mod fee_speed_ffi {
    use super::*;

    #[uniffi::export]
    fn fee_speed_to_string(fee_speed: FeeSpeed) -> String {
        fee_speed.to_string()
    }

    #[uniffi::export]
    fn fee_speed_to_circle_color(fee_speed: FeeSpeed) -> FfiColor {
        fee_speed.circle_color()
    }

    #[uniffi::export]
    fn fee_speed_duration(fee_speed: FeeSpeed) -> String {
        fee_speed.duration()
    }
}
