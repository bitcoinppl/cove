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
}
