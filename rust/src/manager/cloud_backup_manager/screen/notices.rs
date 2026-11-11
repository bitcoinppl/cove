//! The notices the screen shows: the heal notice from the stored record and the outcome of the
//! last control that didn't do what it was asked
//!
//! Each has one owner. The heal notice lives in the lifecycle record, which only the lifecycle
//! owner writes, so the projection reads it from there. The last outcome is decided by dispatch,
//! the one function that takes an intent, so the supervisor keeps it in a [`LastOutcome`]

use super::types::{
    CloudBackupHealNotice, CloudBackupLastOutcome, CloudBackupOutcome, CloudBackupOutcomeTarget,
    CloudBackupRejection, CloudBackupRowControl, CloudBackupRowTarget, CloudBackupScreenAction,
};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::admission::{Admission, ReviewExit, UserIntent};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupNoticeKind, RecordId};

/// The notices the screen projection reads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Notices {
    pub(crate) heal: Option<CloudBackupHealNotice>,
    pub(crate) last_outcome: Option<CloudBackupLastOutcome>,
}

impl Notices {
    /// The heal notice `record` holds, and the last outcome dispatch recorded
    pub(crate) fn new(record: &PersistedCloudBackupState, last_outcome: &LastOutcome) -> Self {
        let heal = match record {
            PersistedCloudBackupState::Configured(configured) => {
                configured.heal_notice.map(CloudBackupHealNotice::from)
            }
            _ => None,
        };

        Self { heal, last_outcome: last_outcome.outcome.clone() }
    }
}

/// The outcome of the last control that didn't simply do what it was asked
///
/// Only the supervisor holds one. Dispatch records each admission of an intent a control sends:
/// a rejection is shown next to that control, and a later accepted tap on any control replaces
/// it, since the user moved on. An operation a control started records its failure here when it
/// ends, next to the same control
#[derive(Debug, Default)]
pub(crate) struct LastOutcome {
    outcome: Option<CloudBackupLastOutcome>,
    /// The control that started the running operation, for its failure
    started_by: Option<(CloudBackupExclusiveOperationClaim, CloudBackupOutcomeTarget)>,
}

impl LastOutcome {
    /// Records what admission decided for `intent`; true when the shown outcome changed
    ///
    /// Intents no control sends, such as entering the detail screen, record nothing, and a
    /// coalesced tap leaves the outcome as it was, since the running work it joined is on screen
    pub(crate) fn admitted(&mut self, intent: &UserIntent, admission: Admission) -> bool {
        let Some(target) = target_of(intent) else { return false };

        let next = match admission {
            Admission::Accepted => None,
            Admission::Coalesced(_) => return false,
            Admission::Rejected(rejection) => Some(CloudBackupLastOutcome {
                target,
                outcome: CloudBackupOutcome::Rejected {
                    reason: CloudBackupRejection::from(rejection),
                },
            }),
        };

        self.replace(next)
    }

    /// `claim` started from the control `intent` came from, so its failure is drawn there
    pub(crate) fn started(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        intent: &UserIntent,
    ) {
        self.started_by = target_of(intent).map(|target| (claim, target));
    }

    /// The operation `claim` ended with `error`; true when the shown outcome changed
    ///
    /// An operation no control started, such as onboarding's restore, is drawn next to the
    /// control that would start it on this screen
    pub(crate) fn operation_failed(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        error: &CloudBackupError,
    ) -> bool {
        let target = match self.started_by.take() {
            Some((started, target)) if started == claim => Some(target),
            _ => operation_control(claim.operation()).map(CloudBackupOutcomeTarget::Action),
        };
        let Some(target) = target else { return false };

        self.replace(Some(CloudBackupLastOutcome { target, outcome: failure(error) }))
    }

    /// Work `action` did in the supervisor's turn, with no operation, ended with `error`; true
    /// when the shown outcome changed
    pub(crate) fn action_failed(
        &mut self,
        action: CloudBackupScreenAction,
        error: &CloudBackupError,
    ) -> bool {
        self.action_ended(action, failure(error))
    }

    /// Work `action` started ended with `outcome` instead of doing what it was asked, such as a
    /// Turn Off that stopped before it committed; true when the shown outcome changed
    pub(crate) fn action_ended(
        &mut self,
        action: CloudBackupScreenAction,
        outcome: CloudBackupOutcome,
    ) -> bool {
        let target = CloudBackupOutcomeTarget::Action(action);
        self.replace(Some(CloudBackupLastOutcome { target, outcome }))
    }

    /// The user dismissed the shown outcome; true when there was one
    pub(crate) fn dismiss(&mut self) -> bool {
        self.replace(None)
    }

    fn replace(&mut self, next: Option<CloudBackupLastOutcome>) -> bool {
        if self.outcome == next {
            return false;
        }

        self.outcome = next;
        true
    }
}

/// A cancelled passkey prompt is the user's choice, not a failure to report
fn failure(error: &CloudBackupError) -> CloudBackupOutcome {
    if error.is_passkey_not_completed() {
        return CloudBackupOutcome::PasskeyNotCompleted;
    }

    CloudBackupOutcome::Failed { message: error.reader_message() }
}

/// The control `intent` is sent from, or `None` for an intent no control sends
///
/// A row intent names its row control, not the confirmation it asked for, so its outcome stays on
/// that control when a refresh changes the confirmation
fn target_of(intent: &UserIntent) -> Option<CloudBackupOutcomeTarget> {
    use CloudBackupScreenAction as A;
    use UserIntent as I;

    let action = match intent {
        I::RestoreCloudWallet(record_id) => {
            return Some(row_target(record_id, CloudBackupRowControl::Restore));
        }
        I::DeleteCloudWallet(record_id) => {
            return Some(row_target(record_id, CloudBackupRowControl::Delete));
        }
        I::Enable(..) => A::Enable,
        I::RestoreFromCloudBackup => A::RestoreFromCloudBackup,
        I::CancelRestore => A::CancelRestore,
        I::ConfirmSavedPasskey => A::ConfirmSavedPasskey,
        I::AcceptEnablePrompt(choice) => A::accepting_enable_prompt(*choice),
        I::DiscardPendingEnable => A::DiscardPendingEnable,
        I::ConfirmPendingEnableCleanup => A::ConfirmPendingEnableCleanup,
        I::StartVerification { .. } => A::Verify,
        I::RepairPasskey { .. } => A::RepairPasskey,
        I::SyncUnsynced => A::SyncUnsynced,
        I::RefreshDetail => A::RefreshDetail,
        I::RecreateManifest => A::RecreateManifest,
        I::ReinitializeBackup => A::ReinitializeBackup,
        I::StartRestoreAll => A::StartRestoreAll,
        I::RetryRestoreAllRemaining => A::RetryRestoreAllRemaining,
        I::CancelRestoreAll => A::CancelRestoreAll,
        I::DeleteUndecryptableWalletBackups => A::DeleteUnreadableBackups,
        I::RecoverOtherBackups => A::RecoverOtherBackups,
        I::DeleteOtherBackups => A::DeleteOtherBackups,
        I::TurnOff => A::TurnOff,
        I::ConfirmDisable { .. } => A::ConfirmDisable,
        I::CancelDisable => A::CancelDisable,
        I::LeaveReview { to: ReviewExit::RestoreFirst, .. } => A::RestoreFirst,
        I::LeaveReview { to: ReviewExit::VerifyFirst, .. } => A::VerifyFirst,
        I::ResumeDisable => A::ResumeDisable,
        I::StopUsingOnThisDevice => A::StopUsingOnThisDevice,
        I::DismissNotice(CloudBackupNoticeKind::Heal) => A::DismissHealNotice,
        I::ReconnectDrive => A::ReconnectDrive,
        I::SwitchDriveAccount => A::SwitchDriveAccount,
        I::RetryDriveAccountSwitchRecovery => A::RetryDriveAccountSwitchRecovery,
        I::CancelDriveAccountSwitch => A::CancelDriveAccountSwitch,
        I::RetryLoad => A::RetryLoad,
        I::ResetOnThisDevice => A::ResetOnThisDevice,
        // prompts, screen lifecycle and platform calls have no control to draw an outcome by
        I::DismissNotice(CloudBackupNoticeKind::LastOutcome)
        | I::DismissPasskeyChoicePrompt
        | I::DismissMissingPasskeyReminder
        | I::DismissVerificationPrompt
        | I::PromptEnablePasskeyChoice(_)
        | I::EnterDetail
        | I::LeaveDetail
        | I::LocalReset => return None,
    };

    Some(CloudBackupOutcomeTarget::Action(action))
}

fn row_target(record_id: &RecordId, control: CloudBackupRowControl) -> CloudBackupOutcomeTarget {
    CloudBackupOutcomeTarget::Row {
        target: CloudBackupRowTarget { record_id: record_id.clone(), control },
    }
}

/// The global control that starts `operation`, for a failure no dispatch started
fn operation_control(operation: CloudBackupExclusiveOperation) -> Option<CloudBackupScreenAction> {
    use CloudBackupExclusiveOperation as Op;
    use CloudBackupScreenAction as A;

    let action = match operation {
        Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery => A::Enable,
        Op::Restore => A::RestoreFromCloudBackup,
        Op::Disable => A::TurnOff,
        Op::RecreateManifest => A::RecreateManifest,
        Op::ReinitializeBackup | Op::VerifyReinitializedBackup => A::ReinitializeBackup,
        Op::RepairPasskey => A::RepairPasskey,
        Op::Verify => A::Verify,
        Op::RecoverOtherBackups => A::RecoverOtherBackups,
        Op::DeleteOtherBackups => A::DeleteOtherBackups,
        Op::RestoreAllCloudWallets => A::StartRestoreAll,
        Op::DeleteUndecryptableWalletBackups => A::DeleteUnreadableBackups,
        Op::DiscardPendingEnable => A::DiscardPendingEnable,
        // a row operation names its record, which only the dispatch that started it knows
        Op::RestoreCloudWallet | Op::DeleteCloudWallet => return None,
    };

    Some(action)
}
