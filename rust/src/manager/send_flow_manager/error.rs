use cove_types::{Network, address::AddressError};

use crate::manager::wallet_manager::WalletManagerError;

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
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

    #[error("send amount is below the dust limit")]
    SendBelowDustLimit,

    #[error("unable to get fee rate")]
    UnableToGetFeeRate,

    #[error("unable to build txn: {0}")]
    UnableToBuildTxn(String),

    #[error("unable to save unsigned transaction")]
    UnableToSaveUnsignedTransaction(String),

    #[error(transparent)]
    WalletManager(WalletManagerError),

    #[error("unable to get fee details: {0}")]
    UnableToGetFeeDetails(String),
}

impl From<WalletManagerError> for SendFlowError {
    fn from(error: WalletManagerError) -> Self {
        match error {
            WalletManagerError::OutputBelowDustLimit => Self::SendBelowDustLimit,
            WalletManagerError::InsufficientFunds(_) => Self::InsufficientFunds,
            error => Self::WalletManager(error),
        }
    }
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

    pub(crate) fn unable_to_build_txn(error: impl std::fmt::Display) -> Self {
        Self::UnableToBuildTxn(error.to_string())
    }

    pub(crate) fn unable_to_get_fee_details(error: impl std::fmt::Display) -> Self {
        Self::UnableToGetFeeDetails(error.to_string())
    }

    pub(crate) fn unable_to_get_max_send(error: impl std::fmt::Display) -> Self {
        Self::UnableToGetMaxSend(error.to_string())
    }

    pub(crate) fn unable_to_save_unsigned_transaction(error: impl std::fmt::Display) -> Self {
        Self::UnableToSaveUnsignedTransaction(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{SendFlowError, WalletManagerError};

    #[test]
    fn wallet_output_below_dust_maps_to_send_below_dust() {
        let error = SendFlowError::from(WalletManagerError::OutputBelowDustLimit);

        assert!(matches!(error, SendFlowError::SendBelowDustLimit));
    }

    #[test]
    fn wallet_insufficient_funds_maps_to_send_insufficient_funds() {
        let error = SendFlowError::from(WalletManagerError::InsufficientFunds(
            "not enough funds".to_string(),
        ));

        assert!(matches!(error, SendFlowError::InsufficientFunds));
    }
}
