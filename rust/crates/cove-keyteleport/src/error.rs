/// All errors that can occur during Key Teleport operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("checksum mismatch — wrong key or corrupted data")]
    ChecksumMismatch,

    #[error("invalid receiver packet: {0}")]
    InvalidReceiverPacket(String),

    #[error("invalid sender packet: {0}")]
    InvalidSenderPacket(String),

    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    #[error("invalid BBQr: {0}")]
    InvalidBbqr(String),

    #[error("secp256k1 error: {0}")]
    Secp(String),
}

impl From<bitcoin::secp256k1::Error> for Error {
    fn from(e: bitcoin::secp256k1::Error) -> Self {
        Error::Secp(e.to_string())
    }
}
