use cove_device::cloud_storage::CloudSyncHealth;

use crate::database::cloud_backup::DriveAccountSwitchId;
use crate::manager::cloud_backup_manager::owner::operation::OperationId;

use crate::manager::cloud_backup_manager::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint, CloudBackupProgress,
    CloudBackupSettingsRowStatus, CloudBackupStatus, CloudBackupUndecryptableWalletDeletionState,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
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
    /// Verification of a backup that a reinitialize just replaced
    ///
    /// It holds the exclusive slot so nothing else starts before the new backup is checked, but
    /// unlike `ReinitializeBackup` it no longer reports the backup as enabling, since verification
    /// only reads the cloud for an enabled backup
    VerifyReinitializedBackup,
    RepairPasskey,
    /// A Verify the user tapped, held through authentication, checking, the auto-sync of
    /// missing wallets and wrapper repair
    ///
    /// It holds the exclusive slot from its admission, so nothing else starts while it reads the
    /// backup and its evidence belongs to this one run
    Verify,
    RecoverOtherBackups,
    DeleteOtherBackups,
    RestoreCloudWallet,
    RestoreAllCloudWallets,
    DeleteCloudWallet,
    DeleteUndecryptableWalletBackups,
    /// Discarding an interrupted Enable, whose remote delete runs off the supervisor mailbox
    ///
    /// Only the supervisor's runtime holds it; it is never projected as an exclusive operation,
    /// since the recovery screen already shows the discard
    DiscardPendingEnable,
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

/// Ownership proof for an exclusive operation
///
/// Async completions must present the claim they started with before the
/// reducer or supervisor accepts their result
///
/// Every run gets its own `OperationId`, and completions match on it. A Google Drive account
/// switch's reinitialization also carries the switch's persisted transition id, which platform
/// callbacks name across a relaunch. The transition id says which switch a run works for, never
/// which run it is: resuming the switch after a relaunch or an abandoned reset starts a run with
/// a fresh id, so a completion from the run a reset revoked never matches the one that resumed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupExclusiveOperationClaim {
    operation: CloudBackupExclusiveOperation,
    id: OperationId,
    drive_account_switch: Option<DriveAccountSwitchId>,
}

impl CloudBackupExclusiveOperationClaim {
    pub(crate) fn new(operation: CloudBackupExclusiveOperation, id: OperationId) -> Self {
        Self { operation, id, drive_account_switch: None }
    }

    /// One run of the reinitialization for the Drive account switch `transition_id`
    pub(crate) fn drive_account_switch(
        transition_id: DriveAccountSwitchId,
        id: OperationId,
    ) -> Self {
        Self {
            operation: CloudBackupExclusiveOperation::ReinitializeBackup,
            id,
            drive_account_switch: Some(transition_id),
        }
    }

    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        self.operation
    }

    pub(crate) fn drive_account_switch_id(self) -> Option<DriveAccountSwitchId> {
        self.drive_account_switch
    }
}

/// Event accepted by the private reducer
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupStateReducerEvent {
    ExclusiveOperationStarted(CloudBackupExclusiveOperationClaim),
    ExclusiveOperationFinished(CloudBackupExclusiveOperationClaim),
    /// The active claim moves to a follow-up operation without the slot ever being free
    ExclusiveOperationHandedOff {
        from: CloudBackupExclusiveOperationClaim,
        to: CloudBackupExclusiveOperationClaim,
    },
    EnableContextStarted(CloudBackupEnableContext),
    RuntimeStatusReconciled(CloudBackupStatus),
    /// The supervisor holds an interrupted Enable; only it decides when the recovery ends
    PendingEnableRecoveryProjected(CloudBackupPendingEnableRecovery),
    /// The supervisor's runtime left the recovery; the lifecycle shows this status instead
    PendingEnableRecoveryCleared(CloudBackupStatus),
    ExistingBackupFoundPromptSet {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    /// Ends the prompt an Enable waits on, the existing-backup prompt or the passkey choice
    EnablePromptCleared,
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
    /// Rust and the platform disagree about a Google Drive account switch; `None` clears it
    DriveAccountSwitchRecoveryReported(Option<CloudBackupDriveAccountSwitchRecovery>),
    UndecryptableWalletDeletionStateResolved(CloudBackupUndecryptableWalletDeletionState),
}

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
