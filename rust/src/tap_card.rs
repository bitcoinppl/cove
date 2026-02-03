pub mod tap_signer_reader;

use rust_cktap::{apdu::Error as ApduError, commands::CkTransport};
use std::{fmt::Debug, sync::Arc};

// Define error types
#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum TransportError {
    #[error("CiborDe: {0}")]
    CiborDe(String),

    #[error("CiborValue: {0}")]
    CiborValue(String),

    #[error("CkTapError: {0}")]
    CkTap(CkTapError),

    #[error("IncorrectSignature: {0}")]
    IncorrectSignature(String),

    #[error("UnknownCardType: {0}")]
    UnknownCardType(String),

    #[error("CvcChangeError: {0}")]
    CvcChangeError(String),

    #[error("UnknownError: {0}")]
    UnknownError(String),
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CkTapError {
    #[error("Rare or unlucky value used/occurred. Start again")]
    UnluckyNumber,
    #[error("Invalid/incorrect/incomplete arguments provided to command")]
    BadArguments,
    #[error("Authentication details (CVC/epubkey) are wrong")]
    BadAuth,
    #[error("Command requires auth, and none was provided")]
    NeedsAuth,
    #[error("The 'cmd' field is an unsupported command")]
    UnknownCommand,
    #[error("Command is not valid at this time, no point retrying")]
    InvalidCommand,
    #[error("You can't do that right now when card is in this state")]
    InvalidState,
    #[error("Nonce is not unique-looking enough")]
    WeakNonce,
    #[error("Unable to decode CBOR data stream")]
    BadCBOR,
    #[error("Can't change CVC without doing a backup first (TAPSIGNER only)")]
    BackupFirst,
    #[error("Due to auth failures, delay required")]
    RateLimited,
}

// Define the callback interface that Swift will implement
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait TapcardTransportProtocol: Send + Sync + std::fmt::Debug + 'static {
    fn set_message(&self, message: String);
    fn append_message(&self, message: String);
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TransportError>;
}

// Implement the CkTransport trait for our callback-based transport
#[derive(Debug, Clone)]
pub struct TapcardTransport(Arc<Box<dyn TapcardTransportProtocol>>);

impl TapcardTransport {
    pub fn set_message(&self, message: String) {
        self.0.set_message(message);
    }

    #[allow(dead_code)]
    pub fn append_message(&self, message: String) {
        self.0.append_message(message);
    }
}

impl CkTransport for TapcardTransport {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, ApduError> {
        let response_bytes = self.0.transmit_apdu(command_apdu).await?;
        Ok(response_bytes)
    }
}

// Convert ApduError type to TransportError for UniFFI
impl From<ApduError> for TransportError {
    fn from(error: ApduError) -> Self {
        match error {
            ApduError::CiborDe(msg) => Self::CiborDe(msg),
            ApduError::CiborValue(msg) => Self::CiborValue(msg),
            ApduError::CkTap(error) => Self::CkTap(error.into()),
            ApduError::IncorrectSignature(msg) => Self::IncorrectSignature(msg),
            ApduError::UnknownCardType(msg) => Self::UnknownCardType(msg),
        }
    }
}

impl From<TransportError> for ApduError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::CiborDe(msg) => Self::CiborDe(msg),
            TransportError::CiborValue(msg) => Self::CiborValue(msg),
            TransportError::CkTap(error) => Self::CkTap(error.into()),
            TransportError::IncorrectSignature(msg) => Self::IncorrectSignature(msg),
            TransportError::UnknownCardType(msg) => Self::UnknownCardType(msg),
            TransportError::CvcChangeError(_) => Self::CkTap(CkTapError::BadArguments.into()),
            TransportError::UnknownError(_) => Self::CkTap(CkTapError::BadArguments.into()),
        }
    }
}

impl From<rust_cktap::tap_signer::TapSignerError> for TransportError {
    fn from(value: rust_cktap::tap_signer::TapSignerError) -> Self {
        use rust_cktap::tap_signer::TapSignerError as TE;
        match value {
            TE::ApduError(error) => Self::from(error),
            TE::CvcChangeError(cvc_change_error) => {
                Self::CvcChangeError(cvc_change_error.to_string())
            }
        }
    }
}

impl From<rust_cktap::apdu::CkTapError> for CkTapError {
    fn from(error: rust_cktap::apdu::CkTapError) -> Self {
        use rust_cktap::apdu::CkTapError as CTE;

        match error {
            CTE::UnluckyNumber => Self::UnluckyNumber,
            CTE::BadArguments => Self::BadArguments,
            CTE::BadAuth => Self::BadAuth,
            CTE::NeedsAuth => Self::NeedsAuth,
            CTE::UnknownCommand => Self::UnknownCommand,
            CTE::InvalidCommand => Self::InvalidCommand,
            CTE::InvalidState => Self::InvalidState,
            CTE::WeakNonce => Self::WeakNonce,
            CTE::BadCBOR => Self::BadCBOR,
            CTE::BackupFirst => Self::BackupFirst,
            CTE::RateLimited => Self::RateLimited,
        }
    }
}

impl From<CkTapError> for rust_cktap::apdu::CkTapError {
    fn from(error: CkTapError) -> Self {
        match error {
            CkTapError::UnluckyNumber => Self::UnluckyNumber,
            CkTapError::BadArguments => Self::BadArguments,
            CkTapError::BadAuth => Self::BadAuth,
            CkTapError::NeedsAuth => Self::NeedsAuth,
            CkTapError::UnknownCommand => Self::UnknownCommand,
            CkTapError::InvalidCommand => Self::InvalidCommand,
            CkTapError::InvalidState => Self::InvalidState,
            CkTapError::WeakNonce => Self::WeakNonce,
            CkTapError::BadCBOR => Self::BadCBOR,
            CkTapError::BackupFirst => Self::BackupFirst,
            CkTapError::RateLimited => Self::RateLimited,
        }
    }
}

#[uniffi::export]
pub fn create_transport_error_from_code(code: u16, message: String) -> TransportError {
    use rust_cktap::apdu::CkTapError as CTE;
    let error = CTE::error_from_code(code);

    if let Some(error) = error {
        let cktap_error = CkTapError::from(error);
        return TransportError::CkTap(cktap_error);
    }

    TransportError::CiborDe(message)
}

#[uniffi::export]
pub fn is_valid_chain_code(chain_code: String) -> bool {
    let Ok(chain_code) = hex::decode(chain_code) else { return false };
    chain_code.len() == 32
}
