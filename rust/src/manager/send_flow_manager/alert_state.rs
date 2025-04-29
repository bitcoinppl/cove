use cove_types::address::AddressError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendFlowAlertState {
    EmptyAddress,
    InvalidNumber,
    InvalidAddress(String),
    WrongNetwork(String),
    NoBalance,
    ZeroAmount,
    InsufficientFunds,
    SendAmountToLow,
    UnableToGetFeeRate,
    UnableToBuildTxn(String),
}

impl SendFlowAlertState {
    pub fn from_address_error(error: AddressError, address: String) -> Self {
        match error {
            AddressError::EmptyAddress => Self::EmptyAddress,
            AddressError::InvalidAddress => Self::InvalidAddress(address),
            AddressError::WrongNetwork { .. } => Self::WrongNetwork(address),
            _ => Self::InvalidAddress(address),
        }
    }
}

#[uniffi::export]
fn address_error_to_alert_state(error: AddressError, address: String) -> SendFlowAlertState {
    SendFlowAlertState::from_address_error(error, address)
}
