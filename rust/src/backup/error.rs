use crate::wallet_identity::WalletIdentityError;
use cove_types::WalletId;

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

    /// The wallet id is already used by local wallet state or restore artifacts
    #[error("Wallet id is already occupied: {0}")]
    WalletIdOccupied(WalletId),

    /// The backup contains a wallet id that cannot be used as a local path component
    #[error("Invalid wallet id: {0}")]
    InvalidWalletId(String),

    /// The local artifact snapshot changed after the import was prepared
    #[error("Import approval is stale for wallet id: {0}")]
    ImportApprovalStale(WalletId),

    /// Destructive cleanup requires a one-use import approval
    #[error("Import approval is required before removing existing wallet artifacts: {0}")]
    ImportApprovalRequired(WalletId),

    /// An import preparation or approval object was already consumed
    #[error("Import approval has already been used")]
    ImportApprovalUsed,

    #[error("Failed to decompress: {0}")]
    Decompression(String),
}

impl From<WalletIdentityError> for BackupError {
    fn from(error: WalletIdentityError) -> Self {
        match error {
            WalletIdentityError::Database(error) => Self::Database(error.to_string()),
            error => Self::Restore(error.to_string()),
        }
    }
}
