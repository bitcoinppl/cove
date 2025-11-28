use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi::export(Display)]
pub enum UrError {
    #[error("Failed to encode CBOR: {0}")]
    CborEncodeError(String),

    #[error("Failed to decode CBOR: {0}")]
    CborDecodeError(String),

    #[error("Invalid field: {0}")]
    InvalidField(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid key data length: expected {expected}, got {actual}")]
    InvalidKeyDataLength { expected: u64, actual: u64 },

    #[error("Invalid tag: expected {expected}, got {actual}")]
    InvalidTag { expected: u64, actual: u64 },

    #[error("Invalid payload length: {0}")]
    InvalidPayloadLength(String),

    #[error("Failed to parse UR: {0}")]
    UrParseError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Invalid key data: {0}")]
    InvalidKeyData(String),

    #[error("Master key not allowed - use account-level key instead")]
    MasterKeyNotAllowed,

    #[error("Invalid UTF-8: {0}")]
    InvalidUtf8(String),
}

pub type Result<T> = std::result::Result<T, UrError>;

/// Helper trait to convert any error to UrError
pub trait ToUrError<T> {
    fn map_err_cbor_encode(self) -> Result<T>;
    fn map_err_cbor_decode(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> ToUrError<T> for std::result::Result<T, E> {
    fn map_err_cbor_encode(self) -> Result<T> {
        self.map_err(|e| UrError::CborEncodeError(e.to_string()))
    }

    fn map_err_cbor_decode(self) -> Result<T> {
        self.map_err(|e| UrError::CborDecodeError(e.to_string()))
    }
}
