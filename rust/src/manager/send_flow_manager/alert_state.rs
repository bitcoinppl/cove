use cove_types::address::AddressError;

use super::error::SendFlowError;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendFlowWarningKind {
    SmallAmount,
    HighFee,
    VeryHighFee,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendFlowAlertState {
    Error(SendFlowError),
    General { title: String, message: String },
    Warning { kind: SendFlowWarningKind, title: String, message: String },
}

impl SendFlowAlertState {
    pub fn from_address_error(error: AddressError, address: String) -> Self {
        Self::Error(SendFlowError::from_address_error(error, address))
    }
}

#[uniffi::export]
fn send_flow_alert_state_from_address_error(
    error: AddressError,
    address: String,
) -> SendFlowAlertState {
    SendFlowAlertState::from_address_error(error, address)
}

impl From<SendFlowError> for SendFlowAlertState {
    fn from(error: SendFlowError) -> Self {
        Self::Error(error)
    }
}

/// How far a network fee is above normal for the amount it sends
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum)]
pub enum FeeWarningLevel {
    /// At least 5% of the amount sent
    High,

    /// At least 20% of the amount sent
    VeryHigh,
}

/// A network fee as a share of the amount it sends, which the send screens warn about when high
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FeeShare(f64);

impl FeeShare {
    const HIGH_PERCENT: f64 = 5.0;
    const VERY_HIGH_PERCENT: f64 = 20.0;

    /// `None` when nothing is sent, so the share is undefined
    #[allow(clippy::cast_precision_loss)] // a percentage only needs to be roughly right
    pub fn of(fee_sats: u64, amount_sats: u64) -> Option<Self> {
        if amount_sats == 0 {
            return None;
        }

        Some(Self(fee_sats as f64 / amount_sats as f64 * 100.0))
    }

    pub fn warning_level(self) -> Option<FeeWarningLevel> {
        if self.0 >= Self::VERY_HIGH_PERCENT {
            return Some(FeeWarningLevel::VeryHigh);
        }

        (self.0 >= Self::HIGH_PERCENT).then_some(FeeWarningLevel::High)
    }

    /// The share as a whole percentage, such as "25"
    pub fn display_percent(self) -> String {
        format!("{:.0}", self.0)
    }
}
