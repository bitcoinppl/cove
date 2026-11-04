#[derive(Debug, thiserror::Error)]
pub enum CsppError {
    #[error("unable to save: {0}")]
    Save(String),
    /// The store could not read an entry, so whether it exists is unknown
    #[error("unable to read: {0}")]
    Read(String),
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
    #[error("wallet backup version {0} is not supported")]
    UnsupportedWalletBackupVersion(u32),
    #[error("wallet backup version {version} does not support its decrypted content")]
    WalletBackupVersionMismatch { version: u32 },
}

impl CsppError {
    /// Combines a failed write with the failed attempt to undo it
    ///
    /// A read failure in either leaves the stored outcome unknown until the keychain can be read
    /// again, so the combined error stays a retryable read error instead of a failed write
    pub(crate) fn with_failed_rollback(self, rollback: Self, context: &str) -> Self {
        let read_failed = matches!(self, Self::Read(_)) || matches!(rollback, Self::Read(_));
        let message = format!("{self}; {context}: {rollback}");

        if read_failed { Self::Read(message) } else { Self::Save(message) }
    }
}

impl From<crate::backup_data::UnsupportedWalletBackupVersion> for CsppError {
    fn from(unsupported: crate::backup_data::UnsupportedWalletBackupVersion) -> Self {
        Self::UnsupportedWalletBackupVersion(unsupported.0)
    }
}
