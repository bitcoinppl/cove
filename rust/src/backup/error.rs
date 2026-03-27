#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum BackupError {
    #[error("Password must be at least 20 characters")]
    PasswordTooShort,

    /// Inner error deliberately omitted to prevent oracle attacks
    #[error("Wrong password or corrupted backup file")]
    DecryptionFailed,

    #[error("Not a valid Cove backup file")]
    InvalidFormat,

    #[error("Backup file is too large (max 50 MB)")]
    FileTooLarge,

    #[error("Unsupported backup version {0}, please update the app")]
    UnsupportedVersion(u32),

    #[error("Unsupported backup payload version {0}, please update the app")]
    UnsupportedPayloadVersion(u32),

    #[error("Backup file is truncated or corrupted")]
    Truncated,

    #[error("Failed to encrypt: {0}")]
    Encryption(String),

    #[error("Failed to serialize: {0}")]
    Serialization(String),

    #[error("Failed to deserialize: {0}")]
    Deserialization(String),

    #[error("Failed to gather wallet data: {0}")]
    Gather(String),

    #[error("Failed to restore wallet: {0}")]
    Restore(String),

    #[error("Failed to read keychain: {0}")]
    Keychain(String),

    #[error("Failed to access database: {0}")]
    Database(String),

    #[error("Failed to decompress: {0}")]
    Decompression(String),
}
