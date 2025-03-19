pub mod tap_signer_reader;

use rust_cktap::{apdu::Error as ApduError, commands::CkTransport};
use std::fmt::Debug;

// Define error types
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TapSignerError {
    #[error("CiborDe: {0}")]
    CiborDe(String),

    #[error("CiborValue: {0}")]
    CiborValue(String),

    #[error("CkTap: erro: {error}, code: {code}")]
    CkTap { error: String, code: u64 },

    #[error("IncorrectSignature: {0}")]
    IncorrectSignature(String),

    #[error("UnknownCardType: {0}")]
    UnknownCardType(String),

    #[error("CvcChangeError: {0}")]
    CvcChangeError(String),
}

// Define the callback interface that Swift will implement
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait TapcardTransportProtocol: Send + Sync + std::fmt::Debug + 'static {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TapSignerError>;
}

// Implement the CkTransport trait for our callback-based transport
#[derive(Debug)]
pub struct TapcardTransport(Box<dyn TapcardTransportProtocol>);

impl CkTransport for TapcardTransport {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, ApduError> {
        let response_bytes = self.0.transmit_apdu(command_apdu).await?;
        Ok(response_bytes)
    }
}

// Convert ApduError type to TransportError for UniFFI
impl From<ApduError> for TapSignerError {
    fn from(error: ApduError) -> Self {
        match error {
            ApduError::CiborDe(msg) => TapSignerError::CiborDe(msg),
            ApduError::CiborValue(msg) => TapSignerError::CiborValue(msg),
            ApduError::CkTap { error, code } => TapSignerError::CkTap {
                error,
                code: code as u64,
            },
            ApduError::IncorrectSignature(msg) => TapSignerError::IncorrectSignature(msg),
            ApduError::UnknownCardType(msg) => TapSignerError::UnknownCardType(msg),
        }
    }
}

impl From<TapSignerError> for ApduError {
    fn from(error: TapSignerError) -> Self {
        match error {
            TapSignerError::CiborDe(msg) => ApduError::CiborDe(msg),
            TapSignerError::CiborValue(msg) => ApduError::CiborValue(msg),
            TapSignerError::CkTap { error, code } => ApduError::CkTap {
                error,
                code: code as usize,
            },
            TapSignerError::IncorrectSignature(msg) => ApduError::IncorrectSignature(msg),
            TapSignerError::UnknownCardType(msg) => ApduError::UnknownCardType(msg),
            TapSignerError::CvcChangeError(error) => ApduError::CkTap {
                error: error.to_string(),
                code: 0,
            },
        }
    }
}

impl From<rust_cktap::tap_signer::TapSignerError> for TapSignerError {
    fn from(value: rust_cktap::tap_signer::TapSignerError) -> Self {
        use rust_cktap::tap_signer::TapSignerError as TE;
        match value {
            TE::ApduError(error) => TapSignerError::from(error),
            TE::CvcChangeError(cvc_change_error) => {
                TapSignerError::CvcChangeError(cvc_change_error.to_string())
            }
        }
    }
}
