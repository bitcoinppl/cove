//! Admission: the one decision about what Cloud Backup does with a user intent
//!
//! Every [`CloudBackupManagerAction`] becomes a [`UserIntent`] through an exhaustive `match`, so
//! a new action does not compile until it is mapped here. [`evaluate_admission`] decides what
//! happens to an intent from an [`AdmissionSnapshot`], and is pure: the UI-side checks and
//! `CloudBackupSupervisor::dispatch` call the same function, so a projected action and a
//! dispatched one cannot disagree

use super::screen::CloudBackupTurnOffFailure;
use super::turn_off::FinishFailure;
use crate::database::cloud_backup::DriveAccountSwitchId;

use super::model::CloudBackupExclusiveOperationClaim;
use super::{CloudBackupBatchDeleteConsent, CloudBackupRowDeleteConsent};
use super::{
    CloudBackupEnableContext, CloudBackupEnablePromptChoice, CloudBackupExclusiveOperation,
    CloudBackupManagerAction, CloudBackupNoticeKind, CloudBackupVerificationSource, RecordId,
};

/// A user tap or platform call, before admission
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserIntent {
    Enable(EnableIntent, CloudBackupEnableContext),
    ConfirmSavedPasskey,
    DiscardPendingEnable,
    ConfirmPendingEnableCleanup,
    DismissPasskeyChoicePrompt,
    DismissMissingPasskeyReminder,
    RestoreFromCloudBackup,
    CancelRestore,
    StartVerification {
        source: CloudBackupVerificationSource,
        discoverable: bool,
    },
    DismissVerificationPrompt,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey {
        no_discovery: bool,
    },
    SyncUnsynced,
    RestoreCloudWallet(RecordId),
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    /// A row's Delete, with the consent its confirmation was opened for
    DeleteCloudWallet(CloudBackupRowDeleteConsent),
    /// The batch delete of undecryptable backups, frozen to the records its consent names
    DeleteUndecryptableWalletBackups(CloudBackupBatchDeleteConsent),
    RecoverOtherBackups,
    DeleteOtherBackups,
    TurnOff,
    ConfirmDisable {
        review_id: u64,
    },
    CancelDisable,
    /// A Turn Off review button that leaves the review for the work it warned about
    LeaveReview {
        review_id: u64,
        to: ReviewExit,
    },
    ResumeDisable,
    StopUsingOnThisDevice,
    DismissNotice(CloudBackupNoticeKind),
    ReconnectDrive,
    RefreshDetail,
    EnterDetail,
    LeaveDetail,
    RetryLoad,
    ResetOnThisDevice,
    PromptEnablePasskeyChoice(CloudBackupEnableContext),
    AcceptEnablePrompt(CloudBackupEnablePromptChoice),
    /// The user starts moving Cloud Backup to another Google Drive account
    SwitchDriveAccount,
    /// Cancel on a switch that needs attention: restores the previous account
    CancelDriveAccountSwitch,
    /// Try Again on a switch that needs attention: runs its failed step again
    RetryDriveAccountSwitchRecovery,
    /// A full wipe resets Cloud Backup's local state
    LocalReset,
}

impl UserIntent {
    /// Whether this intent may reach work paused on its saved passkey: the pause's own exits, a
    /// local reset, and the prompts and screen lifecycle that start no work
    // exhaustive on purpose: a new intent is decided here before it can reach a pause
    #[deny(clippy::wildcard_enum_match_arm)]
    fn answers_saved_passkey_pause(&self) -> bool {
        match self {
            Self::ConfirmSavedPasskey
            | Self::DiscardPendingEnable
            | Self::LocalReset
            | Self::DismissPasskeyChoicePrompt
            | Self::DismissMissingPasskeyReminder
            | Self::DismissVerificationPrompt
            | Self::DismissNotice(_)
            | Self::EnterDetail
            | Self::LeaveDetail => true,
            Self::Enable(..)
            | Self::ConfirmPendingEnableCleanup
            | Self::RestoreFromCloudBackup
            | Self::CancelRestore
            | Self::StartVerification { .. }
            | Self::RecreateManifest
            | Self::ReinitializeBackup
            | Self::RepairPasskey { .. }
            | Self::SyncUnsynced
            | Self::RestoreCloudWallet(_)
            | Self::StartRestoreAll
            | Self::RetryRestoreAllRemaining
            | Self::CancelRestoreAll
            | Self::DeleteCloudWallet(_)
            | Self::DeleteUndecryptableWalletBackups(_)
            | Self::RecoverOtherBackups
            | Self::DeleteOtherBackups
            | Self::TurnOff
            | Self::ConfirmDisable { .. }
            | Self::CancelDisable
            | Self::LeaveReview { .. }
            | Self::ResumeDisable
            | Self::StopUsingOnThisDevice
            | Self::ReconnectDrive
            | Self::RefreshDetail
            | Self::RetryLoad
            | Self::ResetOnThisDevice
            | Self::PromptEnablePasskeyChoice(_)
            | Self::AcceptEnablePrompt(_)
            | Self::SwitchDriveAccount
            | Self::CancelDriveAccountSwitch
            | Self::RetryDriveAccountSwitchRecovery => false,
        }
    }
}

/// The work a Turn Off review button leaves the review for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewExit {
    /// Restore Them First: restore the wallets the review found only in the cloud
    RestoreFirst,
    /// Verify First: verify the backups the review couldn't check
    VerifyFirst,
}

/// Which Enable the user asked for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnableIntent {
    Standard,
    ForceNew,
    NoDiscovery,
}

impl EnableIntent {
    /// The operation this Enable runs as
    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        match self {
            Self::Standard => CloudBackupExclusiveOperation::Enable,
            Self::ForceNew => CloudBackupExclusiveOperation::EnableForceNew,
            Self::NoDiscovery => CloudBackupExclusiveOperation::EnableNoDiscovery,
        }
    }
}

impl From<CloudBackupManagerAction> for UserIntent {
    // exhaustive on purpose: a new action is a compile error until it has an intent, and the
    // lint keeps a catch-all arm from ever making it compile unmapped
    #[deny(clippy::wildcard_enum_match_arm)]
    fn from(action: CloudBackupManagerAction) -> Self {
        use CloudBackupManagerAction as A;

        match action {
            A::EnableCloudBackup(context) => Self::Enable(EnableIntent::Standard, context),
            A::EnableCloudBackupForceNew(context) => Self::Enable(EnableIntent::ForceNew, context),
            A::EnableCloudBackupNoDiscovery(context) => {
                Self::Enable(EnableIntent::NoDiscovery, context)
            }
            A::ConfirmSavedPasskey => Self::ConfirmSavedPasskey,
            A::DiscardPendingEnableCloudBackup => Self::DiscardPendingEnable,
            A::ConfirmPendingEnableCleanup => Self::ConfirmPendingEnableCleanup,
            A::DismissPasskeyChoicePrompt => Self::DismissPasskeyChoicePrompt,
            A::DismissMissingPasskeyReminder => Self::DismissMissingPasskeyReminder,
            A::RestoreFromCloudBackup => Self::RestoreFromCloudBackup,
            A::CancelRestore => Self::CancelRestore,
            A::StartVerification(source) => Self::StartVerification { source, discoverable: false },
            A::StartVerificationDiscoverable(source) => {
                Self::StartVerification { source, discoverable: true }
            }
            A::DismissVerificationPrompt => Self::DismissVerificationPrompt,
            A::RecreateManifest => Self::RecreateManifest,
            A::ReinitializeBackup => Self::ReinitializeBackup,
            A::RepairPasskey => Self::RepairPasskey { no_discovery: false },
            A::RepairPasskeyNoDiscovery => Self::RepairPasskey { no_discovery: true },
            A::SyncUnsynced => Self::SyncUnsynced,
            A::RestoreCloudWallet(record_id) => Self::RestoreCloudWallet(record_id),
            A::StartRestoreAll => Self::StartRestoreAll,
            A::RetryRestoreAllRemaining => Self::RetryRestoreAllRemaining,
            A::CancelRestoreAll => Self::CancelRestoreAll,
            A::DeleteCloudWallet(consent) => Self::DeleteCloudWallet(consent),
            A::DeleteUndecryptableWalletBackups(consent) => {
                Self::DeleteUndecryptableWalletBackups(consent)
            }
            A::RecoverOtherBackups => Self::RecoverOtherBackups,
            A::DeleteOtherBackups => Self::DeleteOtherBackups,
            A::DisableCloudBackup => Self::TurnOff,
            A::ConfirmDisable { review_id } => Self::ConfirmDisable { review_id },
            A::CancelDisable => Self::CancelDisable,
            A::RestoreBeforeTurnOff { review_id } => {
                Self::LeaveReview { review_id, to: ReviewExit::RestoreFirst }
            }
            A::VerifyBeforeTurnOff { review_id } => {
                Self::LeaveReview { review_id, to: ReviewExit::VerifyFirst }
            }
            A::ResumeDisable => Self::ResumeDisable,
            A::StopUsingOnThisDevice => Self::StopUsingOnThisDevice,
            A::DismissNotice(kind) => Self::DismissNotice(kind),
            A::ReconnectDrive => Self::ReconnectDrive,
            A::SwitchDriveAccount => Self::SwitchDriveAccount,
            A::RetryDriveAccountSwitchRecovery => Self::RetryDriveAccountSwitchRecovery,
            A::CancelDriveAccountSwitch => Self::CancelDriveAccountSwitch,
            A::LeaveDetail => Self::LeaveDetail,
            A::RetryLoad => Self::RetryLoad,
            A::ResetOnThisDevice => Self::ResetOnThisDevice,
            A::RefreshDetail => Self::RefreshDetail,
            A::EnterDetail => Self::EnterDetail,
            A::PromptEnablePasskeyChoice(context) => Self::PromptEnablePasskeyChoice(context),
            A::AcceptEnablePrompt(choice) => Self::AcceptEnablePrompt(choice),
        }
    }
}

/// What dispatch does with an intent
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Accepted,
    /// The intent joins work that is already running; only the closed set below can coalesce
    Coalesced(CoalescibleIntent),
    Rejected(Rejection),
}

/// The only intents that may coalesce, each of which leaves visible feedback on screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoalescibleIntent {
    /// A detail refresh is already loading, and the inventory shows it loading; the tap gives
    /// the load after it the tap's authority
    RefreshDetail,
    /// A sync is already running, and the wallet rows show it syncing
    SyncUnsynced,
    /// The same operation is already running, and its progress is on screen
    RepeatOfRunning(CloudBackupExclusiveOperation),
}

/// Why an intent was rejected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    CloudBackupOff,
    /// Enable or restore was asked for while a backup is already configured
    AlreadyOn,
    /// The work needs a configured backup, and the Enable or restore that sets one up hasn't
    /// finished
    SetupNotFinished,
    /// The stored record could not be loaded; Try Again runs the load again
    LoadFailed,
    /// The stored record can't be read as a backup; only resetting Cloud Backup on this device
    /// runs on it
    Corrupted,
    /// Turn Off already deleted remote data, so it only goes forward
    TurnOffCommitted,
    OperationRunning(CloudBackupExclusiveOperation),
    DetailInventoryNotLoaded,
    OtherBackupsNotLoaded,
    RestoreAllUnavailable,
    NothingToCancel,
    /// Nothing is waiting to be continued or discarded in this lifecycle, such as a saved
    /// passkey with no paused Enable, or a Discard after the Enable finished
    NothingToContinue,
    /// A local reset is wiping Cloud Backup's local state
    Resetting,
    /// What the confirmation answers is no longer current: the Turn Off review, or the rows and
    /// evidence a delete's consent names
    ReviewStale,
    /// Stop Using on This Device is offered only after a failed Try Again
    TryAgainFirst,
    /// A recorded Google Drive account switch holds the backup until its Retry or Cancel
    DriveAccountSwitch,
}

/// Why the write lane is not running a command class right now
///
/// An admission fact, not a storage issue: it is never persisted in a failed blob row, and a
/// paused command keeps its local state as it was, so it runs again once the pause ends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseReason {
    /// Turn Off is running or finishing
    TurnOff,
    /// A Google Drive account switch holds the backup
    AccountSwitch,
    /// A local reset is wiping Cloud Backup's local state
    LocalReset,
    /// Cloud Backup is not configured on this device
    BackupOff,
    /// The stored record could not be loaded
    LoadFailed,
    /// The command was read under an older configuration; its work is redone under the new one
    ConfigurationChanged,
    /// A reinitialize is replacing the configuration; background work runs again under the new one
    Reinitialize,
}

/// The lifecycle body admission decides on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionBody {
    Disabled,
    Enabling,
    Restoring,
    Configured,
    /// A committed Turn Off that still has to finish
    FinishingDisable,
    PendingEnableRecovery,
    /// A recorded Google Drive account switch; only its Retry and Cancel reach it
    DriveAccountSwitch,
    LoadFailed,
    /// The stored record can't be read as a backup
    Corrupted,
    /// A local reset closed admission until it finishes
    Resetting,
}

/// Detail inventory readiness, as row actions need it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailInventory {
    NotLoaded,
    /// Rows exist but the listing is incomplete
    Ready,
    Complete,
}

/// Where Restore All stands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreAllAdmission {
    Unavailable,
    StartAvailable,
    RetryAvailable,
    Running,
}

/// Where Turn Off stands outside a running operation
///
/// The review holds no operation, so it is its own fact; a committed Turn Off whose Try Again
/// failed is the only state that offers Stop Using on This Device, and one the provider stopped
/// for consent the only one that offers Reconnect
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnOffAdmission {
    None,
    /// The silent listing behind the review is running
    Preparing,
    /// The provider couldn't be reached; Try Again taps Turn Off again
    Offline,
    Reviewing {
        review_id: u64,
        /// What Restore Them First restores, when the review names wallets not on this device
        restore_first: Option<RestoreTarget>,
    },
    /// Why the last finishing run of the committed Turn Off stopped
    FinishFailed(FinishFailure),
}

/// The restore Restore Them First starts: the one wallet the review found only in the cloud, or
/// every such wallet through Restore All
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreTarget {
    OneWallet,
    AllWallets,
}

impl TurnOffAdmission {
    /// The review a Confirm would consent to, when one is shown
    pub(crate) fn review_id(self) -> Option<u64> {
        match self {
            Self::Reviewing { review_id, .. } => Some(review_id),
            Self::None | Self::Preparing | Self::Offline | Self::FinishFailed(_) => None,
        }
    }
}

/// Everything admission reads, taken in one place at one time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdmissionSnapshot {
    pub(crate) body: AdmissionBody,
    pub(crate) running: Option<CloudBackupExclusiveOperation>,
    pub(crate) sync_running: bool,
    pub(crate) detail_inventory: DetailInventory,
    /// The inventory shows a detail load in progress
    pub(crate) detail_checking: bool,
    pub(crate) other_backups_loaded: bool,
    pub(crate) restore_all: RestoreAllAdmission,
    /// An Enable or reinitialize that stopped before it finished, if one is waiting
    pub(crate) continuation: Option<PausedContinuation>,
    /// What each answer to the Enable prompt on screen starts
    pub(crate) enable_prompt: EnablePromptAnswers,
    /// What a Turn Off intent can act on: the held review and the running Turn Off's stage
    pub(crate) turn_off: TurnOffAdmission,
    /// What a Google Drive account switch's Retry and Cancel can act on
    pub(crate) drive_account_switch: SwitchAdmission,
}

/// Where a Google Drive account switch stands for the user
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchAdmission {
    /// No switch waits for the user; a recorded one moves on its own or waits on its passkey
    None,
    /// A step of the switch failed, or the switch stopped, and waits for the user's Try Again
    NeedsAttention {
        /// The platform hasn't committed the new account, so Cancel can restore the previous one
        can_cancel: bool,
    },
}

/// The Enable each answer to the Enable prompt on screen starts; both are `None` with no prompt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EnablePromptAnswers {
    pub(crate) use_existing: Option<EnableIntent>,
    pub(crate) create_new: Option<EnableIntent>,
}

impl EnablePromptAnswers {
    /// The Enable `choice` starts, if the prompt on screen offers it
    fn starts(self, choice: CloudBackupEnablePromptChoice) -> Option<EnableIntent> {
        match choice {
            CloudBackupEnablePromptChoice::UseExisting => self.use_existing,
            CloudBackupEnablePromptChoice::CreateNew => self.create_new,
        }
    }
}

/// An Enable or reinitialize that stopped before it finished, as admission sees it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PausedContinuation {
    pub(crate) owner: ContinuationOwner,
    pub(crate) stage: ContinuationStage,
}

/// The lifecycle a paused continuation belongs to, which decides what continues it
///
/// A continuation only goes on in its own lifecycle: a fresh Enable continues while the backup is
/// off, a reinitialize continues on the configured backup it replaces, and a Google Drive account
/// switch's reinitialize continues in the switch it belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinuationOwner {
    /// An Enable on a device without a configured backup
    Enable,
    /// A reinitialize of the configured backup
    Reinitialize,
    /// The reinitialize of the Google Drive account switch it names
    DriveAccountSwitch(DriveAccountSwitchId),
}

/// Where a paused continuation stopped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinuationStage {
    /// A registered passkey waits for the user to confirm they saved it
    AwaitingSavedPasskey,
    /// Enable asks the user how to go on, at the existing-backup prompt or the passkey choice;
    /// nothing is held
    EnablePrompt,
}

impl ContinuationOwner {
    /// The owner of the work an Enable flow runs under `claim`
    ///
    /// The Enable flow runs under one of Enable's claims or a reinitialize's, and a switch's
    /// reinitialize claim names its switch
    pub(crate) fn of(claim: CloudBackupExclusiveOperationClaim) -> Self {
        match (claim.operation(), claim.drive_account_switch_id()) {
            (_, Some(transition_id)) => Self::DriveAccountSwitch(transition_id),
            (CloudBackupExclusiveOperation::ReinitializeBackup, None) => Self::Reinitialize,
            (_, None) => Self::Enable,
        }
    }

    /// The owner of work found paused in `body` at restart
    ///
    /// A fresh Enable runs only while the backup is off, a reinitialize only on a configured
    /// backup, and a switch's reinitialize only in the recorded switch `switch` names
    pub(crate) fn of_body(body: AdmissionBody, switch: Option<DriveAccountSwitchId>) -> Self {
        match (body, switch) {
            (AdmissionBody::DriveAccountSwitch, Some(transition_id)) => {
                Self::DriveAccountSwitch(transition_id)
            }
            (AdmissionBody::Configured, _) => Self::Reinitialize,
            (
                AdmissionBody::Disabled
                | AdmissionBody::Enabling
                | AdmissionBody::Restoring
                | AdmissionBody::FinishingDisable
                | AdmissionBody::PendingEnableRecovery
                | AdmissionBody::DriveAccountSwitch
                | AdmissionBody::LoadFailed
                | AdmissionBody::Corrupted
                | AdmissionBody::Resetting,
                _,
            ) => Self::Enable,
        }
    }

    /// The operation that continues the paused work
    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        match self {
            Self::Enable => CloudBackupExclusiveOperation::Enable,
            Self::Reinitialize | Self::DriveAccountSwitch(_) => {
                CloudBackupExclusiveOperation::ReinitializeBackup
            }
        }
    }

    /// Whether `body` is the lifecycle this continuation belongs to
    fn owns(self, body: AdmissionBody) -> bool {
        match (self, body) {
            (Self::Enable, AdmissionBody::Disabled | AdmissionBody::PendingEnableRecovery)
            | (Self::Reinitialize, AdmissionBody::Configured)
            | (Self::DriveAccountSwitch(_), AdmissionBody::DriveAccountSwitch) => true,
            (
                Self::Enable | Self::Reinitialize | Self::DriveAccountSwitch(_),
                AdmissionBody::Disabled
                | AdmissionBody::Enabling
                | AdmissionBody::Restoring
                | AdmissionBody::Configured
                | AdmissionBody::FinishingDisable
                | AdmissionBody::PendingEnableRecovery
                | AdmissionBody::DriveAccountSwitch
                | AdmissionBody::LoadFailed
                | AdmissionBody::Corrupted
                | AdmissionBody::Resetting,
            ) => false,
        }
    }
}

impl PausedContinuation {
    /// The paused continuation an intent of the current body can act on
    fn in_body(snapshot: &AdmissionSnapshot) -> Option<Self> {
        snapshot.continuation.filter(|paused| paused.owner.owns(snapshot.body))
    }
}

impl AdmissionSnapshot {
    /// The owner of work paused on its saved passkey that the current body continues, while
    /// nothing runs: Confirm Saved Passkey is then the way that work goes on
    ///
    /// A switch whose step failed, such as saving the pause itself, needs the user's Try Again
    /// first, so its pause doesn't own the screen or close admission to that step
    pub(crate) fn paused_on_saved_passkey(&self) -> Option<ContinuationOwner> {
        if self.running.is_some() {
            return None;
        }

        // a failed switch leads with its own recovery: its pause may not have been saved
        if matches!(self.drive_account_switch, SwitchAdmission::NeedsAttention { .. }) {
            return None;
        }

        PausedContinuation::in_body(self)
            .filter(|paused| paused.stage == ContinuationStage::AwaitingSavedPasskey)
            .map(|paused| paused.owner)
    }
}

/// Decides what happens to `intent` in `snapshot`
///
/// Pure: it does not log or allocate, so projections can call it as often as they like
pub(crate) fn evaluate_admission(intent: &UserIntent, snapshot: &AdmissionSnapshot) -> Admission {
    use Admission::{Accepted, Coalesced, Rejected};
    use CloudBackupExclusiveOperation as Op;
    use UserIntent as I;

    match intent {
        // a reset closed admission: nothing starts and nothing is presented until it finishes
        _ if snapshot.body == AdmissionBody::Resetting => Rejected(Rejection::Resetting),

        // continues a paused Enable or reinitialize in the lifecycle it belongs to; while one
        // runs, its confirmation is already on screen
        I::ConfirmSavedPasskey => match (PausedContinuation::in_body(snapshot), snapshot.running) {
            (_, Some(running @ (Op::Enable | Op::ReinitializeBackup))) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(running))
            }
            (_, Some(running)) => Rejected(Rejection::OperationRunning(running)),
            (
                Some(PausedContinuation { stage: ContinuationStage::AwaitingSavedPasskey, .. }),
                None,
            ) => Accepted,
            (
                Some(PausedContinuation { stage: ContinuationStage::EnablePrompt, .. }) | None,
                None,
            ) => Rejected(Rejection::NothingToContinue),
        },

        // ends a paused Enable or reinitialize; a Discard from a prompt that outlived its
        // continuation has nothing to end, so it changes nothing on screen
        I::DiscardPendingEnable => {
            match (PausedContinuation::in_body(snapshot), snapshot.running) {
                (_, Some(Op::DiscardPendingEnable)) => {
                    Coalesced(CoalescibleIntent::RepeatOfRunning(Op::DiscardPendingEnable))
                }
                (_, Some(running)) => Rejected(Rejection::OperationRunning(running)),
                // a switch's reinitialize has no Discard: the platform staged the new account
                // for it, and only the switch decides whether that account stays
                (
                    Some(PausedContinuation {
                        owner: ContinuationOwner::DriveAccountSwitch(_),
                        ..
                    }),
                    None,
                ) => Rejected(Rejection::NothingToCancel),
                (Some(_), None) => Accepted,
                (None, None) => Rejected(Rejection::NothingToContinue),
            }
        }

        // a full wipe is admitted from every body; only another reset closes it, above
        I::LocalReset => Accepted,

        // work paused on its saved passkey owns the screen until its Confirm or Discard, above:
        // nothing else starts on the backup that work is replacing
        _ if !intent.answers_saved_passkey_pause()
            && let Some(owner) = snapshot.paused_on_saved_passkey() =>
        {
            Rejected(Rejection::OperationRunning(owner.operation()))
        }

        // a switch's Try Again and Cancel answer the step that failed; a switch that moves on
        // its own needs neither
        I::RetryDriveAccountSwitchRecovery => match snapshot.drive_account_switch {
            SwitchAdmission::NeedsAttention { .. } => Accepted,
            SwitchAdmission::None => Rejected(Rejection::NothingToContinue),
        },
        I::SwitchDriveAccount => configured_and_idle(snapshot, Some(Op::ReinitializeBackup)),
        I::CancelDriveAccountSwitch => match snapshot.drive_account_switch {
            SwitchAdmission::NeedsAttention { can_cancel: true } => Accepted,
            SwitchAdmission::NeedsAttention { can_cancel: false } | SwitchAdmission::None => {
                Rejected(Rejection::NothingToCancel)
            }
        },

        // presentation intents answer prompts; their handlers own the prompt they answer
        I::ConfirmPendingEnableCleanup
        | I::DismissPasskeyChoicePrompt
        | I::DismissMissingPasskeyReminder
        | I::DismissVerificationPrompt
        | I::PromptEnablePasskeyChoice(_)
        | I::EnterDetail => Accepted,

        // a tap while the inventory shows a load joins it, and the load after it lists with the
        // tap's authority, since the running one may be silent; a silent refresh under a loaded
        // inventory shows nothing, so the tap is accepted and queues the next load
        I::RefreshDetail if snapshot.detail_checking => Coalesced(CoalescibleIntent::RefreshDetail),
        I::RefreshDetail => Accepted,

        I::Enable(kind, _) => starts_while_off(snapshot, kind.operation()),

        // an answer to the Enable prompt is the Enable it starts, decided as that Enable, so the
        // prompt stays on screen when that Enable can't start
        I::AcceptEnablePrompt(choice) => match snapshot.enable_prompt.starts(*choice) {
            Some(kind) => starts_while_off(snapshot, kind.operation()),
            None => Rejected(Rejection::NothingToContinue),
        },
        I::RestoreFromCloudBackup => starts_while_off(snapshot, Op::Restore),

        I::CancelRestore => match snapshot.running {
            Some(Op::Restore) => Accepted,
            _ => Rejected(Rejection::NothingToCancel),
        },
        I::CancelRestoreAll => match snapshot.restore_all {
            RestoreAllAdmission::Running => Accepted,
            _ => Rejected(Rejection::NothingToCancel),
        },

        // the review holds no operation, so Turn Off is admitted beside a review, and a tap
        // while the review is preparing joins it
        I::TurnOff => match (snapshot.body, snapshot.running) {
            (AdmissionBody::Configured, None)
                if snapshot.turn_off == TurnOffAdmission::Preparing =>
            {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::Configured, None) => Accepted,
            (AdmissionBody::Configured, Some(Op::Disable)) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::Configured, Some(running)) => {
                Rejected(Rejection::OperationRunning(running))
            }
            (body, _) => Rejected(not_configured(body)),
        },

        // consent answers exactly the review on screen; any change since ended that review
        I::ConfirmDisable { review_id } => match (snapshot.body, snapshot.running) {
            (AdmissionBody::Configured, Some(Op::Disable)) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::Configured, None)
                if snapshot.turn_off.review_id() == Some(*review_id) =>
            {
                Accepted
            }
            (AdmissionBody::Configured, _) => Rejected(Rejection::ReviewStale),
            (body, _) => Rejected(not_configured(body)),
        },
        // Cancel reaches Turn Off until it commits: the review, and the drain before the commit
        I::CancelDisable => match (snapshot.body, snapshot.running, snapshot.turn_off) {
            (AdmissionBody::Configured, Some(Op::Disable), _) => Accepted,
            (
                AdmissionBody::Configured,
                None,
                TurnOffAdmission::Preparing
                | TurnOffAdmission::Offline
                | TurnOffAdmission::Reviewing { .. },
            ) => Accepted,
            (AdmissionBody::FinishingDisable, _, _) => Rejected(Rejection::TurnOffCommitted),
            _ => Rejected(Rejection::NothingToCancel),
        },
        // a leave button answers the review on screen, then is decided as the work it starts, so
        // a rejection starts nothing and the review stays open
        I::LeaveReview { review_id, to } => match (snapshot.turn_off, to) {
            (TurnOffAdmission::Reviewing { review_id: shown, .. }, _) if shown != *review_id => {
                Rejected(Rejection::ReviewStale)
            }
            (
                TurnOffAdmission::Reviewing {
                    restore_first: Some(RestoreTarget::OneWallet), ..
                },
                ReviewExit::RestoreFirst,
            ) => restore_cloud_wallet(snapshot),
            (
                TurnOffAdmission::Reviewing {
                    restore_first: Some(RestoreTarget::AllWallets), ..
                },
                ReviewExit::RestoreFirst,
            ) => start_restore_all(snapshot),
            // the review names no wallet that isn't on this device
            (TurnOffAdmission::Reviewing { restore_first: None, .. }, ReviewExit::RestoreFirst) => {
                Rejected(Rejection::RestoreAllUnavailable)
            }
            (TurnOffAdmission::Reviewing { .. }, ReviewExit::VerifyFirst) => {
                start_verification(snapshot)
            }
            (
                TurnOffAdmission::None
                | TurnOffAdmission::Preparing
                | TurnOffAdmission::Offline
                | TurnOffAdmission::FinishFailed(_),
                _,
            ) => Rejected(Rejection::ReviewStale),
        },
        I::ResumeDisable => match (snapshot.body, snapshot.running) {
            (AdmissionBody::FinishingDisable, None) => Accepted,
            (AdmissionBody::FinishingDisable, Some(Op::Disable)) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::FinishingDisable, Some(running)) => {
                Rejected(Rejection::OperationRunning(running))
            }
            _ => Rejected(Rejection::NothingToContinue),
        },
        I::StopUsingOnThisDevice => match (snapshot.body, snapshot.running, snapshot.turn_off) {
            (AdmissionBody::FinishingDisable, Some(running), _) => {
                Rejected(Rejection::OperationRunning(running))
            }
            (
                AdmissionBody::FinishingDisable,
                None,
                TurnOffAdmission::FinishFailed(FinishFailure::TryAgainFailed(_)),
            ) => Accepted,
            (AdmissionBody::FinishingDisable, None, _) => Rejected(Rejection::TryAgainFirst),
            _ => Rejected(Rejection::NothingToContinue),
        },
        // each is the one exit of its body, and nothing elsewhere for it to continue
        I::RetryLoad => match snapshot.body {
            AdmissionBody::LoadFailed => Accepted,
            _ => Rejected(Rejection::NothingToContinue),
        },
        I::ResetOnThisDevice => match snapshot.body {
            AdmissionBody::Corrupted => Accepted,
            _ => Rejected(Rejection::NothingToContinue),
        },
        I::DismissNotice(_) | I::LeaveDetail => Accepted,
        // Reconnect on a committed Turn Off the provider stopped for consent resumes it with the
        // user's consent; on a configured backup it is the tapped inventory refresh, whose
        // listing may ask for consent, so it joins a refresh already loading as Check Again does
        I::ReconnectDrive => match (snapshot.body, snapshot.running, snapshot.turn_off) {
            (AdmissionBody::FinishingDisable, Some(Op::Disable), _) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::FinishingDisable, Some(running), _) => {
                Rejected(Rejection::OperationRunning(running))
            }
            (AdmissionBody::FinishingDisable, None, TurnOffAdmission::FinishFailed(failure))
                if failure.reason() == CloudBackupTurnOffFailure::AuthorizationRequired =>
            {
                Accepted
            }
            (AdmissionBody::FinishingDisable, None, _) => Rejected(Rejection::NothingToContinue),
            (AdmissionBody::Configured, ..) if snapshot.detail_checking => {
                Coalesced(CoalescibleIntent::RefreshDetail)
            }
            (AdmissionBody::Configured, ..) => Accepted,
            (body, ..) => Rejected(not_configured(body)),
        },

        I::SyncUnsynced => match configured_and_idle(snapshot, None) {
            Accepted if snapshot.sync_running => Coalesced(CoalescibleIntent::SyncUnsynced),
            admission => admission,
        },
        I::StartVerification { .. } => start_verification(snapshot),
        I::RepairPasskey { .. } => configured_and_idle(snapshot, Some(Op::RepairPasskey)),
        // the batch is frozen to the undecryptable records the inventory lists, so it needs them
        I::DeleteUndecryptableWalletBackups(_) => {
            needs_inventory(snapshot, Op::DeleteUndecryptableWalletBackups, DetailInventory::Ready)
        }
        I::RecreateManifest => {
            needs_inventory(snapshot, Op::RecreateManifest, DetailInventory::Complete)
        }
        I::ReinitializeBackup => {
            needs_inventory(snapshot, Op::ReinitializeBackup, DetailInventory::Complete)
        }
        I::RestoreCloudWallet(_) => restore_cloud_wallet(snapshot),
        I::DeleteCloudWallet(_) => {
            needs_inventory(snapshot, Op::DeleteCloudWallet, DetailInventory::Ready)
        }
        I::RecoverOtherBackups => needs_other_backups(snapshot, Op::RecoverOtherBackups),
        I::DeleteOtherBackups => needs_other_backups(snapshot, Op::DeleteOtherBackups),
        I::StartRestoreAll => start_restore_all(snapshot),
        I::RetryRestoreAllRemaining => {
            needs_restore_all(snapshot, RestoreAllAdmission::RetryAvailable)
        }
    }
}

/// A detail verification, which a Turn Off review's Verify First also starts
///
/// A verification is an operation, so a repeat while one runs joins it
fn start_verification(snapshot: &AdmissionSnapshot) -> Admission {
    configured_and_idle(snapshot, Some(CloudBackupExclusiveOperation::Verify))
}

/// A restore of one wallet that is only in the cloud, which Restore Them First also starts
fn restore_cloud_wallet(snapshot: &AdmissionSnapshot) -> Admission {
    needs_inventory(
        snapshot,
        CloudBackupExclusiveOperation::RestoreCloudWallet,
        DetailInventory::Ready,
    )
}

/// Restore All, which Restore Them First also starts
fn start_restore_all(snapshot: &AdmissionSnapshot) -> Admission {
    needs_restore_all(snapshot, RestoreAllAdmission::StartAvailable)
}

/// Enable and restore start from a device without a configured backup
fn starts_while_off(
    snapshot: &AdmissionSnapshot,
    operation: CloudBackupExclusiveOperation,
) -> Admission {
    match (snapshot.body, snapshot.running) {
        (_, Some(running)) if running == operation => {
            Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(running))
        }
        (_, Some(running)) => Admission::Rejected(Rejection::OperationRunning(running)),
        (
            AdmissionBody::Disabled
            | AdmissionBody::Enabling
            | AdmissionBody::Restoring
            | AdmissionBody::PendingEnableRecovery,
            None,
        ) => Admission::Accepted,
        (AdmissionBody::Configured, None) => Admission::Rejected(Rejection::AlreadyOn),
        (AdmissionBody::DriveAccountSwitch, None) => {
            Admission::Rejected(Rejection::DriveAccountSwitch)
        }
        (AdmissionBody::FinishingDisable, None) => Admission::Rejected(Rejection::TurnOffCommitted),
        (AdmissionBody::LoadFailed, None) => Admission::Rejected(Rejection::LoadFailed),
        (AdmissionBody::Corrupted, None) => Admission::Rejected(Rejection::Corrupted),
        (AdmissionBody::Resetting, None) => Admission::Rejected(Rejection::Resetting),
    }
}

/// Work on a configured backup that is not in the middle of anything else
///
/// `operation` is the exclusive operation the intent starts, if it starts one; repeating it
/// while it runs coalesces
fn configured_and_idle(
    snapshot: &AdmissionSnapshot,
    operation: Option<CloudBackupExclusiveOperation>,
) -> Admission {
    match (snapshot.body, snapshot.running) {
        (AdmissionBody::Configured, Some(running)) if Some(running) == operation => {
            Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(running))
        }
        (AdmissionBody::Configured, Some(running)) => {
            Admission::Rejected(Rejection::OperationRunning(running))
        }
        (AdmissionBody::Configured, None) => Admission::Accepted,
        (body, _) => Admission::Rejected(not_configured(body)),
    }
}

fn needs_inventory(
    snapshot: &AdmissionSnapshot,
    operation: CloudBackupExclusiveOperation,
    needed: DetailInventory,
) -> Admission {
    let loaded = match (needed, snapshot.detail_inventory) {
        (_, DetailInventory::Complete) => true,
        (DetailInventory::Ready, DetailInventory::Ready) => true,
        (DetailInventory::NotLoaded | DetailInventory::Ready | DetailInventory::Complete, _) => {
            false
        }
    };

    match configured_and_idle(snapshot, Some(operation)) {
        Admission::Accepted if !loaded => Admission::Rejected(Rejection::DetailInventoryNotLoaded),
        admission => admission,
    }
}

fn needs_other_backups(
    snapshot: &AdmissionSnapshot,
    operation: CloudBackupExclusiveOperation,
) -> Admission {
    match configured_and_idle(snapshot, Some(operation)) {
        Admission::Accepted if !snapshot.other_backups_loaded => {
            Admission::Rejected(Rejection::OtherBackupsNotLoaded)
        }
        admission => admission,
    }
}

fn needs_restore_all(snapshot: &AdmissionSnapshot, needed: RestoreAllAdmission) -> Admission {
    match configured_and_idle(snapshot, Some(CloudBackupExclusiveOperation::RestoreAllCloudWallets))
    {
        Admission::Accepted if snapshot.restore_all != needed => {
            Admission::Rejected(Rejection::RestoreAllUnavailable)
        }
        admission => admission,
    }
}

fn not_configured(body: AdmissionBody) -> Rejection {
    match body {
        AdmissionBody::Disabled => Rejection::CloudBackupOff,
        AdmissionBody::LoadFailed => Rejection::LoadFailed,
        AdmissionBody::Corrupted => Rejection::Corrupted,
        AdmissionBody::FinishingDisable => Rejection::TurnOffCommitted,
        AdmissionBody::Resetting => Rejection::Resetting,
        AdmissionBody::DriveAccountSwitch => Rejection::DriveAccountSwitch,
        AdmissionBody::Enabling
        | AdmissionBody::Restoring
        | AdmissionBody::PendingEnableRecovery => Rejection::SetupNotFinished,

        // every caller decides a configured body itself; if one ever lands here, the backup is on
        AdmissionBody::Configured => Rejection::AlreadyOn,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Admission, AdmissionBody, AdmissionSnapshot, DetailInventory, EnableIntent,
        EnablePromptAnswers, Rejection, RestoreAllAdmission, SwitchAdmission, TurnOffAdmission,
        UserIntent, evaluate_admission,
    };
    use crate::manager::cloud_backup_manager::CloudBackupEnableContext;

    /// A snapshot of `body` with nothing running, paused or loaded
    fn idle(body: AdmissionBody) -> AdmissionSnapshot {
        AdmissionSnapshot {
            body,
            running: None,
            sync_running: false,
            detail_inventory: DetailInventory::NotLoaded,
            detail_checking: false,
            other_backups_loaded: false,
            restore_all: RestoreAllAdmission::Unavailable,
            continuation: None,
            enable_prompt: EnablePromptAnswers { use_existing: None, create_new: None },
            turn_off: TurnOffAdmission::None,
            drive_account_switch: SwitchAdmission::None,
        }
    }

    // each case carries its own reason, so both apps can word each one truthfully
    #[test]
    fn already_on_and_setup_not_finished_are_separate_reasons() {
        let configured = idle(AdmissionBody::Configured);
        let enable =
            UserIntent::Enable(EnableIntent::Standard, CloudBackupEnableContext::settings_manual());
        let already_on = Admission::Rejected(Rejection::AlreadyOn);
        assert_eq!(evaluate_admission(&enable, &configured), already_on);
        assert_eq!(
            evaluate_admission(&UserIntent::RestoreFromCloudBackup, &configured),
            already_on
        );

        let setup_not_finished = Admission::Rejected(Rejection::SetupNotFinished);
        for body in [
            AdmissionBody::Enabling,
            AdmissionBody::Restoring,
            AdmissionBody::PendingEnableRecovery,
        ] {
            let snapshot = idle(body);
            for intent in
                [UserIntent::TurnOff, UserIntent::SyncUnsynced, UserIntent::RecreateManifest]
            {
                assert_eq!(
                    evaluate_admission(&intent, &snapshot),
                    setup_not_finished,
                    "{intent:?} in {body:?}"
                );
            }
        }
    }
}
