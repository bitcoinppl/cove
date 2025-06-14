pub mod tap_signer_reader;

use rust_cktap::{apdu::Error as ApduError, commands::CkTransport};
use std::{fmt::Debug, sync::Arc};

// Define error types
#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
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
            ApduError::CiborDe(msg) => TransportError::CiborDe(msg),
            ApduError::CiborValue(msg) => TransportError::CiborValue(msg),
            ApduError::CkTap(error) => TransportError::CkTap(error.into()),
            ApduError::IncorrectSignature(msg) => TransportError::IncorrectSignature(msg),
            ApduError::UnknownCardType(msg) => TransportError::UnknownCardType(msg),
        }
    }
}

impl From<TransportError> for ApduError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::CiborDe(msg) => ApduError::CiborDe(msg),
            TransportError::CiborValue(msg) => ApduError::CiborValue(msg),
            TransportError::CkTap(error) => ApduError::CkTap(error.into()),
            TransportError::IncorrectSignature(msg) => ApduError::IncorrectSignature(msg),
            TransportError::UnknownCardType(msg) => ApduError::UnknownCardType(msg),
            TransportError::CvcChangeError(_) => ApduError::CkTap(CkTapError::BadArguments.into()),
            TransportError::UnknownError(_) => ApduError::CkTap(CkTapError::BadArguments.into()),
        }
    }
}

impl From<rust_cktap::tap_signer::TapSignerError> for TransportError {
    fn from(value: rust_cktap::tap_signer::TapSignerError) -> Self {
        use rust_cktap::tap_signer::TapSignerError as TE;
        match value {
            TE::ApduError(error) => TransportError::from(error),
            TE::CvcChangeError(cvc_change_error) => {
                TransportError::CvcChangeError(cvc_change_error.to_string())
            }
        }
    }
}

impl From<rust_cktap::apdu::CkTapError> for CkTapError {
    fn from(error: rust_cktap::apdu::CkTapError) -> Self {
        use rust_cktap::apdu::CkTapError as CTE;

        match error {
            CTE::UnluckyNumber => CkTapError::UnluckyNumber,
            CTE::BadArguments => CkTapError::BadArguments,
            CTE::BadAuth => CkTapError::BadAuth,
            CTE::NeedsAuth => CkTapError::NeedsAuth,
            CTE::UnknownCommand => CkTapError::UnknownCommand,
            CTE::InvalidCommand => CkTapError::InvalidCommand,
            CTE::InvalidState => CkTapError::InvalidState,
            CTE::WeakNonce => CkTapError::WeakNonce,
            CTE::BadCBOR => CkTapError::BadCBOR,
            CTE::BackupFirst => CkTapError::BackupFirst,
            CTE::RateLimited => CkTapError::RateLimited,
        }
    }
}

impl From<CkTapError> for rust_cktap::apdu::CkTapError {
    fn from(error: CkTapError) -> Self {
        match error {
            CkTapError::UnluckyNumber => rust_cktap::apdu::CkTapError::UnluckyNumber,
            CkTapError::BadArguments => rust_cktap::apdu::CkTapError::BadArguments,
            CkTapError::BadAuth => rust_cktap::apdu::CkTapError::BadAuth,
            CkTapError::NeedsAuth => rust_cktap::apdu::CkTapError::NeedsAuth,
            CkTapError::UnknownCommand => rust_cktap::apdu::CkTapError::UnknownCommand,
            CkTapError::InvalidCommand => rust_cktap::apdu::CkTapError::InvalidCommand,
            CkTapError::InvalidState => rust_cktap::apdu::CkTapError::InvalidState,
            CkTapError::WeakNonce => rust_cktap::apdu::CkTapError::WeakNonce,
            CkTapError::BadCBOR => rust_cktap::apdu::CkTapError::BadCBOR,
            CkTapError::BackupFirst => rust_cktap::apdu::CkTapError::BackupFirst,
            CkTapError::RateLimited => rust_cktap::apdu::CkTapError::RateLimited,
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
