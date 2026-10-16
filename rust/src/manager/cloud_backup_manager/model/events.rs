use cove_device::cloud_storage::CloudSyncHealth;

use crate::database::cloud_backup::DriveAccountSwitchId;

use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupDisableOutcome, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint, CloudBackupProgress,
    CloudBackupSettingsRowStatus, CloudBackupStatus, CloudBackupUndecryptableWalletDeletionState,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation, CloudOnlyOperation,
    CloudOnlyState, OtherBackupsOperation, PendingUploadVerificationState, RecoveryState,
    SyncState, VerificationState,
};

use super::cloud_only_inventory::{CloudOnlyFetch, CloudOnlyListingUpdate};
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
    DeleteUndecryptableWalletBackups,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudBackupExclusiveOperationOwner {
    Generation(u64),
    DriveAccountSwitch(DriveAccountSwitchId),
}

/// Ownership proof for an exclusive operation
///
/// Async completions must present the claim they started with before the
/// reducer or supervisor accepts their result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupExclusiveOperationClaim {
    operation: CloudBackupExclusiveOperation,
    owner: CloudBackupExclusiveOperationOwner,
}

impl CloudBackupExclusiveOperationClaim {
    pub(crate) fn new(operation: CloudBackupExclusiveOperation, generation: u64) -> Self {
        Self { operation, owner: CloudBackupExclusiveOperationOwner::Generation(generation) }
    }

    pub(crate) fn drive_account_switch(transition_id: DriveAccountSwitchId) -> Self {
        Self {
            operation: CloudBackupExclusiveOperation::ReinitializeBackup,
            owner: CloudBackupExclusiveOperationOwner::DriveAccountSwitch(transition_id),
        }
    }

    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        self.operation
    }

    pub(crate) fn drive_account_switch_id(self) -> Option<DriveAccountSwitchId> {
        match self.owner {
            CloudBackupExclusiveOperationOwner::DriveAccountSwitch(transition_id) => {
                Some(transition_id)
            }
            CloudBackupExclusiveOperationOwner::Generation(_) => None,
        }
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
    UndecryptableWalletDeletionStateResolved(CloudBackupUndecryptableWalletDeletionState),
    DetailRefreshStarted,
    DetailRefreshProvisional(CloudBackupDetail),
    DetailCleared,
    DetailRefreshApplied {
        detail: (CloudBackupDetail, super::CloudBackupInventoryAuthority),
        cloud_only_listing: CloudOnlyListingUpdate,
    },
    DetailRefreshFailed {
        reason: super::CloudBackupInventoryIncompleteReason,
        error: String,
    },
    CloudOnlyStateResolved(CloudOnlyState),
    /// A cloud-only fetch finished, applied only if its listing generation is still current
    CloudOnlyFetchLoaded(CloudOnlyFetch),
    /// A listed record was restored or deleted, so it is no longer cloud-only
    CloudOnlyRowRemoved(String),
    /// Cloud-only rows no longer belong to any known listing, such as after disable or reset
    CloudOnlyInventoryInvalidated,
    /// The local wallet set changed, so the listing and any loaded rows may now count a record as
    /// cloud-only, restorable, or a duplicate when it no longer is
    CloudOnlyLocalWalletsChanged,
    CloudOnlyOperationResolved(CloudOnlyOperation),
    OtherBackupsOperationResolved(OtherBackupsOperation),
    OtherBackupsStateResolved(crate::manager::cloud_backup_manager::CloudBackupOtherBackupsState),
}

/// Side effects the manager should emit after applying a reducer event
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudBackupStateReducerEffects {
    pub(crate) lifecycle: Option<CloudBackupLifecycleEffect>,
    pub(crate) enable_completed: Option<CloudBackupEnableContext>,
    pub(crate) status_changed: bool,
    pub(crate) verification_presentation_changed: bool,
    pub(crate) verification_decision_pending: bool,
    /// Applying the event invalidated cloud-only rows that a fetch has to repopulate
    pub(crate) cloud_only_refetch_needed: bool,
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
