#[derive(Debug, thiserror::Error)]
pub enum CsppError {
    #[error("unable to save: {0}")]
    Save(String),
    #[error("unable to encrypt: {0}")]
    Encrypt(String),
    #[error("unable to decrypt: {0}")]
    Decrypt(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("wrong key: decryption failed due to incorrect key")]
    WrongKey,
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("deserialization failed: {0}")]
    Deserialization(String),
}
