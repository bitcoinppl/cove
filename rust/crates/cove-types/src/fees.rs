use std::sync::Arc;

use crate::{amount::Amount, color::FfiColor};
use derive_more::{AsRef, Deref, Display, From, Into};
use tracing::debug;

// MARK: FeeRate
//
pub type BdkFeeRate = bitcoin::FeeRate;

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
    serde::Serialize,
    serde::Deserialize,
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
    pub fn from_sat_per_vb(sat_per_vb: f32) -> Self {
        let sat_per_kwu = sat_per_vb * (1000 / 4) as f32;
        let fee_rate = BdkFeeRate::from_sat_per_kwu(sat_per_kwu.ceil() as u64);

        Self(fee_rate)
    }

    pub fn sat_per_vb(&self) -> f32 {
        self.0.to_sat_per_kwu() as f32 / (1000 / 4) as f32
    }
}

// MARK: FeeRateOptions

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptions {
    pub fast: FeeRateOption,
    pub medium: FeeRateOption,
    pub slow: FeeRateOption,
}

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

    #[uniffi::constructor(name = "previewNew")]
    pub fn _ffi_preview_new() -> Self {
        Self {
            fast: FeeRateOption::new(FeeSpeed::Fast, 9.87),
            medium: FeeRateOption::new(FeeSpeed::Medium, 7.22),
            slow: FeeRateOption::new(FeeSpeed::Slow, 2.11),
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
    pub fn new(fee_speed: FeeSpeed, fee_rate: f32) -> Self {
        Self { fee_speed, fee_rate: FeeRate::from_sat_per_vb(fee_rate) }
    }

    pub fn sat_per_vb(&self) -> f32 {
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
#[uniffi::export(Display)]
pub enum FeeSpeed {
    Fast,
    Medium,
    Slow,
    #[display("Custom")]
    Custom {
        duration_mins: u32,
    },
}

impl FeeSpeed {
    pub fn circle_color(&self) -> FfiColor {
        match self {
            FeeSpeed::Fast => FfiColor::Green(Default::default()),
            FeeSpeed::Medium => FfiColor::Yellow(Default::default()),
            FeeSpeed::Slow => FfiColor::Orange(Default::default()),
            FeeSpeed::Custom { .. } => FfiColor::Blue(Default::default()),
        }
    }

    pub fn duration(&self) -> String {
        match self {
            FeeSpeed::Fast => "15 minutes".to_string(),
            FeeSpeed::Medium => "30 minutes".to_string(),
            FeeSpeed::Slow => "1+ hours".to_string(),
            FeeSpeed::Custom { duration_mins } => {
                let duration_mins = *duration_mins;
                if duration_mins < 60_u32 {
                    return format!("{duration_mins} minutes");
                }

                let hours = duration_mins / 60;
                let minutes = duration_mins % 60;

                match (hours, minutes) {
                    (1, 0) => "1 hour".to_string(),
                    (1, _) => "1+ hours".to_string(),
                    (h, 0) => format!("{h} hours"),
                    (h, _) => format!("{h}+ hours"),
                }
            }
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, FeeSpeed::Custom { .. })
    }
}

mod fee_speed_ffi {
    use super::*;

    #[uniffi::export]
    fn fee_speed_to_circle_color(fee_speed: FeeSpeed) -> FfiColor {
        fee_speed.circle_color()
    }

    #[uniffi::export]
    fn fee_speed_duration(fee_speed: FeeSpeed) -> String {
        fee_speed.duration()
    }

    #[uniffi::export]
    fn fee_speed_is_custom(fee_speed: FeeSpeed) -> bool {
        fee_speed.is_custom()
    }
}

// MARK: FeeRateOptionWithTotalFee

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptionsWithTotalFee {
    pub fast: FeeRateOptionWithTotalFee,
    pub medium: FeeRateOptionWithTotalFee,
    pub slow: FeeRateOptionWithTotalFee,
    pub custom: Option<FeeRateOptionWithTotalFee>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Object)]
pub struct FeeRateOptionWithTotalFee {
    pub fee_speed: FeeSpeed,
    pub fee_rate: FeeRate,
    pub total_fee: Amount,
}

impl FeeRateOptionWithTotalFee {
    pub fn new(option: FeeRateOption, total_fee: impl Into<Amount>) -> Self {
        Self { fee_speed: option.fee_speed, fee_rate: option.fee_rate, total_fee: total_fee.into() }
    }
}

#[uniffi::export]
impl FeeRateOptionWithTotalFee {
    #[uniffi::method]
    pub fn is_custom(&self) -> bool {
        self.fee_speed.is_custom()
    }
}

#[uniffi::export]
impl FeeRateOptionsWithTotalFee {
    #[uniffi::method]
    pub fn custom(&self) -> Option<Arc<FeeRateOptionWithTotalFee>> {
        self.custom.map(Arc::new)
    }

    #[uniffi::method]
    pub fn remove_custom_fee(self: Arc<Self>) -> Self {
        Self { fast: self.fast, medium: self.medium, slow: self.slow, custom: None }
    }

    #[uniffi::method]
    pub fn get_fee_rate_with(&self, fee_rate: f32) -> Option<Arc<FeeRateOptionWithTotalFee>> {
        debug!("get_fee_rate_with: {fee_rate}");
        if let Some(custom) = self.custom
            && custom.fee_rate.sat_per_vb() == fee_rate
        {
            return Some(custom.into());
        }

        if self.fast.fee_rate.sat_per_vb() == fee_rate {
            return Some(self.fast.into());
        }

        if self.medium.fee_rate.sat_per_vb() == fee_rate {
            return Some(self.medium.into());
        }

        if self.slow.fee_rate.sat_per_vb() == fee_rate {
            return Some(self.slow.into());
        }

        None
    }

    #[uniffi::method]
    pub fn add_custom_fee_rate(&self, fee_rate: Arc<FeeRateOptionWithTotalFee>) -> Self {
        let fee_rate = Arc::unwrap_or_clone(fee_rate);
        Self { fast: self.fast, medium: self.medium, slow: self.slow, custom: Some(fee_rate) }
    }

    #[uniffi::method]
    pub fn transaction_size(&self) -> u64 {
        let fast_total_fee_in_kwu = self.fast.total_fee.as_sats() * 250;
        let fast_size = fast_total_fee_in_kwu / self.fast.fee_rate.to_sat_per_kwu();

        let medium_total_fee_in_kwu = self.medium.total_fee.as_sats() * 250;
        let medium_size = medium_total_fee_in_kwu / self.medium.fee_rate.to_sat_per_kwu();

        let slow_total_fee_in_kwu = self.slow.total_fee.as_sats() * 250;
        let slow_size = slow_total_fee_in_kwu / self.slow.fee_rate.to_sat_per_kwu();

        (fast_size + medium_size + slow_size) / 3
    }

    #[uniffi::method]
    pub fn calculate_custom_fee_speed(&self, fee_rate: f32) -> FeeSpeed {
        let fee_rate_kwu = ((fee_rate * 1000.0) / 4.0).round() as u64;

        let fast_fee_rate = self.fast.fee_rate.to_sat_per_kwu();
        let medium_fee_rate = self.medium.fee_rate.to_sat_per_kwu();
        let slow_fee_rate = self.slow.fee_rate.to_sat_per_kwu();

        let fee_rate_u64 = fee_rate as u64;
        if fee_rate_u64 == self.fast.sat_per_vb() as u64 {
            return FeeSpeed::Custom { duration_mins: 15 };
        }

        if fee_rate_u64 == self.medium.sat_per_vb() as u64 {
            return FeeSpeed::Custom { duration_mins: 30 };
        }

        if fee_rate_u64 == self.slow.sat_per_vb() as u64 {
            return FeeSpeed::Custom { duration_mins: 90 };
        }

        let mins = match fee_rate_kwu {
            rate if rate <= slow_fee_rate => 150,
            rate if rate == slow_fee_rate => 90,
            rate if rate > slow_fee_rate && rate < medium_fee_rate => 45,
            rate if rate == medium_fee_rate => 30,
            rate if rate > medium_fee_rate && rate < fast_fee_rate => 20,
            rate if rate == fast_fee_rate => 15,
            rate if rate > fast_fee_rate => 10,
            _ => unreachable!(),
        };

        FeeSpeed::Custom { duration_mins: mins }
    }
}

// MARK: FFI
#[uniffi::export]
impl FeeRateOptionWithTotalFee {
    #[uniffi::constructor(name = "new")]
    pub fn _ffi_new(fee_speed: FeeSpeed, fee_rate: Arc<FeeRate>, total_fee: Arc<Amount>) -> Self {
        let fee_rate = Arc::unwrap_or_clone(fee_rate);
        let total_fee = Arc::unwrap_or_clone(total_fee);

        Self { fee_speed, fee_rate, total_fee }
    }

    pub fn fee_speed(&self) -> FeeSpeed {
        self.fee_speed
    }

    pub fn fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    pub fn total_fee(&self) -> Amount {
        self.total_fee
    }

    pub fn sat_per_vb(&self) -> f32 {
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

    #[uniffi::constructor(name = "previewNew")]
    pub fn _ffi_preview_new() -> Self {
        let options = FeeRateOptions::_ffi_preview_new();

        Self {
            fast: FeeRateOptionWithTotalFee::new(options.fast, Amount::from_sat(3050)),
            medium: FeeRateOptionWithTotalFee::new(options.medium, Amount::from_sat(2344)),
            slow: FeeRateOptionWithTotalFee::new(options.slow, Amount::from_sat(1375)),
            custom: None,
        }
    }
}

impl From<FeeRateOptionWithTotalFee> for FeeRateOption {
    fn from(fee_rate: FeeRateOptionWithTotalFee) -> Self {
        FeeRateOption { fee_speed: fee_rate.fee_speed, fee_rate: fee_rate.fee_rate }
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

#[uniffi::export]
fn fee_rate_options_with_total_fee_is_equal(
    lhs: Arc<FeeRateOptionsWithTotalFee>,
    rhs: Arc<FeeRateOptionsWithTotalFee>,
) -> bool {
    lhs == rhs
}
