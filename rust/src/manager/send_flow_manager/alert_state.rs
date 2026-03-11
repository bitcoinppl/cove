use cove_types::address::AddressError;

use super::error::SendFlowError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendFlowAlertState {
    Error(SendFlowError),
    General { title: String, message: String },
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
