pub mod client;

use std::sync::Arc;

use crate::{color::FfiColor, transaction::Amount};
use derive_more::{AsRef, Deref, Display, From, Into};

// MARK: FeeRate
//
pub type BdkFeeRate = bitcoin_units::FeeRate;

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
        pub fn preview_new() -> Self {
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

    pub fn duration(&self) -> String {
        self.fee_speed.duration()
    }

    pub fn fee_speed(&self) -> FeeSpeed {
        self.fee_speed
    }

    pub fn fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    pub fn is_equal(&self, rhs: &FeeRateOption) -> bool {
        self.fee_speed == rhs.fee_speed && self.fee_rate.sat_per_vb() == rhs.fee_rate.sat_per_vb()
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
            FeeSpeed::Slow => "1+ hours".to_string(),
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

// MARK: FeeRateOptionWithTotalFee

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptionsWithTotalFee {
    pub fast: FeeRateOptionWithTotalFee,
    pub medium: FeeRateOptionWithTotalFee,
    pub slow: FeeRateOptionWithTotalFee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptionWithTotalFee {
    pub fee_speed: FeeSpeed,
    pub fee_rate: FeeRate,
    pub total_fee: Amount,
}

impl FeeRateOptionWithTotalFee {
    pub fn new(option: FeeRateOption, total_fee: impl Into<Amount>) -> Self {
        Self {
            fee_speed: option.fee_speed,
            fee_rate: option.fee_rate,
            total_fee: total_fee.into(),
        }
    }
}

mod fee_rate_option_with_total_fee_ffi {
    use super::*;

    #[uniffi::export]
    impl FeeRateOptionWithTotalFee {
        pub fn fee_speed(&self) -> FeeSpeed {
            self.fee_speed
        }

        pub fn fee_rate(&self) -> FeeRate {
            self.fee_rate
        }

        pub fn total_fee(&self) -> Amount {
            self.total_fee
        }

        pub fn sat_per_vb(&self) -> f64 {
            self.fee_rate.sat_per_vb()
        }

        pub fn duration(&self) -> String {
            self.fee_speed.duration()
        }

        pub fn fee_rate_options(&self) -> FeeRateOption {
            (*self).into()
        }

        pub fn is_equal(&self, rhs: Arc<FeeRateOptionWithTotalFee>) -> bool {
            self.fee_speed == rhs.fee_speed
                && self.fee_rate == rhs.fee_rate
                && self.total_fee == rhs.total_fee
        }
    }

    #[uniffi::export]
    impl FeeRateOptionsWithTotalFee {
        pub fn fast(&self) -> FeeRateOptionWithTotalFee {
            self.fast
        }

        pub fn medium(&self) -> FeeRateOptionWithTotalFee {
            self.medium
        }

        pub fn slow(&self) -> FeeRateOptionWithTotalFee {
            self.slow
        }

        pub fn fee_rate_options(&self) -> FeeRateOptions {
            (*self).into()
        }

        #[uniffi::constructor]
        fn preview_new() -> Self {
            let options = FeeRateOptions::preview_new();

            Self {
                fast: FeeRateOptionWithTotalFee::new(options.fast, Amount::from_sat(3050)),
                medium: FeeRateOptionWithTotalFee::new(options.medium, Amount::from_sat(2344)),
                slow: FeeRateOptionWithTotalFee::new(options.slow, Amount::from_sat(1375)),
            }
        }
    }
}

impl From<FeeRateOptionWithTotalFee> for FeeRateOption {
    fn from(fee_rate: FeeRateOptionWithTotalFee) -> Self {
        FeeRateOption {
            fee_speed: fee_rate.fee_speed,
            fee_rate: fee_rate.fee_rate,
        }
    }
}

impl From<FeeRateOptionsWithTotalFee> for FeeRateOptions {
    fn from(fee_rates: FeeRateOptionsWithTotalFee) -> Self {
        FeeRateOptions {
            fast: fee_rates.fast.into(),
            medium: fee_rates.medium.into(),
            slow: fee_rates.slow.into(),
        }
    }
}
