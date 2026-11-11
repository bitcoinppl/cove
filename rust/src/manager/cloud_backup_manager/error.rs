use std::{error::Error as StdError, fmt, ops::Deref};

use cove_device::passkey::{PasskeyFailureReason, PasskeyOperation};
use cove_device::{
    cloud_storage::CloudStorageError, keychain::KeychainError, passkey::PasskeyError,
};
use cove_util::json::JsonDecodeError;
use tracing::{error, info, warn};

use crate::backup::import::LocalWalletConflict;
use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::admission::{PauseReason, Rejection};

const PASSKEY_ACCESS_RECOVERY_MESSAGE: &str = "Cove couldn't access your passkey. Check your connection and passkey account, then try again. If this keeps happening, choose another passkey provider or contact support.";
const PASSKEY_NOT_FOUND_MESSAGE: &str =
    "Cove couldn't find the requested passkey. Please try again or choose another passkey.";
const GENERIC_PASSKEY_ERROR_MESSAGE: &str = "Cove couldn't use this passkey. Please try again.";
const UNSUPPORTED_PASSKEY_PROVIDER_MESSAGE: &str =
    "This passkey provider can't protect Cove backups. Choose another passkey provider.";
pub(crate) const GENERIC_CLOUD_BACKUP_ERROR_MESSAGE: &str =
    "Cove couldn't complete this cloud backup request. Please try again.";
pub(crate) const CLOUD_BACKUP_LABELS_WARNING_MESSAGE: &str =
    "Cove restored the wallet, but couldn't restore its labels.";
pub(crate) const CLOUD_BACKUP_RECREATE_MESSAGE: &str =
    "Some wallet backups are missing from your cloud backup.";
pub(crate) const CLOUD_BACKUP_REINITIALIZE_MESSAGE: &str =
    "Cove couldn't verify the key needed to unlock this cloud backup.";
const CLOUD_BACKUP_AUTHORIZATION_MESSAGE: &str =
    "Cove couldn't access your cloud backup. Reconnect your cloud account, then try again.";
const CLOUD_BACKUP_OFFLINE_MESSAGE: &str =
    "Cove couldn't reach your cloud backup. Reconnect to the internet, then try again.";
const CLOUD_BACKUP_SYNC_PENDING_MESSAGE: &str =
    "Your cloud provider is still loading Cove backup files. Keep Cove open, then try again.";
const CLOUD_BACKUP_UNAVAILABLE_MESSAGE: &str =
    "Your cloud backup is temporarily unavailable. Please try again.";
const CLOUD_BACKUP_NOT_FOUND_MESSAGE: &str =
    "Cove couldn't find that cloud backup. Refresh and try again.";
const CLOUD_BACKUP_QUOTA_MESSAGE: &str =
    "Your cloud storage is full. Free up space, then try again.";
const CLOUD_BACKUP_CRYPTO_MESSAGE: &str =
    "Cove couldn't unlock this cloud backup. Check the selected passkey and try again.";
pub(crate) const CLOUD_BACKUP_COMPATIBILITY_MESSAGE: &str =
    "This cloud backup was created by an unsupported version of Cove.";
const CLOUD_BACKUP_WALLET_SUPPORT_MESSAGE: &str =
    "This cloud backup contains a wallet this version of Cove can't restore.";
const PASSKEY_REQUEST_INCOMPLETE_MESSAGE: &str = "The passkey request did not complete. Try again.";
const NEEDS_PASSKEY_MESSAGE: &str =
    "Cloud Backup needs your passkey. Open Cloud Backup and tap Verify to continue.";
const PASSKEY_BUSY_MESSAGE: &str =
    "Another passkey request is still open. Finish or close it, then try again.";
const LOCAL_WALLET_MISMATCH_MESSAGE: &str = concat!(
    "Cove unlocked your backup, but some saved wallet data on this device does not match it. ",
    "Cove kept that data unchanged."
);
const LOCAL_WALLET_UNREADABLE_MESSAGE: &str = concat!(
    "Cove unlocked your backup, but some saved wallet data on this device could not be read. ",
    "Cove kept that data unchanged."
);
const ANDROID_PASSKEY_ASSOCIATION_MESSAGE: &str = concat!(
    "Cove could not verify Android passkey setup yet. Wait a few minutes and try again. ",
    "If this keeps happening, update Cove or contact support."
);

/// Why a step of an Android Google Drive account switch could not run
///
/// Internal to the switch's owner, which turns each into what the screen shows
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum CloudBackupDriveAccountSwitchError {
    /// Another exclusive Cloud Backup operation owns the supervisor
    #[error("another cloud backup operation is already in progress")]
    Busy,
    /// Cloud Backup has no configured state to move to another account
    #[error("cloud backup is not configured")]
    NotConfigured,
    /// The supplied transition does not own the current operation
    #[error("invalid Google Drive account switch transition")]
    InvalidTransition,
    /// Persistence or actor coordination failed
    #[error("internal Google Drive account switch error: {0}")]
    Internal(String),
}

#[derive(Debug)]
struct CloudBackupErrorSource {
    message: String,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl CloudBackupErrorSource {
    fn message(message: impl Into<String>) -> Self {
        Self { message: message.into(), source: None }
    }

    fn source(source: impl StdError + Send + Sync + 'static) -> Self {
        Self { message: source.to_string(), source: Some(Box::new(source)) }
    }

    pub(crate) fn context(
        context: impl fmt::Display,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        let message = format!("{context}: {source}");

        Self { message, source: Some(Box::new(source)) }
    }
}

impl fmt::Display for CloudBackupErrorSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl StdError for CloudBackupErrorSource {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source.as_deref().map(|source| source as &(dyn StdError + 'static))
    }
}

impl Deref for CloudBackupErrorSource {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl PartialEq<&str> for CloudBackupErrorSource {
    fn eq(&self, other: &&str) -> bool {
        self.message == *other
    }
}

impl PartialEq<str> for CloudBackupErrorSource {
    fn eq(&self, other: &str) -> bool {
        self.message == other
    }
}

impl PartialEq<String> for CloudBackupErrorSource {
    fn eq(&self, other: &String) -> bool {
        self.message == *other
    }
}

macro_rules! cloud_backup_source_error {
    ($name:ident) => {
        #[derive(Debug)]
        pub(crate) struct $name(CloudBackupErrorSource);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl StdError for $name {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                self.0.source()
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                self.0 == *other
            }
        }

        impl From<String> for $name {
            fn from(message: String) -> Self {
                Self(CloudBackupErrorSource::message(message))
            }
        }

        impl From<&str> for $name {
            fn from(message: &str) -> Self {
                Self(CloudBackupErrorSource::message(message))
            }
        }
    };
}

cloud_backup_source_error!(CloudBackupPasskeyError);
cloud_backup_source_error!(CloudBackupCryptoError);
cloud_backup_source_error!(CloudBackupInternalError);

impl CloudBackupPasskeyError {
    fn passkey_error(&self) -> Option<&PasskeyError> {
        self.0.source.as_deref()?.downcast_ref::<PasskeyError>()
    }
}

impl CloudBackupCryptoError {
    pub(crate) fn context(
        context: impl fmt::Display,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self(CloudBackupErrorSource::context(context, source))
    }
}

impl CloudBackupInternalError {
    pub(crate) fn context(
        context: impl fmt::Display,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self(CloudBackupErrorSource::context(context, source))
    }
}

pub(crate) fn is_connectivity_related_issue(issue: impl Into<CloudStorageIssue>) -> bool {
    matches!(
        issue.into(),
        CloudStorageIssue::Offline
            | CloudStorageIssue::SyncPending
            | CloudStorageIssue::Unavailable
    )
}

pub(crate) fn is_provider_wide_interruption(issue: impl Into<CloudStorageIssue>) -> bool {
    matches!(
        issue.into(),
        CloudStorageIssue::AuthorizationRequired
            | CloudStorageIssue::Offline
            | CloudStorageIssue::SyncPending
            | CloudStorageIssue::Unavailable
    )
}

pub(crate) fn blocking_cloud_error(
    step: BlockingCloudStep,
    error: CloudBackupError,
) -> CloudBackupError {
    if CloudStorageIssue::from(&error) == CloudStorageIssue::Offline {
        return offline_error_for_step(step);
    }

    error
}

impl From<CloudBackupError> for CloudStorageIssue {
    fn from(error: CloudBackupError) -> Self {
        Self::from(&error)
    }
}

impl From<&CloudBackupError> for CloudStorageIssue {
    fn from(error: &CloudBackupError) -> Self {
        match error {
            CloudBackupError::Offline(_) | CloudBackupError::Deferred(_) => Self::Offline,
            CloudBackupError::CloudStorage(error) => error.into(),
            CloudBackupError::CloudStorageContext { source, .. } => source.into(),
            CloudBackupError::Cloud(_) => Self::Other,
            // never persisted: the upload path keeps a paused row Dirty before it classifies an
            // error, so this arm only names paused work for callers that report it
            CloudBackupError::WritesPaused(_) => Self::Other,
            CloudBackupError::NotAdmitted(_) => Self::Other,
            CloudBackupError::NotSupported(_)
            | CloudBackupError::UnsupportedPasskeyProvider
            | CloudBackupError::RecoveryRequired(_)
            | CloudBackupError::Passkey(_)
            | CloudBackupError::Crypto(_)
            | CloudBackupError::Internal(_)
            | CloudBackupError::Compatibility(_)
            | CloudBackupError::PasskeyMismatch
            | CloudBackupError::NoBackupFound
            | CloudBackupError::PasskeyDiscoveryCancelled
            | CloudBackupError::LocalWalletConflict(_)
            | CloudBackupError::NeedsPasskey
            | CloudBackupError::PasskeyBusy
            | CloudBackupError::Cancelled => Self::Other,
        }
    }
}

impl From<CloudStorageError> for CloudStorageIssue {
    fn from(error: CloudStorageError) -> Self {
        Self::from(&error)
    }
}

impl From<&CloudStorageError> for CloudStorageIssue {
    fn from(error: &CloudStorageError) -> Self {
        match error {
            CloudStorageError::AuthorizationRequired(_) => Self::AuthorizationRequired,
            CloudStorageError::Offline(_) => Self::Offline,
            CloudStorageError::SyncPending(_) => Self::SyncPending,
            CloudStorageError::NotAvailable(_) => Self::Unavailable,
            CloudStorageError::NotFound(_) => Self::NotFound,
            CloudStorageError::QuotaExceeded => Self::QuotaExceeded,
            CloudStorageError::UploadFailed(_)
            | CloudStorageError::DownloadFailed(_)
            | CloudStorageError::InvalidNamespace(_) => Self::Other,
        }
    }
}

pub(crate) fn offline_error_for_step(step: BlockingCloudStep) -> CloudBackupError {
    CloudBackupError::Offline(offline_message_for_step(step).into())
}

fn offline_message_for_step(step: BlockingCloudStep) -> &'static str {
    use BlockingCloudStep as B;
    match step {
        B::Enable => "Reconnect to the internet, then try enabling cloud backup again",
        B::Restore => "Reconnect to the internet, then try restoring from cloud backup again",
        B::Verify => "Reconnect to the internet, then try verifying cloud backup again",
        B::Sync => "Reconnect to the internet, then try syncing cloud backup again",
        B::FetchCloudOnly => "Reconnect to the internet, then try loading cloud-only wallets again",
        B::RestoreCloudWallet => {
            "Reconnect to the internet, then try restoring this cloud wallet again"
        }
        B::DeleteCloudWallet => {
            "Reconnect to the internet, then try deleting this cloud wallet again"
        }
        B::DeleteUndecryptableWalletBackups => {
            "Reconnect to the internet, then try deleting the inaccessible wallet backups again"
        }
        B::RecoverOtherBackups => {
            "Reconnect to the internet, then try recovering the other cloud backups again"
        }
        B::DeleteOtherBackups => {
            "Reconnect to the internet, then try deleting the other cloud backups again"
        }
        B::Disable => "Reconnect to the internet, then try disabling cloud backup again",
        B::RecreateManifest => {
            "Reconnect to the internet, then try recreating the cloud backup manifest again"
        }
        B::RepairPasskey => "Reconnect to the internet, then try repairing cloud backup again",
        B::DetailRefresh => {
            "Reconnect to the internet, then try refreshing cloud backup details again"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingCloudStep {
    Enable,
    Restore,
    Verify,
    Sync,
    FetchCloudOnly,
    RestoreCloudWallet,
    DeleteCloudWallet,
    DeleteUndecryptableWalletBackups,
    RecoverOtherBackups,
    DeleteOtherBackups,
    Disable,
    RecreateManifest,
    RepairPasskey,
    DetailRefresh,
}

/// How a passkey request ended without a usable credential
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasskeyRequestFailure {
    /// The user dismissed the request, or no window could present it yet; neither says
    /// anything about the passkey
    NotCompleted,
    /// The provider couldn't create or use a passkey
    Failed,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudBackupError {
    #[error("not supported: {0}")]
    NotSupported(String),

    #[error("passkey provider does not support PRF for Cloud Backup")]
    UnsupportedPasskeyProvider,

    #[error("{0}")]
    RecoveryRequired(String),

    #[error("passkey error: {0}")]
    Passkey(#[source] CloudBackupPasskeyError),

    #[error("crypto error: {0}")]
    Crypto(#[source] CloudBackupCryptoError),

    #[error("cloud storage error: {0}")]
    Cloud(String),

    #[error("cloud storage error: {0}")]
    CloudStorage(#[from] CloudStorageError),

    #[error("cloud storage error: {context}: {source}")]
    CloudStorageContext { context: String, source: CloudStorageError },

    #[error("offline: {0}")]
    Offline(String),

    #[error("deferred until connected: {0}")]
    Deferred(String),

    /// The write lane did not run the write because its policy pauses that command class
    ///
    /// Pausing is not failing: a paused upload keeps its row Dirty, with no retry timer, and the
    /// lane wakes the upload scheduler once the policy lets ordinary writes run again
    #[error("cloud backup writes are paused: {0:?}")]
    WritesPaused(PauseReason),

    /// The lifecycle owner did not admit the request in its current state
    #[error("cloud backup did not admit the request: {0:?}")]
    NotAdmitted(Rejection),

    #[error("internal error: {0}")]
    Internal(#[source] CloudBackupInternalError),

    #[error("compatibility error: {0}")]
    Compatibility(String),

    #[error(
        "This passkey did not unlock backups currently available in cloud storage. Wait for cloud storage to finish syncing, then try again."
    )]
    PasskeyMismatch,

    #[error("no cloud backups found")]
    NoBackupFound,

    #[error("user cancelled passkey discovery")]
    PasskeyDiscoveryCancelled,

    #[error("restore cancelled")]
    Cancelled,

    /// Work no user tap started needed the passkey, so it stopped without asking for it
    #[error("background work needs the passkey, which only a user action may request")]
    NeedsPasskey,

    /// Another passkey request was still on screen, so this one was not presented
    #[error("another passkey request is on screen")]
    PasskeyBusy,

    /// A restore kept local wallet data unchanged because it did not match the backup
    #[error("local wallet conflict: {0}")]
    LocalWalletConflict(#[source] LocalWalletConflict),
}

impl CloudBackupError {
    pub(crate) fn passkey(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Passkey(CloudBackupPasskeyError(CloudBackupErrorSource::source(source)))
    }

    pub(crate) fn crypto(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Crypto(CloudBackupCryptoError(CloudBackupErrorSource::source(source)))
    }

    pub(crate) fn crypto_context(
        context: impl fmt::Display,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Crypto(CloudBackupCryptoError::context(context, source))
    }

    pub(crate) fn internal(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Internal(CloudBackupInternalError(CloudBackupErrorSource::source(source)))
    }

    pub(crate) fn internal_context(
        context: impl fmt::Display,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Internal(CloudBackupInternalError::context(context, source))
    }

    /// Whether a keychain read failed somewhere in this error's source chain
    ///
    /// The keychain cannot be read while the device is locked, so the same operation can succeed
    /// later without any change, and callers must not record it as a permanent failure
    pub(crate) fn is_keychain_unreadable(&self) -> bool {
        let Self::Internal(error) = self else {
            return false;
        };

        let first: &(dyn StdError + 'static) = error;
        std::iter::successors(Some(first), |&error| error.source()).any(|error| {
            matches!(error.downcast_ref(), Some(KeychainError::Read(_)))
                || matches!(error.downcast_ref(), Some(cove_cspp::CsppError::Read(_)))
        })
    }

    pub(crate) fn cloud_storage_context(
        context: impl Into<String>,
        source: CloudStorageError,
    ) -> Self {
        Self::CloudStorageContext { context: context.into(), source }
    }

    /// Adds `context` to a bare cloud storage error, such as one the write lane returned
    pub(crate) fn with_cloud_storage_context(self, context: impl Into<String>) -> Self {
        match self {
            Self::CloudStorage(source) => Self::cloud_storage_context(context, source),
            error => error,
        }
    }

    /// The user didn't finish a passkey request; nothing changed and nothing is wrong
    pub(crate) fn is_passkey_not_completed(&self) -> bool {
        matches!(self, Self::Passkey(error) if matches!(error.passkey_error(), Some(PasskeyError::NotCompleted)))
    }

    /// What this error says about a passkey request, or `None` when it isn't a passkey error
    pub(crate) fn passkey_request_failure(&self) -> Option<PasskeyRequestFailure> {
        match self {
            Self::PasskeyDiscoveryCancelled => Some(PasskeyRequestFailure::NotCompleted),
            Self::Passkey(error)
                if matches!(
                    error.passkey_error(),
                    Some(PasskeyError::NotCompleted | PasskeyError::NotReady { .. })
                ) =>
            {
                Some(PasskeyRequestFailure::NotCompleted)
            }
            Self::Passkey(_) | Self::UnsupportedPasskeyProvider => {
                Some(PasskeyRequestFailure::Failed)
            }
            _ => None,
        }
    }

    pub(crate) fn is_cloud_error(&self) -> bool {
        matches!(self, Self::Cloud(_) | Self::CloudStorage(_) | Self::CloudStorageContext { .. })
    }

    pub(crate) fn reader_message(&self) -> String {
        match self {
            Self::Passkey(error) => match error.passkey_error() {
                Some(PasskeyError::RequestFailed {
                    reason:
                        PasskeyFailureReason::PlatformAuthorizationFailed
                        | PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
                    ..
                }) => PASSKEY_ACCESS_RECOVERY_MESSAGE.into(),
                Some(PasskeyError::RequestFailed {
                    operation: PasskeyOperation::Registration,
                    reason: PasskeyFailureReason::DeviceNotConfigured,
                }) => ANDROID_PASSKEY_ASSOCIATION_MESSAGE.into(),
                Some(PasskeyError::NotCompleted) => PASSKEY_REQUEST_INCOMPLETE_MESSAGE.into(),
                Some(PasskeyError::NoCredentialFound) => PASSKEY_NOT_FOUND_MESSAGE.into(),
                Some(PasskeyError::PrfUnsupportedProvider) => {
                    UNSUPPORTED_PASSKEY_PROVIDER_MESSAGE.into()
                }
                Some(_) | None => GENERIC_PASSKEY_ERROR_MESSAGE.into(),
            },
            Self::UnsupportedPasskeyProvider => UNSUPPORTED_PASSKEY_PROVIDER_MESSAGE.into(),
            Self::CloudStorage(error) | Self::CloudStorageContext { source: error, .. } => {
                cloud_storage_reader_message(error).into()
            }
            Self::Offline(message) | Self::RecoveryRequired(message) => message.clone(),
            Self::NotSupported(_) => CLOUD_BACKUP_WALLET_SUPPORT_MESSAGE.into(),
            Self::Crypto(_) => CLOUD_BACKUP_CRYPTO_MESSAGE.into(),
            Self::Compatibility(_) => CLOUD_BACKUP_COMPATIBILITY_MESSAGE.into(),
            Self::Cloud(_)
            | Self::Deferred(_)
            | Self::WritesPaused(_)
            | Self::NotAdmitted(_)
            | Self::Internal(_) => GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into(),
            Self::LocalWalletConflict(LocalWalletConflict::Mismatch) => {
                LOCAL_WALLET_MISMATCH_MESSAGE.into()
            }
            Self::LocalWalletConflict(LocalWalletConflict::Unreadable) => {
                LOCAL_WALLET_UNREADABLE_MESSAGE.into()
            }
            // a bare platform cancellation does not prove the reader cancelled, so the
            // copy states the outcome instead of claiming intent
            Self::PasskeyDiscoveryCancelled => PASSKEY_REQUEST_INCOMPLETE_MESSAGE.into(),
            Self::NeedsPasskey => NEEDS_PASSKEY_MESSAGE.into(),
            Self::PasskeyBusy => PASSKEY_BUSY_MESSAGE.into(),
            Self::PasskeyMismatch | Self::NoBackupFound | Self::Cancelled => self.to_string(),
        }
    }

    /// Logs the full error under `operation` at a level that matches its kind, then returns the
    /// message shown to the reader
    ///
    /// The reader message drops the underlying cause, so a failure that reaches the UI without
    /// a log leaves nothing in a diagnostics report to explain it
    pub(crate) fn logged_reader_message(&self, operation: impl fmt::Display) -> String {
        match self {
            // a locked device keeps the keychain unreadable until unlock, and callers retry
            Self::Internal(_) if self.is_keychain_unreadable() => warn!("{operation}: {self}"),

            // local storage or app bugs, which retrying will not fix
            Self::Internal(_) => error!("{operation}: {self}"),

            // the passkey sheet was dismissed, which is an expected outcome
            Self::Passkey(error)
                if matches!(error.passkey_error(), Some(PasskeyError::NotCompleted)) =>
            {
                info!("{operation}: {self}");
            }

            // the reader stopped the operation, so it is an expected outcome
            Self::Cancelled | Self::PasskeyDiscoveryCancelled => info!("{operation}: {self}"),

            // work no tap started stopped before a passkey sheet, or a tap found another sheet
            // on screen; both are expected and the user's next tap goes on
            Self::NeedsPasskey | Self::PasskeyBusy => info!("{operation}: {self}"),

            Self::NotSupported(_)
            | Self::UnsupportedPasskeyProvider
            | Self::RecoveryRequired(_)
            | Self::Passkey(_)
            | Self::Compatibility(_)
            | Self::LocalWalletConflict(_)
            | Self::Crypto(_)
            | Self::Cloud(_)
            | Self::CloudStorage(_)
            | Self::CloudStorageContext { .. }
            | Self::Offline(_)
            | Self::Deferred(_)
            | Self::WritesPaused(_)
            | Self::NotAdmitted(_)
            | Self::PasskeyMismatch
            | Self::NoBackupFound => warn!("{operation}: {self}"),
        }

        self.reader_message()
    }
}

fn cloud_storage_reader_message(error: &CloudStorageError) -> &'static str {
    match CloudStorageIssue::from(error) {
        CloudStorageIssue::AuthorizationRequired => CLOUD_BACKUP_AUTHORIZATION_MESSAGE,
        CloudStorageIssue::Offline => CLOUD_BACKUP_OFFLINE_MESSAGE,
        CloudStorageIssue::SyncPending => CLOUD_BACKUP_SYNC_PENDING_MESSAGE,
        CloudStorageIssue::Unavailable => CLOUD_BACKUP_UNAVAILABLE_MESSAGE,
        CloudStorageIssue::NotFound => CLOUD_BACKUP_NOT_FOUND_MESSAGE,
        CloudStorageIssue::QuotaExceeded => CLOUD_BACKUP_QUOTA_MESSAGE,
        CloudStorageIssue::Other => GENERIC_CLOUD_BACKUP_ERROR_MESSAGE,
    }
}

impl From<CloudBackupPasskeyError> for CloudBackupError {
    fn from(error: CloudBackupPasskeyError) -> Self {
        Self::Passkey(error)
    }
}

impl From<PasskeyError> for CloudBackupError {
    fn from(error: PasskeyError) -> Self {
        Self::passkey(error)
    }
}

impl From<CloudBackupCryptoError> for CloudBackupError {
    fn from(error: CloudBackupCryptoError) -> Self {
        Self::Crypto(error)
    }
}

impl From<cove_cspp::CsppError> for CloudBackupError {
    fn from(error: cove_cspp::CsppError) -> Self {
        Self::crypto(error)
    }
}

impl From<CloudBackupInternalError> for CloudBackupError {
    fn from(error: CloudBackupInternalError) -> Self {
        Self::Internal(error)
    }
}

impl From<crate::backup::BackupError> for CloudBackupInternalError {
    fn from(error: crate::backup::BackupError) -> Self {
        Self(CloudBackupErrorSource::source(error))
    }
}

// serde's text quotes the rejected value, which can be decrypted backup content
impl From<serde_json::Error> for CloudBackupError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(JsonDecodeError::from(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_wide_interruption_covers_access_and_provider_availability() {
        for issue in [
            CloudStorageIssue::AuthorizationRequired,
            CloudStorageIssue::Offline,
            CloudStorageIssue::Unavailable,
        ] {
            assert!(is_provider_wide_interruption(issue));
        }

        for issue in [
            CloudStorageIssue::NotFound,
            CloudStorageIssue::QuotaExceeded,
            CloudStorageIssue::Other,
        ] {
            assert!(!is_provider_wide_interruption(issue));
        }
    }

    #[test]
    fn platform_authorization_failures_use_durable_recovery_message() {
        for reason in [
            PasskeyFailureReason::PlatformAuthorizationFailed,
            PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
        ] {
            let error = CloudBackupError::from(PasskeyError::RequestFailed {
                operation: PasskeyOperation::DiscoverAssertion,
                reason,
            });

            assert_eq!(error.reader_message(), PASSKEY_ACCESS_RECOVERY_MESSAGE);
        }
    }

    #[test]
    fn raw_passkey_diagnostic_cannot_reach_reader_message() {
        let diagnostic =
            "Q8UP8C53Y8 org.bitcoinppl.cove webcredentials:covebitcoinwallet.com credential=secret";
        let error = CloudBackupError::from(PasskeyError::RequestFailed {
            operation: PasskeyOperation::DiscoverAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: diagnostic.into() },
        });
        let message = error.reader_message();

        assert_eq!(message, GENERIC_PASSKEY_ERROR_MESSAGE);
        for marker in [
            "Q8UP8C53Y8",
            "org.bitcoinppl.cove",
            "covebitcoinwallet.com",
            "webcredentials",
            "credential",
            "secret",
        ] {
            assert!(!message.contains(marker));
        }
    }

    #[test]
    fn reader_message_keeps_distinct_passkey_outcomes() {
        let cancellation = CloudBackupError::from(PasskeyError::NotCompleted).reader_message();
        let missing = CloudBackupError::from(PasskeyError::NoCredentialFound).reader_message();
        let unsupported =
            CloudBackupError::from(PasskeyError::PrfUnsupportedProvider).reader_message();

        assert_eq!(cancellation, PASSKEY_REQUEST_INCOMPLETE_MESSAGE);
        assert_eq!(
            CloudBackupError::PasskeyDiscoveryCancelled.reader_message(),
            PASSKEY_REQUEST_INCOMPLETE_MESSAGE
        );
        assert_eq!(missing, PASSKEY_NOT_FOUND_MESSAGE);
        assert_eq!(unsupported, UNSUPPORTED_PASSKEY_PROVIDER_MESSAGE);
        assert_ne!(cancellation, missing);
        assert_ne!(missing, unsupported);
    }

    #[test]
    fn passkey_mismatch_reader_message_is_provider_neutral() {
        let message = CloudBackupError::PasskeyMismatch.reader_message();

        assert!(message.contains("cloud storage"));
        assert!(!message.contains("iCloud"));
        assert!(!message.contains("Google Drive"));
    }

    #[test]
    fn raw_cloud_and_internal_diagnostics_cannot_reach_reader_messages() {
        let diagnostic = "account=user@example.com namespace=secret-namespace record=secret-record localized=privado";
        let errors = [
            CloudBackupError::Cloud(diagnostic.into()),
            CloudBackupError::CloudStorage(CloudStorageError::DownloadFailed(diagnostic.into())),
            CloudBackupError::cloud_storage_context(
                diagnostic,
                CloudStorageError::NotAvailable(diagnostic.into()),
            ),
            CloudBackupError::Internal(diagnostic.into()),
            CloudBackupError::Crypto(diagnostic.into()),
            CloudBackupError::Compatibility(diagnostic.into()),
            CloudBackupError::NotSupported(diagnostic.into()),
        ];

        for error in errors {
            let message = error.reader_message();
            for marker in ["user@example.com", "secret-namespace", "secret-record", "privado"] {
                assert!(!message.contains(marker), "reader message leaked {marker}: {message}");
            }
        }
    }

    #[test]
    fn cloud_storage_reader_messages_preserve_safe_recovery_categories() {
        let cases = [
            (
                CloudStorageError::AuthorizationRequired("raw account".into()),
                CLOUD_BACKUP_AUTHORIZATION_MESSAGE,
            ),
            (CloudStorageError::Offline("localized".into()), CLOUD_BACKUP_OFFLINE_MESSAGE),
            (CloudStorageError::NotAvailable("localized".into()), CLOUD_BACKUP_UNAVAILABLE_MESSAGE),
            (CloudStorageError::NotFound("secret record".into()), CLOUD_BACKUP_NOT_FOUND_MESSAGE),
            (CloudStorageError::QuotaExceeded, CLOUD_BACKUP_QUOTA_MESSAGE),
            (
                CloudStorageError::UploadFailed("localized".into()),
                GENERIC_CLOUD_BACKUP_ERROR_MESSAGE,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(CloudBackupError::from(error).reader_message(), expected);
        }
    }
}
