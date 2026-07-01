use std::fmt::Display;

use cove_types::{Network, address::AddressError};
use tracing::warn;

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

    #[error("unable to get max send")]
    UnableToGetMaxSend,

    #[error("insufficient funds")]
    InsufficientFunds,

    #[error("send amount to low")]
    SendAmountToLow,

    #[error("unable to get fee rate")]
    UnableToGetFeeRate,

    #[error("unable to build txn")]
    UnableToBuildTxn,

    #[error("unable to save unsigned transaction")]
    UnableToSaveUnsignedTransaction,

    #[error("wallet manager error")]
    WalletManager,

    #[error("unable to get fee details")]
    UnableToGetFeeDetails,
}

impl SendFlowError {
    pub fn unable_to_get_max_send(error: impl Display) -> Self {
        warn!("Unable to calculate max send: {error}");
        Self::UnableToGetMaxSend
    }

    pub fn unable_to_build_txn(error: impl Display) -> Self {
        warn!("Unable to build transaction: {error}");
        Self::UnableToBuildTxn
    }

    pub fn unable_to_save_unsigned_transaction(error: impl Display) -> Self {
        warn!("Unable to save unsigned transaction: {error}");
        Self::UnableToSaveUnsignedTransaction
    }

    pub fn unable_to_get_fee_details(error: impl Display) -> Self {
        warn!("Unable to get fee details: {error}");
        Self::UnableToGetFeeDetails
    }

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

impl From<WalletManagerError> for SendFlowError {
    fn from(error: WalletManagerError) -> Self {
        match error {
            WalletManagerError::InsufficientFunds(error) => {
                warn!("Wallet manager reported insufficient funds: {error}");
                Self::InsufficientFunds
            }

            error => {
                warn!("Wallet manager send flow error: {error}");
                Self::WalletManager
            }
        }
    }
}
