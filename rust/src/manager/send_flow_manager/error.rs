use cove_types::{Network, address::AddressError};
use futures::channel::oneshot;

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
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SendFlowBuildTxnError {
    #[error("no amount")]
    NoAmount,

    #[error("no address")]
    NoAddress,

    #[error("watch only")]
    WatchOnly,

    #[error(transparent)]
    Actor(#[from] oneshot::Canceled),

    #[error(transparent)]
    WalletManager(#[from] WalletManagerError),
}

impl From<SendFlowBuildTxnError> for SendFlowError {
    fn from(error: SendFlowBuildTxnError) -> Self {
        match error {
            SendFlowBuildTxnError::WalletManager(error) => Self::from(error),
            error => Self::UnableToBuildTxn(error.to_string()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SendFlowFeeDetailsError {
    #[error(transparent)]
    Psbt(#[from] cove_types::psbt::PsbtError),

    #[error(transparent)]
    SendFlow(#[from] SendFlowError),
}

impl From<SendFlowFeeDetailsError> for SendFlowError {
    fn from(error: SendFlowFeeDetailsError) -> Self {
        Self::UnableToGetFeeDetails(error.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SendFlowMaxSendError {
    #[error(transparent)]
    FeeDetails(#[from] SendFlowFeeDetailsError),

    #[error(transparent)]
    SendFlow(#[from] SendFlowError),

    #[error(transparent)]
    WalletManager(#[from] WalletManagerError),
}

impl From<SendFlowMaxSendError> for SendFlowError {
    fn from(error: SendFlowMaxSendError) -> Self {
        match error {
            SendFlowMaxSendError::SendFlow(error @ Self::SendBelowDustLimit)
            | SendFlowMaxSendError::SendFlow(error @ Self::InsufficientFunds) => error,
            SendFlowMaxSendError::WalletManager(error) => match Self::from(error) {
                error @ Self::SendBelowDustLimit | error @ Self::InsufficientFunds => error,
                error => Self::UnableToGetMaxSend(error.to_string()),
            },
            error => Self::UnableToGetMaxSend(error.to_string()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SendFlowSaveUnsignedTransactionError {
    #[error(transparent)]
    WalletManager(#[from] WalletManagerError),
}

impl From<SendFlowSaveUnsignedTransactionError> for SendFlowError {
    fn from(error: SendFlowSaveUnsignedTransactionError) -> Self {
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
