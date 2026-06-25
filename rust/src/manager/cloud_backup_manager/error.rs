use cove_device::cloud_storage::CloudStorageError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudStorageIssue {
    AuthorizationRequired,
    Offline,
    Unavailable,
    NotFound,
    QuotaExceeded,
    Other,
}

pub(crate) fn is_connectivity_related_issue(issue: impl Into<CloudStorageIssue>) -> bool {
    matches!(issue.into(), CloudStorageIssue::Offline | CloudStorageIssue::Unavailable)
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
    RecoverOtherBackups,
    DeleteOtherBackups,
    Disable,
    RecreateManifest,
    RepairPasskey,
    DetailRefresh,
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
    Passkey(String),

    #[error("crypto error: {0}")]
    Crypto(String),

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

    #[error("internal error: {0}")]
    Internal(String),

    #[error("compatibility error: {0}")]
    Compatibility(String),

    #[error("Passkey didn't match any backups, please try a new one")]
    PasskeyMismatch,

    #[error("no cloud backups found")]
    NoBackupFound,

    #[error("user cancelled passkey discovery")]
    PasskeyDiscoveryCancelled,

    #[error("restore cancelled")]
    Cancelled,
}

impl CloudBackupError {
    pub(crate) fn cloud_storage_context(
        context: impl Into<String>,
        source: CloudStorageError,
    ) -> Self {
        Self::CloudStorageContext { context: context.into(), source }
    }

    pub(crate) fn is_cloud_error(&self) -> bool {
        matches!(self, Self::Cloud(_) | Self::CloudStorage(_) | Self::CloudStorageContext { .. })
    }
}
