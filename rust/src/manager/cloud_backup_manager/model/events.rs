use cove_device::cloud_storage::CloudSyncHealth;

use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupDisableOutcome, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint, CloudBackupProgress,
    CloudBackupSettingsRowStatus, CloudBackupStatus, CloudBackupVerificationMetadata,
    CloudBackupVerificationPresentation, CloudOnlyOperation, CloudOnlyState, OtherBackupsOperation,
    PendingUploadVerificationState, RecoveryState, SyncState, VerificationState,
};

use super::{CloudBackupLifecycle, CloudBackupPendingEnableRecovery, CloudBackupRestoreFlow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupAcceptedEnablePrompt {
    Enable(CloudBackupEnableContext),
    ForceNew(CloudBackupEnableContext),
    NoDiscovery(CloudBackupEnableContext),
}

/// Exclusive operation category where newer claims replace active older claims
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupExclusiveOperation {
    Enable,
    EnableForceNew,
    EnableNoDiscovery,
    Restore,
    Disable,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
    VerificationRepair,
    RecoverOtherBackups,
    DeleteOtherBackups,
    RestoreCloudWallet,
    RestoreAllCloudWallets,
    DeleteCloudWallet,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum CloudBackupRestoreAllRuntimeState {
    #[default]
    Idle,
    Running {
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        total: u32,
        current_wallet_name: Option<String>,
        cancellation_requested: bool,
    },
    RetryRemaining,
}

/// Generation-tagged ownership proof for an exclusive operation
///
/// Async completions must present the claim they started with before the
/// reducer or supervisor accepts their result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupExclusiveOperationClaim {
    operation: CloudBackupExclusiveOperation,
    generation: u64,
}

impl CloudBackupExclusiveOperationClaim {
    pub(crate) fn new(operation: CloudBackupExclusiveOperation, generation: u64) -> Self {
        Self { operation, generation }
    }

    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        self.operation
    }
}

/// Event accepted by the private reducer
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupStateReducerEvent {
    ExclusiveOperationStarted(CloudBackupExclusiveOperationClaim),
    ExclusiveOperationFinished(CloudBackupExclusiveOperationClaim),
    EnableContextStarted(CloudBackupEnableContext),
    RuntimeStatusReconciled(CloudBackupStatus),
    PendingEnableRecoveryProjected(CloudBackupPendingEnableRecovery),
    ExistingBackupFoundPromptSet {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    ExistingBackupFoundPromptCleared,
    PasskeyChoicePromptSet(CloudBackupPasskeyChoiceIntent),
    PasskeyChoicePromptCleared,
    MissingPasskeyPromptDismissed,
    MissingPasskeyDismissalCleared,
    PromptStateCleared,
    EnableProgressReported(Option<CloudBackupProgress>),
    RestoreProgressReported(CloudBackupRestoreFlow),
    RestoreAllStarted {
        claim: CloudBackupExclusiveOperationClaim,
        total: u32,
    },
    RestoreAllProgressed {
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        current_wallet_name: Option<String>,
    },
    RestoreAllCancellationRequested(CloudBackupExclusiveOperationClaim),
    RestoreAllFinished {
        claim: CloudBackupExclusiveOperationClaim,
        retry_remaining: bool,
    },
    RestoreAllRetryRequired,
    RestoreAllReset,
    SyncHealthObserved(CloudSyncHealth),
    EnableFlowAdvanced(CloudBackupEnableState),
    PendingUploadVerificationReconciled(PendingUploadVerificationState),
    PendingUploadVerificationAndFlagsReconciled {
        pending: PendingUploadVerificationState,
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationFlagsReconciled {
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationPresentationReconciled(CloudBackupVerificationPresentation),
    VerificationStateResolved(VerificationState),
    SyncStateResolved(SyncState),
    RecoveryStateResolved(RecoveryState),
    DisableStateResolved(CloudBackupDisableOutcome),
    DetailRefreshStarted,
    DetailRefreshProvisional(CloudBackupDetail),
    DetailRefreshApplied {
        detail: Option<CloudBackupDetail>,
        reset_cloud_only: bool,
    },
    DetailRefreshFailed {
        reason: super::CloudBackupInventoryIncompleteReason,
        error: String,
    },
    CloudOnlyStateResolved(CloudOnlyState),
    CloudOnlyOperationResolved(CloudOnlyOperation),
    OtherBackupsOperationResolved(OtherBackupsOperation),
}

/// Intentionally uninhabited marker because reducer events are currently total
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupStateReducerEventRejection {}

/// Side effects the manager should emit after applying a reducer event
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudBackupStateReducerEffects {
    pub(crate) lifecycle: Option<CloudBackupLifecycleEffect>,
    pub(crate) enable_completed: Option<CloudBackupEnableContext>,
    pub(crate) status_changed: bool,
    pub(crate) verification_presentation_changed: bool,
    pub(crate) verification_decision_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudBackupLifecycleEffect {
    pub(crate) lifecycle: CloudBackupLifecycle,
    pub(crate) settings_row_status: CloudBackupSettingsRowStatus,
}

impl PartialEq<CloudBackupLifecycle> for CloudBackupLifecycleEffect {
    fn eq(&self, other: &CloudBackupLifecycle) -> bool {
        self.lifecycle == *other
    }
}
