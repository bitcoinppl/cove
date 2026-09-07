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

const NEWER_VERSION_MESSAGE: &str =
    "This backup was created by a newer version of Cove. Update Cove and try again.";
const REVIEW_AGAIN_MESSAGE: &str = "Review the backup again and try again.";

#[uniffi::export]
impl BackupError {
    /// Text safe to show the user: it never includes the inner error payloads that `Display` carries for logs
    pub fn user_message(&self) -> String {
        match self {
            Self::PasswordTooShort => "The backup password is too short.".to_string(),
            Self::DecryptionFailed => {
                "The backup password is incorrect, or the backup is damaged.".to_string()
            }
            Self::InvalidFormat => "The selected file is not a valid Cove backup.".to_string(),
            Self::FileTooLarge => "The backup file is too large.".to_string(),
            Self::UnsupportedVersion(_) | Self::UnsupportedPayloadVersion(_) => {
                NEWER_VERSION_MESSAGE.to_string()
            }
            Self::Truncated => {
                "The backup file is incomplete. Select the original backup and try again."
                    .to_string()
            }
            Self::ImportApprovalStale(_) => format!(
                "The existing wallet data changed while the import was waiting for approval. {REVIEW_AGAIN_MESSAGE}"
            ),
            Self::ImportApprovalRequired(_) => format!(
                "This import needs approval before existing wallet data can be removed. {REVIEW_AGAIN_MESSAGE}"
            ),
            Self::ImportApprovalUsed => {
                format!("This import review has expired. {REVIEW_AGAIN_MESSAGE}")
            }
            Self::InvalidWalletId(_) => {
                "The backup contains an invalid wallet record and cannot be imported.".to_string()
            }
            Self::WalletIdOccupied(_) => {
                format!("A wallet changed while the import was waiting. {REVIEW_AGAIN_MESSAGE}")
            }
            Self::Encryption(_)
            | Self::Serialization(_)
            | Self::Deserialization(_)
            | Self::Gather(_)
            | Self::Restore(_)
            | Self::Keychain(_)
            | Self::Database(_)
            | Self::Decompression(_) => {
                "Cove could not complete this backup operation. Review it and try again."
                    .to_string()
            }
        }
    }
}

impl From<WalletIdentityError> for BackupError {
    fn from(error: WalletIdentityError) -> Self {
        match error {
            WalletIdentityError::Database(error) => Self::Database(error.to_string()),
            error => Self::Restore(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_has_exact_text_and_hides_payloads() {
        let payload = "payload-secret";
        let wallet_id = WalletId::preview_new();
        let review = "Review the backup again and try again.";
        let generic = "Cove could not complete this backup operation. Review it and try again.";
        let cases = [
            (BackupError::PasswordTooShort, "The backup password is too short.".to_string()),
            (
                BackupError::DecryptionFailed,
                "The backup password is incorrect, or the backup is damaged.".to_string(),
            ),
            (
                BackupError::InvalidFormat,
                "The selected file is not a valid Cove backup.".to_string(),
            ),
            (BackupError::FileTooLarge, "The backup file is too large.".to_string()),
            (BackupError::UnsupportedVersion(99), NEWER_VERSION_MESSAGE.to_string()),
            (BackupError::UnsupportedPayloadVersion(99), NEWER_VERSION_MESSAGE.to_string()),
            (
                BackupError::Truncated,
                "The backup file is incomplete. Select the original backup and try again."
                    .to_string(),
            ),
            (
                BackupError::ImportApprovalStale(wallet_id.clone()),
                format!(
                    "The existing wallet data changed while the import was waiting for approval. {review}"
                ),
            ),
            (
                BackupError::ImportApprovalRequired(wallet_id.clone()),
                format!(
                    "This import needs approval before existing wallet data can be removed. {review}"
                ),
            ),
            (BackupError::ImportApprovalUsed, format!("This import review has expired. {review}")),
            (
                BackupError::InvalidWalletId(payload.into()),
                "The backup contains an invalid wallet record and cannot be imported.".to_string(),
            ),
            (
                BackupError::WalletIdOccupied(wallet_id.clone()),
                format!("A wallet changed while the import was waiting. {review}"),
            ),
            (BackupError::Encryption(payload.into()), generic.to_string()),
            (BackupError::Serialization(payload.into()), generic.to_string()),
            (BackupError::Deserialization(payload.into()), generic.to_string()),
            (BackupError::Gather(payload.into()), generic.to_string()),
            (BackupError::Restore(payload.into()), generic.to_string()),
            (BackupError::Keychain(payload.into()), generic.to_string()),
            (BackupError::Database(payload.into()), generic.to_string()),
            (BackupError::Decompression(payload.into()), generic.to_string()),
        ];

        for (error, expected) in cases {
            let message = error.user_message();
            assert_eq!(message, expected, "{error:?}");
            assert!(!message.contains(payload), "{error:?}");
            assert!(!message.contains(wallet_id.to_string().as_str()), "{error:?}");
        }
    }
}
