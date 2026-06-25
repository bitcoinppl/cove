use cove_device::cloud_storage::CloudSyncHealth;

use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupEnableContext, CloudBackupPasskeyChoiceIntent,
    CloudBackupPasskeyHint, CloudBackupProgress, CloudBackupRootPrompt,
    CloudBackupVerificationPresentation, CloudOnlyOperation, CloudOnlyState,
    DeepVerificationFailure, DeepVerificationReport, OtherBackupsOperation,
    SavedPasskeyConfirmationMode,
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
    Verified { report: Option<DeepVerificationReport>, last_verified_at: Option<u64> },
    Required,
    Running,
    AwaitingUploadConfirmation,
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDestructiveOperationState {
    Idle,
    RecreatingManifest,
    ReinitializingBackup,
    Disabling,
    DisableFailed { message: String, can_keep_enabled: bool },
}

/// Detail payload shown after remote backup detail has loaded
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LoadedCloudBackupDetail {
    pub detail: CloudBackupDetail,
    pub cloud_only: CloudOnlyState,
    pub cloud_only_operation: CloudOnlyOperation,
    pub other_backups_operation: OtherBackupsOperation,
}

/// Public loading state for the cloud backup detail screen
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDetailState {
    NotLoaded,
    Loading,
    Loaded { state: LoadedCloudBackupDetail },
    Failed(String),
}

/// Public configured-backup state projected from the private reducer
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupConfiguredState {
    pub passkey: CloudBackupPasskeyState,
    pub verification: CloudBackupVerificationState,
    pub sync: CloudBackupSyncState,
    pub destructive_operation: CloudBackupDestructiveOperationState,
    pub detail: CloudBackupDetailState,
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupFailure {
    pub message: String,
}

/// Public top-level cloud backup lifecycle
#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupLifecycle {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured(CloudBackupConfiguredState),
    Failed(CloudBackupFailure),
}
