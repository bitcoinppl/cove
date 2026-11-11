use cove_device::cloud_storage::CloudSyncHealth;

use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint,
    CloudBackupProgress, CloudBackupRootPrompt, CloudBackupVerificationPresentation,
    DeepVerificationFailure, DeepVerificationReport, SavedPasskeyConfirmationMode,
};

/// Public passkey health state for the configured backup
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyState {
    Available,
    Missing,
    UnsupportedProvider,
    NeedsRepair { state: CloudBackupPasskeyRepairState },
}

/// Public repair status for a missing or stale backup passkey
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyRepairState {
    Idle,
    Running,
    Failed(String),
}

/// Public backup verification state shown by settings and prompts
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationState {
    NotVerified,
    Verified {
        report: Option<DeepVerificationReport>,
        last_verified_at: Option<u64>,
    },
    NeedsAttention {
        report: DeepVerificationReport,
        checked_at: Option<u64>,
    },
    Required,
    Running,
    AwaitingUploadConfirmation,
    /// The passkey request did not complete; the card shows it, and nothing prompts again
    NotCompleted,
    Failed(DeepVerificationFailure),
}

/// Public sync status for background cloud backup work
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSyncState {
    Idle,
    Syncing,
    Blocked(String),
    Failed(String),
}

/// Public status for destructive operations that can affect remote backup data
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudBackupDestructiveOperationState {
    Idle,
    RecreatingManifest,
    ReinitializingBackup,
    Disabling,
    DisableFailed { message: String, can_keep_enabled: bool },
}

/// Status of a request to remove cloud-only backups that the active key cannot decrypt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudBackupUndecryptableWalletDeletionState {
    Idle,
    Deleting,
    Failed(String),
}

/// Typed reason why provider inventory could not be confirmed complete
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupInventoryIncompleteReason {
    AuthorizationRequired,
    Offline,
    ProviderSyncPending,
    ProviderUnavailable,
    Unknown,
}

impl From<CloudStorageIssue> for CloudBackupInventoryIncompleteReason {
    fn from(issue: CloudStorageIssue) -> Self {
        match issue {
            CloudStorageIssue::AuthorizationRequired => Self::AuthorizationRequired,
            CloudStorageIssue::Offline => Self::Offline,
            CloudStorageIssue::SyncPending => Self::ProviderSyncPending,
            CloudStorageIssue::Unavailable => Self::ProviderUnavailable,
            CloudStorageIssue::NotFound
            | CloudStorageIssue::QuotaExceeded
            | CloudStorageIssue::Other => Self::Unknown,
        }
    }
}

/// Rust-owned Restore All availability and in-process progress
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRestoreAllState {
    NotShown,
    StartDisabled {
        wallet_count: u32,
    },
    StartAvailable {
        wallet_count: u32,
    },
    Running {
        completed: u32,
        total: u32,
        current_wallet_name: Option<String>,
        cancellation_requested: bool,
    },
    RetryDisabled {
        wallet_count: u32,
    },
    RetryAvailable {
        wallet_count: u32,
    },
}

/// Public configured-backup state projected from the private reducer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudBackupConfiguredState {
    pub passkey: CloudBackupPasskeyState,
    pub verification: CloudBackupVerificationState,
    pub sync: CloudBackupSyncState,
    pub destructive_operation: CloudBackupDestructiveOperationState,
    pub undecryptable_wallet_deletion: CloudBackupUndecryptableWalletDeletionState,
    pub root_prompt: CloudBackupRootPrompt,
    pub sync_health: CloudSyncHealth,
    pub verification_presentation: CloudBackupVerificationPresentation,
}

/// Public enable flow state for onboarding and settings
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupEnableFlow {
    DiscoveringExistingBackup,
    AwaitingForceNewConfirmation(CloudBackupEnableContext, Option<CloudBackupPasskeyHint>),
    AwaitingPasskeyChoice(CloudBackupPasskeyChoiceIntent),
    CreatingPasskey,
    AwaitingSavedPasskeyConfirmation(SavedPasskeyConfirmationMode),
    ConfirmingSavedPasskey,
    UploadingInitialBackup { progress: Option<CloudBackupProgress> },
    RetryingUploadWithStagedMaterial { progress: Option<CloudBackupProgress> },
    WaitingForPasskeyAvailability,
}

/// Public restore progress state
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRestoreFlow {
    Finding,
    Downloading { completed: u32, total: u32 },
    Restoring { completed: u32, total: u32 },
}

/// Public terminal cloud backup failure
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudBackupFailure {
    pub message: String,
}

/// Availability of local-only cleanup for an interrupted Cloud Backup enable
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPendingEnableCleanupState {
    SupportOnly,
    Available,
    Cleaning,
}

/// Privacy-safe recovery state for an interrupted Cloud Backup enable
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupPendingEnableRecovery {
    pub support_code: String,
    pub cleanup: CloudBackupPendingEnableCleanupState,
}

/// Public top-level cloud backup lifecycle
#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudBackupLifecycle {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured(CloudBackupConfiguredState),
    PendingEnableRecovery(CloudBackupPendingEnableRecovery),
    Failed(CloudBackupFailure),
}
