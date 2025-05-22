use cove_types::address::AddressError;

use super::error::SendFlowError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendFlowAlertState {
    Error(SendFlowError),
}

impl SendFlowAlertState {
    pub fn from_address_error(error: AddressError, address: String) -> Self {
        Self::Error(SendFlowError::from_address_error(error, address))
    }
}

#[uniffi::export]
fn address_error_to_alert_state(error: AddressError, address: String) -> SendFlowAlertState {
    SendFlowAlertState::from_address_error(error, address)
}

impl From<SendFlowError> for SendFlowAlertState {
    fn from(error: SendFlowError) -> Self {
        Self::Error(error)
    }
}
