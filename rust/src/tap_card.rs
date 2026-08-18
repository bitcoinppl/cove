pub mod tap_signer_reader;

use rust_cktap::{CkTapError as CkTapTransportError, CkTransport};
use std::{fmt::Debug, sync::Arc};

/// Errors reported while communicating with a CkTap card
#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum TransportError {
    /// The response could not be decoded as CBOR
    #[error("CiborDe: {0}")]
    CiborDe(String),

    /// A CBOR value could not be represented
    #[error("CiborValue: {0}")]
    CiborValue(String),

    /// A protocol-defined card rejection
    #[error("CkTapError: {0}")]
    CkTap(CkTapError),

    /// The APDU exchange did not establish whether the command reached the card
    #[error("Transport: {0}")]
    Transport(String),

    /// Card authenticity verification failed
    #[error("IncorrectSignature: {0}")]
    IncorrectSignature(String),

    /// The card type is not supported by this operation
    #[error("UnknownCardType: {0}")]
    UnknownCardType(String),

    /// The CVC could not be used for a card command
    #[error("CvcChangeError: {0}")]
    CvcChangeError(String),

    /// The card returned a status word that this version does not know
    #[error("UnknownStatusWord ({code}): {detail}")]
    UnknownStatusWord { code: u16, detail: String },

    /// The card returned a protocol error code that this version does not know
    #[error("UnknownCardErrorCode ({code}): {detail}")]
    UnknownCardErrorCode { code: u16, detail: String },

    /// An error without a more specific public classification
    #[error("UnknownError: {0}")]
    UnknownError(String),
}

/// Card errors defined by the CkTap protocol
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CkTapError {
    /// The card reported a rare or unlucky value
    #[error("Rare or unlucky value used/occurred. Start again")]
    UnluckyNumber,
    /// The command arguments are invalid
    #[error("Invalid/incorrect/incomplete arguments provided to command")]
    BadArguments,
    /// Authentication details are invalid
    #[error("Authentication details (CVC/epubkey) are wrong")]
    BadAuth,
    /// The command requires authentication
    #[error("Command requires auth, and none was provided")]
    NeedsAuth,
    /// The card does not recognize the command
    #[error("The 'cmd' field is an unsupported command")]
    UnknownCommand,
    /// The command cannot be retried in the current state
    #[error("Command is not valid at this time, no point retrying")]
    InvalidCommand,
    /// The card cannot run the command in its current state
    #[error("You can't do that right now when card is in this state")]
    InvalidState,
    /// The card rejected a nonce as too weak
    #[error("Nonce is not unique-looking enough")]
    WeakNonce,
    /// The card could not decode the CBOR request
    #[error("Unable to decode CBOR data stream")]
    BadCBOR,
    /// The card requires a backup before changing its CVC
    #[error("Can't change CVC without doing a backup first (TAPSIGNER only)")]
    BackupFirst,
    /// The card requires an authentication delay
    #[error("Due to auth failures, delay required")]
    RateLimited,
}

/// Transport callback implemented by the mobile NFC layers
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait TapcardTransportProtocol: Send + Sync + std::fmt::Debug + 'static {
    /// Show a progress message to the user
    fn set_message(&self, message: String);
    /// Append a progress message to the user-visible status
    fn append_message(&self, message: String);
    /// Exchange one APDU with the card
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TransportError>;
}

/// Adapter from the mobile callback interface to rust-cktap's transport trait
#[derive(Debug, Clone)]
pub struct TapcardTransport(Arc<Box<dyn TapcardTransportProtocol>>);

impl TapcardTransport {
    /// Set the current user-visible transport message
    pub fn set_message(&self, message: String) {
        self.0.set_message(message);
    }
}

#[async_trait::async_trait]
impl CkTransport for TapcardTransport {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, CkTapTransportError> {
        self.0.transmit_apdu(command_apdu).await.map_err(Into::into)
    }
}

impl From<CkTapTransportError> for TransportError {
    fn from(error: CkTapTransportError) -> Self {
        match error {
            CkTapTransportError::Card(error) => Self::CkTap(error.into()),
            CkTapTransportError::CborDe(message) => Self::CiborDe(message),
            CkTapTransportError::CborValue(message) => Self::CiborValue(message),
            CkTapTransportError::Transport(message) => Self::Transport(message),
            CkTapTransportError::UnknownErrorCode { code, message } => {
                Self::UnknownCardErrorCode { code, detail: message }
            }
            CkTapTransportError::UnknownCardType => {
                Self::UnknownCardType("rust-cktap reported an unknown card type".to_string())
            }
        }
    }
}

impl From<TransportError> for CkTapTransportError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::CiborDe(message) => Self::CborDe(message),
            TransportError::CiborValue(message) => Self::CborValue(message),
            TransportError::CkTap(error) => Self::Card(error.into()),
            TransportError::Transport(message) => Self::Transport(message),
            TransportError::UnknownCardType(_) => Self::UnknownCardType,
            TransportError::IncorrectSignature(message)
            | TransportError::CvcChangeError(message)
            | TransportError::UnknownError(message) => Self::Transport(message),
            TransportError::UnknownCardErrorCode { code, detail } => {
                Self::UnknownErrorCode { code, message: detail }
            }

            // rust-cktap has no APDU status-word variant at the pinned revision
            TransportError::UnknownStatusWord { code, detail } => {
                Self::Transport(format!("unknown APDU status word ({code:#06x}): {detail}"))
            }
        }
    }
}

impl From<rust_cktap::CardError> for CkTapError {
    fn from(error: rust_cktap::CardError) -> Self {
        use rust_cktap::CardError as Card;

        match error {
            Card::UnluckyNumber => Self::UnluckyNumber,
            Card::BadArguments => Self::BadArguments,
            Card::BadAuth => Self::BadAuth,
            Card::NeedsAuth => Self::NeedsAuth,
            Card::UnknownCommand => Self::UnknownCommand,
            Card::InvalidCommand => Self::InvalidCommand,
            Card::InvalidState => Self::InvalidState,
            Card::WeakNonce => Self::WeakNonce,
            Card::BadCBOR => Self::BadCBOR,
            Card::BackupFirst => Self::BackupFirst,
            Card::RateLimited => Self::RateLimited,
        }
    }
}

impl From<CkTapError> for rust_cktap::CardError {
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

impl From<rust_cktap::StatusError> for TransportError {
    fn from(error: rust_cktap::StatusError) -> Self {
        match error {
            rust_cktap::StatusError::CkTap(error) => error.into(),
            rust_cktap::StatusError::KeyFromSlice(error) => Self::UnknownError(error.to_string()),
        }
    }
}

impl From<rust_cktap::CertsError> for TransportError {
    fn from(error: rust_cktap::CertsError) -> Self {
        match error {
            rust_cktap::CertsError::CkTap(error) => error.into(),
            rust_cktap::CertsError::InvalidRootCert(message) => Self::IncorrectSignature(message),
            rust_cktap::CertsError::Secp256k1(error) => Self::IncorrectSignature(error.to_string()),
            rust_cktap::CertsError::KeyFromSlice(error) => {
                Self::IncorrectSignature(error.to_string())
            }
        }
    }
}

impl From<rust_cktap::DeriveError> for TransportError {
    fn from(error: rust_cktap::DeriveError) -> Self {
        match error {
            rust_cktap::DeriveError::CkTap(error) => error.into(),
            rust_cktap::DeriveError::Secp256k1(error) => Self::UnknownError(error.to_string()),
            rust_cktap::DeriveError::KeyFromSlice(error) => Self::UnknownError(error.to_string()),
            rust_cktap::DeriveError::InvalidChainCode(message) => Self::UnknownError(message),
        }
    }
}

impl From<rust_cktap::XpubError> for TransportError {
    fn from(error: rust_cktap::XpubError) -> Self {
        match error {
            rust_cktap::XpubError::CkTap(error) => error.into(),
            rust_cktap::XpubError::Bip32(error) => Self::UnknownError(error.to_string()),
        }
    }
}

impl From<rust_cktap::ChangeError> for TransportError {
    fn from(error: rust_cktap::ChangeError) -> Self {
        match error {
            rust_cktap::ChangeError::CkTap(error) => error.into(),
            rust_cktap::ChangeError::SameAsOld => {
                Self::CvcChangeError("new CVC is the same as the old one".to_string())
            }
        }
    }
}

impl From<rust_cktap::SignPsbtError> for TransportError {
    fn from(error: rust_cktap::SignPsbtError) -> Self {
        match error {
            rust_cktap::SignPsbtError::CkTap(error) => error.into(),
            other => Self::UnknownError(other.to_string()),
        }
    }
}

/// Convert a card protocol error code into a typed error
#[uniffi::export]
fn create_transport_error_from_code(code: u16, message: String) -> TransportError {
    match rust_cktap::CardError::error_from_code(code) {
        Some(error) => TransportError::CkTap(error.into()),
        None => TransportError::UnknownCardErrorCode { code, detail: message },
    }
}

/// Check whether a hexadecimal chain code decodes to exactly 32 bytes
#[uniffi::export]
pub fn is_valid_chain_code(chain_code: String) -> bool {
    let Ok(chain_code) = hex::decode(chain_code) else { return false };
    chain_code.len() == 32
}
