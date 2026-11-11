//! Admission: the one decision about what Cloud Backup does with a user intent
//!
//! Every [`CloudBackupManagerAction`] becomes a [`UserIntent`] through an exhaustive `match`, so
//! a new action does not compile until it is mapped here. [`evaluate_admission`] decides what
//! happens to an intent from an [`AdmissionSnapshot`], and is pure: the UI-side checks and
//! `CloudBackupSupervisor::dispatch` call the same function, so a projected action and a
//! dispatched one cannot disagree

use super::{
    CloudBackupEnableContext, CloudBackupEnablePromptChoice, CloudBackupExclusiveOperation,
    CloudBackupManagerAction, CloudBackupVerificationSource, RecordId,
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
    StartVerification { source: CloudBackupVerificationSource, discoverable: bool },
    DismissVerificationPrompt,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey { no_discovery: bool },
    SyncUnsynced,
    RestoreCloudWallet(RecordId),
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    DeleteCloudWallet(RecordId),
    DeleteUndecryptableWalletBackups,
    RecoverOtherBackups,
    DeleteOtherBackups,
    TurnOff,
    KeepEnabled,
    RefreshDetail,
    EnterDetail,
    PromptEnablePasskeyChoice(CloudBackupEnableContext),
    AcceptEnablePrompt(CloudBackupEnablePromptChoice),
}

/// Which Enable the user asked for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnableIntent {
    Standard,
    ForceNew,
    NoDiscovery,
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
            A::DeleteCloudWallet(record_id) => Self::DeleteCloudWallet(record_id),
            A::DeleteUndecryptableWalletBackups => Self::DeleteUndecryptableWalletBackups,
            A::RecoverOtherBackups => Self::RecoverOtherBackups,
            A::DeleteOtherBackups => Self::DeleteOtherBackups,
            A::DisableCloudBackup => Self::TurnOff,
            A::KeepCloudBackupEnabled => Self::KeepEnabled,
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
    /// A detail refresh is already loading, and the inventory shows it loading
    #[expect(dead_code, reason = "PR 2's inventory owner reports a refresh in flight")]
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
    NotConfigured,
    /// The stored record could not be loaded; a relaunch retries
    LoadFailed,
    /// Turn Off already deleted remote data, so it only goes forward
    TurnOffCommitted,
    /// The last Turn Off failed and its result is waiting for the user
    TurnOffFailed,
    OperationRunning(CloudBackupExclusiveOperation),
    DetailInventoryNotLoaded,
    OtherBackupsNotLoaded,
    RestoreAllUnavailable,
    NothingToCancel,
    KeepEnabledUnavailable,
}

/// The lifecycle body admission decides on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionBody {
    Disabled,
    Enabling,
    Restoring,
    Configured(ConfiguredAdmission),
    /// A committed Turn Off that still has to finish
    FinishingDisable,
    PendingEnableRecovery,
    LoadFailed,
}

/// What admission needs to know about a configured backup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfiguredAdmission {
    Ready,
    /// The last Turn Off failed before it deleted anything; Keep Enabled is offered when allowed
    TurnOffFailed {
        can_keep_enabled: bool,
    },
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

/// Everything admission reads, taken in one place at one time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdmissionSnapshot {
    pub(crate) body: AdmissionBody,
    pub(crate) running: Option<CloudBackupExclusiveOperation>,
    pub(crate) sync_running: bool,
    pub(crate) detail_inventory: DetailInventory,
    pub(crate) other_backups_loaded: bool,
    pub(crate) restore_all: RestoreAllAdmission,
}

/// Decides what happens to `intent` in `snapshot`
///
/// Pure: it does not log or allocate, so projections can call it as often as they like
pub(crate) fn evaluate_admission(intent: &UserIntent, snapshot: &AdmissionSnapshot) -> Admission {
    use Admission::{Accepted, Coalesced, Rejected};
    use CloudBackupExclusiveOperation as Op;
    use UserIntent as I;

    match intent {
        // presentation intents answer prompts; their handlers own the prompt they answer
        I::ConfirmSavedPasskey
        | I::ConfirmPendingEnableCleanup
        | I::DismissPasskeyChoicePrompt
        | I::DismissMissingPasskeyReminder
        | I::DismissVerificationPrompt
        | I::PromptEnablePasskeyChoice(_)
        | I::AcceptEnablePrompt(_)
        | I::EnterDetail
        | I::RefreshDetail => Accepted,

        I::Enable(kind, _) => {
            let operation = match kind {
                EnableIntent::Standard => Op::Enable,
                EnableIntent::ForceNew => Op::EnableForceNew,
                EnableIntent::NoDiscovery => Op::EnableNoDiscovery,
            };
            starts_while_off(snapshot, operation)
        }
        I::RestoreFromCloudBackup => starts_while_off(snapshot, Op::Restore),

        // a Discard while one runs is the same request; any other running work is checked by
        // the discard's own start, which needs the operation slot
        I::DiscardPendingEnable => match snapshot.running {
            Some(Op::DiscardPendingEnable) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::DiscardPendingEnable))
            }
            _ => Accepted,
        },

        I::CancelRestore => match snapshot.running {
            Some(Op::Restore) => Accepted,
            _ => Rejected(Rejection::NothingToCancel),
        },
        I::CancelRestoreAll => match snapshot.restore_all {
            RestoreAllAdmission::Running => Accepted,
            _ => Rejected(Rejection::NothingToCancel),
        },

        I::TurnOff => match (snapshot.body, snapshot.running) {
            (AdmissionBody::Configured(_) | AdmissionBody::FinishingDisable, None) => Accepted,
            (AdmissionBody::Configured(_) | AdmissionBody::FinishingDisable, Some(Op::Disable)) => {
                Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Disable))
            }
            (AdmissionBody::Configured(_) | AdmissionBody::FinishingDisable, Some(running)) => {
                Rejected(Rejection::OperationRunning(running))
            }
            (body, _) => Rejected(not_configured(body)),
        },
        I::KeepEnabled => match (snapshot.body, snapshot.running) {
            (
                AdmissionBody::Configured(ConfiguredAdmission::TurnOffFailed {
                    can_keep_enabled: true,
                }),
                None,
            ) => Accepted,
            (AdmissionBody::Configured(_) | AdmissionBody::FinishingDisable, Some(running)) => {
                Rejected(Rejection::OperationRunning(running))
            }
            _ => Rejected(Rejection::KeepEnabledUnavailable),
        },

        I::SyncUnsynced => match configured_and_idle(snapshot, None) {
            Accepted if snapshot.sync_running => Coalesced(CoalescibleIntent::SyncUnsynced),
            admission => admission,
        },
        I::StartVerification { .. } => configured_and_idle(snapshot, None),
        I::RepairPasskey { .. } => configured_and_idle(snapshot, Some(Op::RepairPasskey)),
        I::DeleteUndecryptableWalletBackups => {
            configured_and_idle(snapshot, Some(Op::DeleteUndecryptableWalletBackups))
        }
        I::RecreateManifest => {
            needs_inventory(snapshot, Op::RecreateManifest, DetailInventory::Complete)
        }
        I::ReinitializeBackup => {
            needs_inventory(snapshot, Op::ReinitializeBackup, DetailInventory::Complete)
        }
        I::RestoreCloudWallet(_) => {
            needs_inventory(snapshot, Op::RestoreCloudWallet, DetailInventory::Ready)
        }
        I::DeleteCloudWallet(_) => {
            needs_inventory(snapshot, Op::DeleteCloudWallet, DetailInventory::Ready)
        }
        I::RecoverOtherBackups => needs_other_backups(snapshot, Op::RecoverOtherBackups),
        I::DeleteOtherBackups => needs_other_backups(snapshot, Op::DeleteOtherBackups),
        I::StartRestoreAll => needs_restore_all(snapshot, RestoreAllAdmission::StartAvailable),
        I::RetryRestoreAllRemaining => {
            needs_restore_all(snapshot, RestoreAllAdmission::RetryAvailable)
        }
    }
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
        (AdmissionBody::Configured(_), None) => Admission::Rejected(Rejection::NotConfigured),
        (AdmissionBody::FinishingDisable, None) => Admission::Rejected(Rejection::TurnOffCommitted),
        (AdmissionBody::LoadFailed, None) => Admission::Rejected(Rejection::LoadFailed),
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
        (AdmissionBody::Configured(_), Some(running)) if Some(running) == operation => {
            Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(running))
        }
        (AdmissionBody::Configured(_), Some(running)) => {
            Admission::Rejected(Rejection::OperationRunning(running))
        }
        (AdmissionBody::Configured(ConfiguredAdmission::Ready), None) => Admission::Accepted,
        (AdmissionBody::Configured(ConfiguredAdmission::TurnOffFailed { .. }), None) => {
            Admission::Rejected(Rejection::TurnOffFailed)
        }
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
        AdmissionBody::FinishingDisable => Rejection::TurnOffCommitted,
        AdmissionBody::Configured(_)
        | AdmissionBody::Enabling
        | AdmissionBody::Restoring
        | AdmissionBody::PendingEnableRecovery => Rejection::NotConfigured,
    }
}
