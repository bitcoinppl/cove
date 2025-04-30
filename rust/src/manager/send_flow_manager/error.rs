use cove_types::{Network, address::AddressError};

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum SendFlowError {
    #[error("empty address")]
    EmptyAddress,

    #[error("invalid number")]
    InvalidNumber,

    #[error("invalid address: {0}")]
    InvalidAddress(String),

    #[error("wrong network {address} is for network: {valid_for}, current network: {current}")]
    WrongNetwork { address: String, valid_for: Network, current: Network },

    #[error("no balance")]
    NoBalance,

    #[error("zero amount")]
    ZeroAmount,

    #[error("insufficient funds")]
    UnableToGetMaxSend(String),

    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("send amount to low")]
    SendAmountToLow,

    #[error("unable to get fee rate")]
    UnableToGetFeeRate,

    #[error("unable to build txn: {0}")]
    UnableToBuildTxn(String),
}

impl SendFlowError {
    pub fn from_address_error(error: AddressError, address: String) -> Self {
        match error {
            AddressError::EmptyAddress => Self::EmptyAddress,
            AddressError::InvalidAddress => Self::InvalidAddress(address),
            AddressError::WrongNetwork { current, valid_for } => {
                Self::WrongNetwork { address, valid_for, current }
            }

            _ => Self::InvalidAddress(address),
        }
    }
}

#[uniffi::export]
fn describe_send_flow_error(error: SendFlowError) -> String {
    error.to_string()
}
