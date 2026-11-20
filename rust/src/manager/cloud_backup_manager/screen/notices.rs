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
use cove_cspp::backup_data::PasskeyProviderHint;

use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::admission::{Admission, ReviewExit, UserIntent};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupError, CloudBackupNoticeKind, CloudBackupPasskeyUnlockHint, RecordId,
};

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
    /// The Enable that Use Existing on the existing-backup prompt started, which checks the same
    /// backup again
    existing_backup_retry: Option<CloudBackupExclusiveOperationClaim>,
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
        self.operation_ended(claim, failure(error))
    }

    /// The operation `claim` ended with `error` on a passkey the keychain doesn't hold yet, such as
    /// one just saved and waiting to be confirmed; true when the shown outcome changed
    ///
    /// A request for it that didn't finish names what to unlock from where that passkey was
    /// created, `passkey`, not from the passkey the keychain holds
    pub(crate) fn operation_failed_on_passkey(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        error: &CloudBackupError,
        passkey: Option<&PasskeyProviderHint>,
    ) -> bool {
        let outcome = failure_on(error, || CloudBackupPasskeyUnlockHint::for_provider(passkey));
        self.operation_ended(claim, outcome)
    }

    /// The operation `claim` ended with `outcome` instead of doing all it was asked, such as a
    /// batch delete that stopped partway; true when the shown outcome changed
    pub(crate) fn operation_ended(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        outcome: CloudBackupOutcome,
    ) -> bool {
        let target = match self.started_by.take() {
            Some((started, target)) if started == claim => Some(target),
            _ => operation_control(claim.operation()).map(CloudBackupOutcomeTarget::Action),
        };
        let Some(target) = target else { return false };

        self.replace(Some(CloudBackupLastOutcome { target, outcome }))
    }

    /// The Enable `claim` was started by Use Existing on the existing-backup prompt, so it checks
    /// the backup that prompt showed again
    pub(crate) fn retrying_existing_backup(&mut self, claim: CloudBackupExclusiveOperationClaim) {
        self.existing_backup_retry = Some(claim);
    }

    /// The Enable `claim` that the existing-backup prompt's Use Existing started came back to a
    /// prompt with `error`; true when the shown outcome changed
    ///
    /// The answer is drawn with its outcome, so trying it again never looks like nothing happened.
    /// An Enable started anywhere else, such as the passkey choice's Use Existing, shows the prompt
    /// for the first time, whose own message says what it found, so nothing is drawn for it
    pub(crate) fn existing_backup_retry_failed(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        error: &CloudBackupError,
    ) -> bool {
        if self.existing_backup_retry.take() != Some(claim) {
            return false;
        }

        self.use_existing_failed(claim, error)
    }

    /// The Enable `claim` that an Enable prompt's Use Existing started tried a passkey that failed
    /// with `error`; true when the shown outcome changed
    ///
    /// The user tried a passkey whichever prompt offered it, so the prompt it comes back to says
    /// why, rather than asking for the same answer again with no reason
    pub(crate) fn use_existing_failed(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        error: &CloudBackupError,
    ) -> bool {
        let target = CloudBackupOutcomeTarget::Action(
            CloudBackupScreenAction::AcceptEnablePromptUseExisting,
        );
        self.existing_backup_retry.take_if(|retry| *retry == claim);

        let retried = self
            .started_by
            .take_if(|(started, started_target)| *started == claim && *started_target == target);
        if retried.is_none() {
            return false;
        }

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
///
/// A locked password manager ends the request the same way as a cancel, so the outcome carries
/// what to unlock for the passkey this device holds, read only when the request didn't finish
fn failure(error: &CloudBackupError) -> CloudBackupOutcome {
    failure_on(error, CloudBackupPasskeyUnlockHint::for_active_passkey)
}

/// [`failure`] for a request on the passkey `unlock_hint` reads what to unlock for
fn failure_on(
    error: &CloudBackupError,
    unlock_hint: impl FnOnce() -> Option<CloudBackupPasskeyUnlockHint>,
) -> CloudBackupOutcome {
    if error.is_passkey_not_completed() {
        return CloudBackupOutcome::PasskeyNotCompleted { unlock_hint: unlock_hint() };
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
        I::DeleteCloudWallet(consent) => {
            return Some(row_target(consent.record_id(), CloudBackupRowControl::Delete));
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
        I::DeleteUndecryptableWalletBackups(_) => A::DeleteUnreadableBackups,
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
