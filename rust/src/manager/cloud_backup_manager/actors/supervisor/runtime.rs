//! The supervisor's stored runtime: the lifecycle body with the running operation nested in it
//!
//! There is no operation slot beside the lifecycle. Each body names the operations that can run
//! in it, so "Turn Off while backup is off" and "verify during a committed disable" have no
//! value: `DisabledOp` has no Turn Off and `FinishingDisable` holds only `ResumeDisable`
//!
//! The body moves in three ways only:
//! - at load: `LifecycleLoad::Failed` gives `LoadFailed`, which nothing leaves; retry is a relaunch
//! - while idle: the supervisor rebases the body on its own committed record when it starts work
//! - while an operation runs, the operation holds its body until it ends or hands off

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::debug;

use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupTransition};
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::admission::{AdmissionBody, ConfiguredAdmission};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDisableOutcome, CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
    CloudBackupStatus,
};
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
use crate::manager::cloud_backup_manager::owner::lifecycle::{CommittedDisable, DisablingRecord};

/// The lifecycle body and the operation running in it
#[derive(Debug)]
pub(crate) enum Runtime {
    Disabled {
        op: Option<DisabledOp>,
    },
    Configured {
        op: Option<ConfiguredOp>,
    },
    FinishingDisable {
        committed: CommittedDisable,
        op: Option<ResumeDisable>,
    },
    DriveAccountSwitch {
        op: Option<SwitchOp>,
    },
    PendingEnableRecovery {
        op: Option<RecoveryOp>,
    },
    /// Load normalization failed; the legacy record is unchanged and nothing can start
    LoadFailed {
        error: DatabaseError,
    },
    /// The stored record can't be read as a backup; Enable or Restore replaces it, as from off
    Corrupted {
        error: String,
        op: Option<DisabledOp>,
    },
}

/// Work that starts from a device without a configured backup
#[derive(Debug)]
pub(crate) enum DisabledOp {
    Enable(OpRun),
    Restore(RestoreRun),
    /// Discarding an Enable that stopped at a prompt, before any recovery was projected
    DiscardEnable(OpRun),
}

/// Work on a configured backup
#[derive(Debug)]
pub(crate) enum ConfiguredOp {
    TurnOff(OpRun),
    Verify(OpRun),
    RepairPasskey(OpRun),
    RestoreCloudWallet(OpRun),
    RestoreAll(RestoreRun),
    DeleteRecords(OpRun),
    RecoverOtherBackups(OpRun),
    DeleteOtherBackups(OpRun),
    /// A reinitialize, including one a Google Drive account switch started
    Reinitialize(OpRun),
    RecreateManifest(OpRun),
}

/// A committed Turn Off going forward; nothing else runs during one
#[derive(Debug)]
pub(crate) struct ResumeDisable(OpRun);

/// The reinitialization of a Google Drive account switch that is still recorded
#[derive(Debug)]
pub(crate) struct SwitchOp(OpRun);

/// Work on an interrupted Enable
#[derive(Debug)]
pub(crate) enum RecoveryOp {
    /// Enable picks the interrupted session up again
    Continue(OpRun),
    /// Discard deletes what the interrupted Enable wrote; its remote delete runs off the mailbox
    Discard(OpRun),
    Restore(RestoreRun),
}

/// A running operation, identified by its claim, which carries its `OperationId`
///
/// Every nested op holds one, so an operation's stage goes here: B2c adds the stage field with
/// `Running` and `Effecting`, for an operation waiting on an off-actor effect it can't cancel,
/// without reshaping `Runtime` or the per-body op enums
#[derive(Debug)]
pub(crate) struct OpRun {
    claim: CloudBackupExclusiveOperationClaim,
}

/// A restore or Restore All run, whose worker keeps draining until it observes cancellation
#[derive(Debug)]
pub(crate) struct RestoreRun {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) cancellation: Arc<AtomicBool>,
    /// Set once the run commits a local wallet, so cloud-only rows are reclassified when it ends
    pub(crate) committed_local_wallet: bool,
}

impl RestoreRun {
    pub(crate) fn new(
        claim: CloudBackupExclusiveOperationClaim,
        cancellation: Arc<AtomicBool>,
    ) -> Self {
        Self { claim, cancellation, committed_local_wallet: false }
    }
}

impl OpRun {
    fn new(claim: CloudBackupExclusiveOperationClaim) -> Self {
        Self { claim }
    }
}

/// An operation the current body has no place for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unstartable {
    pub(crate) body: AdmissionBody,
    pub(crate) operation: CloudBackupExclusiveOperation,
}

/// The owner's facts an idle body is rebased on
pub(crate) struct IdleFacts<'a> {
    /// The record as the supervisor's writer last committed it
    pub(crate) record: &'a PersistedCloudBackupState,
    /// The supervisor projected an interrupted Enable it found at restart
    pub(crate) pending_enable_recovery: bool,
}

/// The operation a start places, before the body decides where it goes
enum NewOp {
    Standard(OpRun),
    Restore(RestoreRun),
}

impl NewOp {
    fn claim(&self) -> CloudBackupExclusiveOperationClaim {
        match self {
            Self::Standard(run) => run.claim,
            Self::Restore(run) => run.claim,
        }
    }
}

impl Runtime {
    /// The body at bootstrap, before any operation
    pub(crate) fn at_load(load_error: Option<DatabaseError>, facts: IdleFacts<'_>) -> Self {
        match load_error {
            Some(error) => Self::LoadFailed { error },
            None => Self::idle(facts),
        }
    }

    fn idle(facts: IdleFacts<'_>) -> Self {
        use PersistedCloudBackupState as State;

        match facts.record {
            State::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(_)) => {
                match DisablingRecord::of(facts.record) {
                    Some(DisablingRecord::Committed(committed)) => {
                        Self::FinishingDisable { committed, op: None }
                    }
                    // only a failed load heal leaves this record, and `LoadFailed` never rebases
                    Some(DisablingRecord::Unnormalized) | None => Self::LoadFailed {
                        error: DatabaseError::DatabaseAccess(
                            "Cloud Backup lifecycle record was not normalized".into(),
                        ),
                    },
                }
            }
            State::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(_)) => {
                Self::DriveAccountSwitch { op: None }
            }
            State::Corrupted { error } => Self::Corrupted { error: error.clone(), op: None },
            State::Disabled | State::Configured(_) if facts.pending_enable_recovery => {
                Self::PendingEnableRecovery { op: None }
            }
            State::Configured(_) => Self::Configured { op: None },
            State::Disabled => Self::Disabled { op: None },
        }
    }

    /// Moves an idle body onto the owner's current facts; a running operation holds its body
    pub(crate) fn rebase_idle(&mut self, facts: IdleFacts<'_>) {
        if self.claim().is_some() || matches!(self, Self::LoadFailed { .. }) {
            return;
        }

        *self = Self::idle(facts);
    }

    /// The body admission decides on, with the running operation's kind
    pub(crate) fn admission_body(&self, configured: ConfiguredAdmission) -> AdmissionBody {
        match self {
            Self::Disabled { op: None | Some(DisabledOp::DiscardEnable(_)) }
            | Self::Corrupted { op: None | Some(DisabledOp::DiscardEnable(_)), .. } => {
                AdmissionBody::Disabled
            }
            Self::Disabled { op: Some(DisabledOp::Enable(_)) }
            | Self::Corrupted { op: Some(DisabledOp::Enable(_)), .. } => AdmissionBody::Enabling,
            Self::Disabled { op: Some(DisabledOp::Restore(_)) }
            | Self::Corrupted { op: Some(DisabledOp::Restore(_)), .. } => AdmissionBody::Restoring,
            Self::Configured { .. } | Self::DriveAccountSwitch { .. } => {
                AdmissionBody::Configured(configured)
            }
            Self::FinishingDisable { .. } => AdmissionBody::FinishingDisable,
            Self::PendingEnableRecovery { .. } => AdmissionBody::PendingEnableRecovery,
            Self::LoadFailed { .. } => AdmissionBody::LoadFailed,
        }
    }

    pub(crate) fn is_some(&self) -> bool {
        self.claim().is_some()
    }

    pub(crate) fn is_restore_all(&self) -> bool {
        matches!(self, Self::Configured { op: Some(ConfiguredOp::RestoreAll(_)) })
    }

    /// The claim of the running operation, which carries its `OperationId`
    pub(crate) fn claim(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        match self {
            Self::Disabled { op } | Self::Corrupted { op, .. } => {
                op.as_ref().map(DisabledOp::claim)
            }
            Self::Configured { op } => op.as_ref().map(ConfiguredOp::claim),
            Self::FinishingDisable { op, .. } => op.as_ref().map(|op| op.0.claim),
            Self::DriveAccountSwitch { op } => op.as_ref().map(|op| op.0.claim),
            Self::PendingEnableRecovery { op } => op.as_ref().map(RecoveryOp::claim),
            Self::LoadFailed { .. } => None,
        }
    }

    /// Whether `claim` still owns the running operation, so completions for superseded claims are
    /// ignored
    pub(crate) fn is_current(&self, claim: CloudBackupExclusiveOperationClaim) -> bool {
        self.claim() == Some(claim)
    }

    /// Whether a completion carrying `claim` belongs to the running operation
    ///
    /// A completion for an operation that already finished, was cancelled or was handed off is
    /// stale. It is discarded here with a `debug!` log and is never reported to the user, since
    /// the user's intent was admitted and the operation that replaced it reports its own result
    pub(crate) fn accepts_completion(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: &'static str,
    ) -> bool {
        let current = self.claim();
        if current == Some(claim) {
            return true;
        }

        debug!("Cloud Backup discarded stale {completion} for {claim:?}, running {current:?}");
        false
    }

    pub(crate) fn start_standard(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), Unstartable> {
        self.place(NewOp::Standard(OpRun::new(claim)))
    }

    pub(crate) fn start_restore(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.place(NewOp::Restore(run))
    }

    pub(crate) fn start_restore_all(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.place(NewOp::Restore(run))
    }

    /// Puts a new operation in the body that has a place for its kind
    ///
    /// This is the constructor that makes a misplaced operation unrepresentable: each body's op
    /// type lists what it can hold, so the kinds it doesn't list have nowhere to go
    #[deny(clippy::wildcard_enum_match_arm)]
    fn place(&mut self, new: NewOp) -> Result<(), Unstartable> {
        use CloudBackupExclusiveOperation as Op;

        let operation = new.claim().operation();
        let unstartable = |runtime: &Self| Unstartable {
            body: runtime.admission_body(ConfiguredAdmission::Ready),
            operation,
        };
        if self.is_some() {
            return Err(unstartable(self));
        }

        match (&mut *self, new) {
            (Self::Disabled { op } | Self::Corrupted { op, .. }, NewOp::Standard(run))
                if matches!(operation, Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery) =>
            {
                *op = Some(DisabledOp::Enable(run));
            }
            (Self::Disabled { op } | Self::Corrupted { op, .. }, NewOp::Standard(run))
                if operation == Op::DiscardPendingEnable =>
            {
                *op = Some(DisabledOp::DiscardEnable(run));
            }
            (Self::Disabled { op } | Self::Corrupted { op, .. }, NewOp::Restore(run))
                if operation == Op::Restore =>
            {
                *op = Some(DisabledOp::Restore(run));
            }
            (Self::Configured { op }, NewOp::Restore(run))
                if operation == Op::RestoreAllCloudWallets =>
            {
                *op = Some(ConfiguredOp::RestoreAll(run));
            }
            (Self::Configured { op }, NewOp::Standard(run)) => {
                let configured = match operation {
                    Op::Disable => ConfiguredOp::TurnOff(run),
                    Op::VerifyReinitializedBackup | Op::VerificationRepair => {
                        ConfiguredOp::Verify(run)
                    }
                    Op::RepairPasskey => ConfiguredOp::RepairPasskey(run),
                    Op::RestoreCloudWallet => ConfiguredOp::RestoreCloudWallet(run),
                    Op::DeleteCloudWallet | Op::DeleteUndecryptableWalletBackups => {
                        ConfiguredOp::DeleteRecords(run)
                    }
                    Op::RecoverOtherBackups => ConfiguredOp::RecoverOtherBackups(run),
                    Op::DeleteOtherBackups => ConfiguredOp::DeleteOtherBackups(run),
                    Op::ReinitializeBackup => ConfiguredOp::Reinitialize(run),
                    Op::RecreateManifest => ConfiguredOp::RecreateManifest(run),
                    Op::Enable
                    | Op::EnableForceNew
                    | Op::EnableNoDiscovery
                    | Op::Restore
                    | Op::RestoreAllCloudWallets
                    | Op::DiscardPendingEnable => return Err(unstartable(self)),
                };
                *op = Some(configured);
            }
            (Self::FinishingDisable { op, .. }, NewOp::Standard(run))
                if operation == Op::Disable =>
            {
                *op = Some(ResumeDisable(run));
            }
            (Self::DriveAccountSwitch { op }, NewOp::Standard(run))
                if operation == Op::ReinitializeBackup =>
            {
                *op = Some(SwitchOp(run));
            }
            (Self::PendingEnableRecovery { op }, NewOp::Standard(run)) => {
                *op = Some(match operation {
                    Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery => {
                        RecoveryOp::Continue(run)
                    }
                    Op::DiscardPendingEnable => RecoveryOp::Discard(run),
                    Op::Restore
                    | Op::Disable
                    | Op::RecreateManifest
                    | Op::ReinitializeBackup
                    | Op::VerifyReinitializedBackup
                    | Op::RepairPasskey
                    | Op::VerificationRepair
                    | Op::RecoverOtherBackups
                    | Op::DeleteOtherBackups
                    | Op::RestoreCloudWallet
                    | Op::RestoreAllCloudWallets
                    | Op::DeleteCloudWallet
                    | Op::DeleteUndecryptableWalletBackups => return Err(unstartable(self)),
                });
            }
            (Self::PendingEnableRecovery { op }, NewOp::Restore(run))
                if operation == Op::Restore =>
            {
                *op = Some(RecoveryOp::Restore(run));
            }
            (
                Self::Disabled { .. }
                | Self::Corrupted { .. }
                | Self::Configured { .. }
                | Self::FinishingDisable { .. }
                | Self::DriveAccountSwitch { .. }
                | Self::PendingEnableRecovery { .. }
                | Self::LoadFailed { .. },
                NewOp::Standard(_) | NewOp::Restore(_),
            ) => return Err(unstartable(self)),
        }

        Ok(())
    }

    /// The Turn Off with `claim` committed: its body becomes the committed disable, and the same
    /// operation keeps running there as the resume, so nothing configured can start beside it
    pub(crate) fn commit_turn_off(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        committed: CommittedDisable,
    ) {
        let Self::Configured { op } = self else { return };
        if !matches!(op, Some(ConfiguredOp::TurnOff(run)) if run.claim == claim) {
            return;
        }

        let Some(ConfiguredOp::TurnOff(run)) = op.take() else { return };
        *self = Self::FinishingDisable { committed, op: Some(ResumeDisable(run)) };
    }

    /// Moves the running operation from `from` to `to` without ever leaving the slot free
    ///
    /// The old operation ends and the new one starts in one step, so the body is rebased on the
    /// owner's facts in between, as for any start. If the new kind has no place, the old
    /// operation keeps running
    pub(crate) fn hand_off(
        &mut self,
        from: CloudBackupExclusiveOperationClaim,
        to: CloudBackupExclusiveOperationClaim,
        facts: IdleFacts<'_>,
    ) -> Result<(), Unstartable> {
        debug_assert!(self.is_current(from));
        let held = std::mem::replace(self, Self::Disabled { op: None });
        *self = Self::idle(facts);
        if let Err(unstartable) = self.start_standard(to) {
            *self = held;
            return Err(unstartable);
        }

        Ok(())
    }

    pub(crate) fn restore(&self, claim: CloudBackupExclusiveOperationClaim) -> Option<&RestoreRun> {
        match self {
            Self::Disabled { op: Some(DisabledOp::Restore(run)) }
            | Self::Corrupted { op: Some(DisabledOp::Restore(run)), .. }
            | Self::PendingEnableRecovery { op: Some(RecoveryOp::Restore(run)) }
                if run.claim == claim =>
            {
                Some(run)
            }
            _ => None,
        }
    }

    pub(crate) fn restore_all(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Option<&RestoreRun> {
        match self {
            Self::Configured { op: Some(ConfiguredOp::RestoreAll(run)) } if run.claim == claim => {
                Some(run)
            }
            _ => None,
        }
    }

    pub(crate) fn restore_all_mut(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Option<&mut RestoreRun> {
        match self {
            Self::Configured { op: Some(ConfiguredOp::RestoreAll(run)) } if run.claim == claim => {
                Some(run)
            }
            _ => None,
        }
    }

    /// Cancels draining restore work but releases operations with no retained cancellation owner
    pub(crate) fn prepare_local_reset(&mut self) -> Option<CloudBackupExclusiveOperationClaim> {
        let restore = match self {
            Self::Disabled { op: Some(DisabledOp::Restore(run)) }
            | Self::Corrupted { op: Some(DisabledOp::Restore(run)), .. }
            | Self::PendingEnableRecovery { op: Some(RecoveryOp::Restore(run)) }
            | Self::Configured { op: Some(ConfiguredOp::RestoreAll(run)) } => Some(run),
            _ => None,
        };
        if let Some(run) = restore {
            run.cancellation.store(true, Ordering::Release);
            return None;
        }

        let claim = self.claim()?;
        self.clear();
        Some(claim)
    }

    /// Ends the running operation; the body stays until the next start rebases it
    pub(crate) fn clear(&mut self) {
        match self {
            Self::Disabled { op } | Self::Corrupted { op, .. } => *op = None,
            Self::Configured { op } => *op = None,
            Self::FinishingDisable { op, .. } => *op = None,
            Self::DriveAccountSwitch { op } => *op = None,
            Self::PendingEnableRecovery { op } => *op = None,
            Self::LoadFailed { .. } => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn take_claim(&mut self) -> Option<CloudBackupExclusiveOperationClaim> {
        let claim = self.claim();
        self.clear();
        claim
    }
}

/// What the runtime shows through today's exported types
///
/// Read-only: it never feeds back into `Runtime`. `None` fields leave the projection to the
/// reducer, which still owns presentation detail for the bodies PR 2 reshapes
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportedRuntime {
    pub(crate) status: Option<CloudBackupStatus>,
    /// A committed Turn Off that failed, shown as today's `DisableFailed`
    pub(crate) disable: Option<CloudBackupDisableOutcome>,
}

impl Runtime {
    /// The adapter from the stored runtime onto today's exported types
    ///
    /// PR 1 changes no exported shape, so each body that has no exported variant of its own
    /// shows as the nearest one today's platforms already render
    pub(crate) fn exported(&self) -> ExportedRuntime {
        let (status, disable) = match self {
            Self::LoadFailed { error } => {
                debug!("Cloud Backup runtime is LoadFailed: {error}");
                (Some(CloudBackupStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into())), None)
            }
            // a committed failure only goes forward, so Keep Enabled is never offered for it
            Self::FinishingDisable {
                committed: CommittedDisable { last_error: Some(message), .. },
                op: None,
            } => (
                None,
                Some(CloudBackupDisableOutcome::Failed {
                    message: message.clone(),
                    can_keep_enabled: false,
                }),
            ),
            Self::FinishingDisable { op: Some(_), .. } => {
                (Some(CloudBackupStatus::Disabling), None)
            }
            // the record alone gives these statuses, and the record projection is their one writer;
            // repeating them here would let a late projection overwrite a status set after it
            Self::Corrupted { error, .. } => {
                debug!("Cloud Backup runtime is Corrupted: {error}");
                (None, None)
            }
            Self::Disabled { .. }
            | Self::FinishingDisable {
                committed: CommittedDisable { last_error: None, .. },
                op: None,
            }
            | Self::Configured { .. }
            | Self::DriveAccountSwitch { .. }
            | Self::PendingEnableRecovery { .. } => (None, None),
        };

        ExportedRuntime { status, disable }
    }
}

impl PartialEq<Option<CloudBackupExclusiveOperationClaim>> for Runtime {
    fn eq(&self, other: &Option<CloudBackupExclusiveOperationClaim>) -> bool {
        self.claim() == *other
    }
}

impl DisabledOp {
    fn claim(&self) -> CloudBackupExclusiveOperationClaim {
        match self {
            Self::Enable(run) | Self::DiscardEnable(run) => run.claim,
            Self::Restore(run) => run.claim,
        }
    }
}

impl ConfiguredOp {
    fn claim(&self) -> CloudBackupExclusiveOperationClaim {
        match self {
            Self::TurnOff(run)
            | Self::Verify(run)
            | Self::RepairPasskey(run)
            | Self::RestoreCloudWallet(run)
            | Self::DeleteRecords(run)
            | Self::RecoverOtherBackups(run)
            | Self::DeleteOtherBackups(run)
            | Self::Reinitialize(run)
            | Self::RecreateManifest(run) => run.claim,
            Self::RestoreAll(run) => run.claim,
        }
    }
}

impl RecoveryOp {
    fn claim(&self) -> CloudBackupExclusiveOperationClaim {
        match self {
            Self::Continue(run) | Self::Discard(run) => run.claim,
            Self::Restore(run) => run.claim,
        }
    }
}
