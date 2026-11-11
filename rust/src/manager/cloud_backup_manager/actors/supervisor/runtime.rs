//! The supervisor's stored runtime: the lifecycle body with the running operation nested in it
//!
//! There is no operation slot beside the lifecycle. Each body names the operations that can run
//! in it, so "Turn Off while backup is off" and "verify during a committed disable" have no
//! value: `DisabledOp` has no Turn Off and `FinishingDisable` holds only `ResumeDisable`. A
//! configured body is either idle, where the Turn Off review lives, or running one operation, so
//! a review beside a running operation has no value either
//!
//! The body moves in three ways only:
//! - at load: `LifecycleLoad::Failed` gives `LoadFailed`, which only a load that succeeds leaves,
//!   at launch or when the user retries it. An idle body with no record the owner ever read
//!   becomes `LoadFailed` the same way
//! - while idle: the supervisor rebases the body on its own committed record when it starts work
//! - while an operation runs, the operation holds its body until it ends or hands off
//! - a local reset replaces the body with `Resetting`, which nothing can start in, and the owner
//!   sets the body the wipe left when it finishes. `Resetting` records whether the reset cleared
//!   the local keys, which can't be taken back, so an abandoned reset that did ends with backup off
//!
//! Every stage of an operation ends by itself or can be cancelled:
//! - `Stage::Running` work is cancellable: a reset revokes the claim, so every later completion
//!   and every write the worker asks the owner for is refused
//! - `Stage::Effecting` work is an off-actor effect that can't be taken back once it started; it
//!   ends with its completion message, and `Effect::bound` names what bounds it

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::oneshot;
use tracing::debug;

use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupTransition};
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::actors::write::{
    ConfigGeneration, ConfiguredWriter, ConfiguredWrites, OffWrites, PolicyBody, WritePolicy,
};
use crate::manager::cloud_backup_manager::admission::{
    AdmissionBody, PauseReason, Rejection, ReviewExit, UserIntent,
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
    CloudBackupPendingEnableRecovery, CloudBackupStatus,
};
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
use crate::manager::cloud_backup_manager::owner::lifecycle::{CommittedDisable, DisablingRecord};
use crate::manager::cloud_backup_manager::owner::operation::OperationId;
use crate::manager::cloud_backup_manager::screen::CloudBackupTurnOffFailure;
use crate::manager::cloud_backup_manager::turn_off::{
    AdmittedFinish, ConfirmedTurnOff, DeleteAuthority, FinishFailure, FinishRun, FinishStart,
    RetryWake, ReviewId, TurnOffReview,
};

/// The lifecycle body and the operation running in it
#[derive(Debug)]
pub(crate) enum Runtime {
    Disabled {
        op: Option<DisabledOp>,
    },
    Configured(ConfiguredBody),
    FinishingDisable {
        committed: CommittedDisable,
        op: Option<ResumeDisable>,
        /// Why the last finishing run stopped, while nothing runs
        failure: Option<FinishFailure>,
        /// The automatic retry armed for the last failure's `retry_after`, while nothing runs
        wake: Option<RetryWake>,
    },
    DriveAccountSwitch {
        op: Option<SwitchOp>,
    },
    /// An interrupted Enable the owner found at restart; it holds until its keychain evidence is
    /// gone, whatever the record says
    PendingEnableRecovery {
        recovery: CloudBackupPendingEnableRecovery,
        op: Option<RecoveryOp>,
    },
    /// Load normalization failed; the legacy record is unchanged and nothing can start
    LoadFailed {
        error: DatabaseError,
    },
    /// The stored record can't be read as a backup; nothing runs on it, and its one exit is a
    /// local reset of Cloud Backup on this device, after which the body is off
    Corrupted {
        error: String,
    },
    /// A local reset closed admission; an effect that was in flight finishes here, and nothing
    /// else can start until the owner sets the body the reset left
    Resetting {
        /// The operation whose off-actor effect was running when the reset began; the reset
        /// waits on its stage, so it continues without blocking a handler
        held: Option<OpRun>,
        /// The interrupted-Enable recovery the body held when the reset began; a reset that is
        /// abandoned before its wipe returns to it while the keychain still holds its evidence
        recovery: Option<CloudBackupPendingEnableRecovery>,
        /// Whether the reset began clearing the local keys
        keys: ResetKeys,
    },
}

/// What a local reset did to Cloud Backup's local keys
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetKeys {
    /// Nothing was cleared, so an abandoned reset leaves the backup as it was
    Kept,
    /// The owner committed Cloud Backup off and the clear began; a partial clear can't be taken
    /// back either, so an abandoned reset can't write with the configuration it had
    Cleared,
}

/// Where a running operation stands
#[derive(Debug, Default)]
pub(crate) enum Stage {
    /// The worker is running; a reset cancels it by revoking the claim
    #[default]
    Running,
    /// An off-actor effect that can't be cancelled is in flight; the operation keeps authority
    /// until the effect's completion message arrives
    Effecting {
        effect: Effect,
        /// Told when the effect's completion arrives
        settled: Vec<oneshot::Sender<()>>,
    },
}

/// An off-actor effect an operation waits on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Turn Off's local cleanup: keychain items, the master key and sync rows
    LocalCleanup,
    /// A platform passkey request whose credential, once created, must be recorded
    PasskeyRegistration,
}

/// What ends a stage that can't be cancelled
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageBound {
    /// A synchronous platform keychain call on a blocking thread, which returns or fails
    KeychainCall,
    /// The platform's interactive passkey timeout: five minutes on iOS
    /// (`PasskeyProviderImpl.interactiveRequestTimeout`); Android has no request-level cap yet
    PlatformPasskeyTimeout,
}

impl Effect {
    pub(crate) fn bound(self) -> StageBound {
        match self {
            Self::LocalCleanup => StageBound::KeychainCall,
            Self::PasskeyRegistration => StageBound::PlatformPasskeyTimeout,
        }
    }
}

/// Work that starts from a device without a configured backup
#[derive(Debug)]
pub(crate) enum DisabledOp {
    Enable(OpRun),
    Restore(RestoreRun),
    /// Discarding an Enable that stopped at a prompt, before any recovery was projected
    DiscardEnable(OpRun),
}

/// A configured backup: idle, where the Turn Off review lives, or running one operation
///
/// The review holds no operation and is bound to evidence any operation may change, so a body
/// never holds both: starting an operation replaces the idle body and the review with it
#[derive(Debug)]
pub(crate) enum ConfiguredBody {
    Idle {
        /// Turn Off between the tap and consent
        review: Option<TurnOffReview>,
    },
    Running(ConfiguredOp),
}

impl ConfiguredBody {
    const IDLE: Self = Self::Idle { review: None };
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
    /// Discarding a reinitialize that stopped before it finished; its remote delete runs off the
    /// mailbox, and the configured backup it would have replaced stays as it is
    DiscardReinitialize(OpRun),
}

/// A committed Turn Off going forward; nothing else runs during one
///
/// How the run started travels with its [`FinishRun`], so the body holds only the operation
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

/// A running operation, identified by its claim, which carries its `OperationId`, and its stage
#[derive(Debug)]
pub(crate) struct OpRun {
    claim: CloudBackupExclusiveOperationClaim,
    stage: Stage,
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
        Self { claim, stage: Stage::Running }
    }

    /// Ends an `Effecting` stage and tells everyone waiting on it
    fn effect_reported(&mut self) {
        if let Stage::Effecting { effect, settled } = std::mem::take(&mut self.stage) {
            debug!("Cloud Backup {:?} finished effecting {effect:?}", self.claim);
            for waiter in settled {
                let _ = waiter.send(());
            }
        }
    }
}

/// What a reset did with the operation that was running when it began
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetStart {
    /// Nothing was running
    Idle,
    /// A cancellable operation ended; its later completions are stale
    Cancelled(CloudBackupExclusiveOperationClaim),
    /// An effect is in flight; the reset continues when its completion arrives
    AwaitingEffect(CloudBackupExclusiveOperationClaim),
}

/// What a finish did with the operation it named
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The named operation was running, and it ended
    Ended,
    /// The named operation no longer runs, so nothing changed
    NotCurrent { running: Option<CloudBackupExclusiveOperationClaim> },
}

/// What an idle body holds beyond its record, carried across a rebase on the same body
enum IdlePresentation {
    None,
    Review(TurnOffReview),
    Finishing { attempt_id: u64, failure: Option<FinishFailure>, wake: Option<RetryWake> },
}

/// The running operation, taken out of its body
enum TakenOp {
    Standard(OpRun),
    Restore(RestoreRun),
}

/// An operation the current body has no place for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unstartable {
    pub(crate) body: AdmissionBody,
    pub(crate) operation: CloudBackupExclusiveOperation,
}

/// The owner's facts an idle body is rebased on
pub(crate) struct IdleFacts<'a> {
    /// The record as the supervisor's writer last read or committed it
    pub(crate) record: &'a PersistedCloudBackupState,
    /// The interrupted Enable the owner holds, while its keychain evidence remains
    pub(crate) pending_enable_recovery: Option<CloudBackupPendingEnableRecovery>,
}

impl<'a> IdleFacts<'a> {
    /// The facts for the owner's record, or the error of a record it never read
    pub(crate) fn of(
        record: &'a Result<PersistedCloudBackupState, DatabaseError>,
        pending_enable_recovery: Option<CloudBackupPendingEnableRecovery>,
    ) -> Result<Self, DatabaseError> {
        record.as_ref().map(|record| Self { record, pending_enable_recovery }).map_err(Clone::clone)
    }
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
    /// The idle body the owner's facts give; a record the owner could not read gives `LoadFailed`
    pub(crate) fn idle(facts: Result<IdleFacts<'_>, DatabaseError>) -> Self {
        use PersistedCloudBackupState as State;

        let facts = match facts {
            Ok(facts) => facts,
            Err(error) => return Self::LoadFailed { error },
        };

        // a held recovery outranks only an off or configured record
        if let Some(recovery) = facts.pending_enable_recovery
            && matches!(facts.record, State::Disabled | State::Configured(_))
        {
            return Self::PendingEnableRecovery { recovery, op: None };
        }

        match facts.record {
            State::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(_)) => {
                match DisablingRecord::of(facts.record) {
                    // a Try Again that failed before a relaunch still offers Stop Using; why it
                    // failed is a typed fact of the runtime that saw it
                    Some(DisablingRecord::Committed(committed)) => {
                        let failure = committed.try_again_failed.then_some(
                            FinishFailure::TryAgainFailed(CloudBackupTurnOffFailure::Other),
                        );
                        Self::FinishingDisable { committed, op: None, failure, wake: None }
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
            State::Corrupted { error } => Self::Corrupted { error: error.clone() },
            State::Configured(_) => Self::Configured(ConfiguredBody::IDLE),
            State::Disabled => Self::Disabled { op: None },
        }
    }

    /// Leaves `LoadFailed` for the body the record loads as, once the owner read it again
    pub(crate) fn loaded(&mut self, facts: Result<IdleFacts<'_>, DatabaseError>) {
        if matches!(self, Self::LoadFailed { .. }) {
            *self = Self::idle(facts);
        }
    }

    /// Moves an idle body onto the owner's current facts; a running operation holds its body
    pub(crate) fn rebase_idle(&mut self, facts: Result<IdleFacts<'_>, DatabaseError>) {
        if self.claim().is_some()
            || matches!(self, Self::LoadFailed { .. } | Self::Resetting { .. })
        {
            return;
        }

        let held = self.idle_presentation();
        *self = Self::idle(facts);
        self.keep_idle_presentation(held);
    }

    /// What an idle body holds beyond its record: the Turn Off review, or why a committed Turn
    /// Off last stopped and the retry it keeps armed
    fn idle_presentation(&mut self) -> IdlePresentation {
        match self {
            Self::Configured(ConfiguredBody::Idle { review }) => {
                review.take().map_or(IdlePresentation::None, IdlePresentation::Review)
            }
            Self::FinishingDisable { committed, failure, wake, .. }
                if failure.is_some() || wake.is_some() =>
            {
                IdlePresentation::Finishing {
                    attempt_id: committed.attempt_id,
                    failure: *failure,
                    wake: *wake,
                }
            }
            _ => IdlePresentation::None,
        }
    }

    /// Puts back what [`Self::idle_presentation`] took, while the body is still the one it
    /// belongs to: a review stays with a configured body, a failure and a wake with the same
    /// attempt
    fn keep_idle_presentation(&mut self, held: IdlePresentation) {
        match (self, held) {
            (Self::Configured(body), IdlePresentation::Review(held)) => {
                *body = ConfiguredBody::Idle { review: Some(held) };
            }
            (
                Self::FinishingDisable { committed, failure, wake, .. },
                IdlePresentation::Finishing { attempt_id, failure: held, wake: armed },
            ) if committed.attempt_id == attempt_id => {
                *failure = held;
                *wake = armed;
            }
            _ => {}
        }
    }

    /// Loads an idle body again from a record a test seeded, as a launch on it would
    ///
    /// Unlike [`Self::rebase_idle`] this leaves `LoadFailed` too: a seed replaces the record the
    /// failed load read, so a supervisor shared across tests is not stuck on an earlier test's
    /// legacy record
    #[cfg(test)]
    pub(crate) fn reload_idle(&mut self, facts: Result<IdleFacts<'_>, DatabaseError>) {
        if self.claim().is_some() || matches!(self, Self::Resetting { .. }) {
            return;
        }

        *self = Self::idle(facts);
    }

    /// For a body a rebase would move, the policy of the body it would move to
    ///
    /// The harness invariants compare it with the body's own policy: an idle body behind its
    /// record would publish a stale policy
    #[cfg(test)]
    pub(crate) fn idle_policy_on(
        &self,
        facts: Result<IdleFacts<'_>, DatabaseError>,
        generation: ConfigGeneration,
    ) -> Option<WritePolicy> {
        if self.claim().is_some()
            || matches!(self, Self::LoadFailed { .. } | Self::Resetting { .. })
        {
            return None;
        }

        Some(Self::idle(facts).write_policy(generation))
    }

    /// The interrupted Enable this body holds, running operation or not
    pub(crate) fn pending_enable_recovery(&self) -> Option<&CloudBackupPendingEnableRecovery> {
        match self {
            Self::PendingEnableRecovery { recovery, .. } => Some(recovery),
            _ => None,
        }
    }

    /// The recovery a reset in progress set aside, which an abandoned reset returns to
    pub(crate) fn recovery_held_by_reset(&self) -> Option<&CloudBackupPendingEnableRecovery> {
        match self {
            Self::Resetting { recovery, .. } => recovery.as_ref(),
            _ => None,
        }
    }

    /// Enters recovery for an interrupted Enable the owner assessed, or updates the snapshot of
    /// the one it holds
    ///
    /// Only an idle off or configured body can enter: a running operation owns its body, and a
    /// committed disable, account switch, failed load or corrupt record outranks the recovery
    pub(crate) fn enter_pending_enable_recovery(
        &mut self,
        next: CloudBackupPendingEnableRecovery,
    ) -> bool {
        match self {
            Self::PendingEnableRecovery { recovery, .. } => *recovery = next,
            Self::Disabled { op: None } | Self::Configured(ConfiguredBody::Idle { .. }) => {
                *self = Self::PendingEnableRecovery { recovery: next, op: None };
            }
            Self::Disabled { op: Some(_) }
            | Self::Configured(ConfiguredBody::Running(_))
            | Self::Corrupted { .. }
            | Self::FinishingDisable { .. }
            | Self::DriveAccountSwitch { .. }
            | Self::LoadFailed { .. }
            | Self::Resetting { .. } => return false,
        }

        true
    }

    /// The body admission decides on, with the running operation's kind
    pub(crate) fn admission_body(&self) -> AdmissionBody {
        match self {
            Self::Disabled { op: None | Some(DisabledOp::DiscardEnable(_)) } => {
                AdmissionBody::Disabled
            }
            Self::Disabled { op: Some(DisabledOp::Enable(_)) } => AdmissionBody::Enabling,
            Self::Disabled { op: Some(DisabledOp::Restore(_)) } => AdmissionBody::Restoring,
            Self::Corrupted { .. } => AdmissionBody::Corrupted,
            Self::Configured(_) => AdmissionBody::Configured,
            Self::DriveAccountSwitch { .. } => AdmissionBody::DriveAccountSwitch,
            Self::FinishingDisable { .. } => AdmissionBody::FinishingDisable,
            Self::PendingEnableRecovery { .. } => AdmissionBody::PendingEnableRecovery,
            Self::LoadFailed { .. } => AdmissionBody::LoadFailed,
            Self::Resetting { .. } => AdmissionBody::Resetting,
        }
    }

    pub(crate) fn is_some(&self) -> bool {
        self.claim().is_some()
    }

    pub(crate) fn is_restore_all(&self) -> bool {
        matches!(self, Self::Configured(ConfiguredBody::Running(ConfiguredOp::RestoreAll(_))))
    }

    /// The claim of the running operation, which carries its `OperationId`
    pub(crate) fn claim(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        match self {
            Self::Disabled { op } => op.as_ref().map(DisabledOp::claim),
            Self::Configured(ConfiguredBody::Running(op)) => Some(op.claim()),
            Self::Configured(ConfiguredBody::Idle { .. }) => None,
            Self::FinishingDisable { op, .. } => op.as_ref().map(|op| op.0.claim),
            Self::DriveAccountSwitch { op } => op.as_ref().map(|op| op.0.claim),
            Self::PendingEnableRecovery { op, .. } => op.as_ref().map(RecoveryOp::claim),
            // the held effect answers only through `accepts_completion`; nothing else may use it
            Self::Corrupted { .. } | Self::LoadFailed { .. } | Self::Resetting { .. } => None,
        }
    }

    /// The running standard operation, where its stage lives; `op_run` is the shared twin
    fn op_run_mut(&mut self) -> Option<&mut OpRun> {
        match self {
            Self::Disabled {
                op: Some(DisabledOp::Enable(run) | DisabledOp::DiscardEnable(run)),
            }
            | Self::FinishingDisable { op: Some(ResumeDisable(run)), .. }
            | Self::DriveAccountSwitch { op: Some(SwitchOp(run)) }
            | Self::PendingEnableRecovery {
                op: Some(RecoveryOp::Continue(run) | RecoveryOp::Discard(run)),
                ..
            }
            | Self::Configured(ConfiguredBody::Running(
                ConfiguredOp::TurnOff(run)
                | ConfiguredOp::Verify(run)
                | ConfiguredOp::RepairPasskey(run)
                | ConfiguredOp::RestoreCloudWallet(run)
                | ConfiguredOp::DeleteRecords(run)
                | ConfiguredOp::RecoverOtherBackups(run)
                | ConfiguredOp::DeleteOtherBackups(run)
                | ConfiguredOp::Reinitialize(run)
                | ConfiguredOp::RecreateManifest(run)
                | ConfiguredOp::DiscardReinitialize(run),
            )) => Some(run),
            Self::Resetting { held, .. } => held.as_mut(),
            _ => None,
        }
    }

    /// Puts the operation with `claim` in an `Effecting` stage for `effect`
    ///
    /// Returns false when `claim` no longer runs, so the caller must not start the effect
    pub(crate) fn begin_effect(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        effect: Effect,
    ) -> bool {
        if !self.is_current(claim) {
            return false;
        }

        let Some(run) = self.op_run_mut() else { return false };
        debug!("Cloud Backup {claim:?} is effecting {effect:?}, bounded by {:?}", effect.bound());
        run.stage = Stage::Effecting { effect, settled: Vec::new() };
        true
    }

    /// Answers `settled` once no off-actor effect is in flight
    pub(crate) fn when_effects_settle(&mut self, settled: oneshot::Sender<()>) {
        match self.op_run_mut().map(|run| &mut run.stage) {
            Some(Stage::Effecting { settled: waiters, .. }) => waiters.push(settled),
            Some(Stage::Running) | None => {
                let _ = settled.send(());
            }
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
    ///
    /// The completion an `Effecting` operation waits on ends that stage here, since every
    /// completion passes through this one match. During a reset, the held effect's completion
    /// frees it and lets the reset continue; the operation itself does not go on
    pub(crate) fn accepts_completion(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: &'static str,
    ) -> bool {
        if let Self::Resetting { held, .. } = self
            && let Some(mut run) = held.take_if(|run| run.claim == claim)
        {
            debug!("Cloud Backup reset received {completion} for {claim:?}");
            run.effect_reported();
            return false;
        }

        let current = self.claim();
        if current == Some(claim) {
            if let Some(run) = self.op_run_mut() {
                run.effect_reported();
            }

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
        let unstartable =
            |runtime: &Self| Unstartable { body: runtime.admission_body(), operation };
        if self.is_some() {
            return Err(unstartable(self));
        }

        match (&mut *self, new) {
            (Self::Disabled { op }, NewOp::Standard(run))
                if matches!(operation, Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery) =>
            {
                *op = Some(DisabledOp::Enable(run));
            }
            (Self::Disabled { op }, NewOp::Standard(run))
                if operation == Op::DiscardPendingEnable =>
            {
                *op = Some(DisabledOp::DiscardEnable(run));
            }
            (Self::Disabled { op }, NewOp::Restore(run)) if operation == Op::Restore => {
                *op = Some(DisabledOp::Restore(run));
            }
            (Self::Configured(body), NewOp::Restore(run))
                if operation == Op::RestoreAllCloudWallets =>
            {
                *body = ConfiguredBody::Running(ConfiguredOp::RestoreAll(run));
            }
            // an account switch's reinitialize runs in the switch body, which holds background
            // writes until the switch ends
            (Self::Configured(_), NewOp::Standard(run))
                if run.claim.drive_account_switch_id().is_some() =>
            {
                *self = Self::DriveAccountSwitch { op: Some(SwitchOp(run)) };
            }
            (Self::Configured(body), NewOp::Standard(run)) => {
                let configured = match operation {
                    Op::Disable => ConfiguredOp::TurnOff(run),
                    Op::VerifyReinitializedBackup | Op::Verify => ConfiguredOp::Verify(run),
                    Op::RepairPasskey => ConfiguredOp::RepairPasskey(run),
                    Op::RestoreCloudWallet => ConfiguredOp::RestoreCloudWallet(run),
                    Op::DeleteCloudWallet | Op::DeleteUndecryptableWalletBackups => {
                        ConfiguredOp::DeleteRecords(run)
                    }
                    Op::RecoverOtherBackups => ConfiguredOp::RecoverOtherBackups(run),
                    Op::DeleteOtherBackups => ConfiguredOp::DeleteOtherBackups(run),
                    Op::ReinitializeBackup => ConfiguredOp::Reinitialize(run),
                    Op::RecreateManifest => ConfiguredOp::RecreateManifest(run),
                    Op::DiscardPendingEnable => ConfiguredOp::DiscardReinitialize(run),
                    Op::Enable
                    | Op::EnableForceNew
                    | Op::EnableNoDiscovery
                    | Op::Restore
                    | Op::RestoreAllCloudWallets => return Err(unstartable(self)),
                };
                *body = ConfiguredBody::Running(configured);
            }
            (Self::DriveAccountSwitch { op }, NewOp::Standard(run))
                if operation == Op::ReinitializeBackup =>
            {
                *op = Some(SwitchOp(run));
            }
            (Self::PendingEnableRecovery { op, .. }, NewOp::Standard(run)) => {
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
                    | Op::Verify
                    | Op::RecoverOtherBackups
                    | Op::DeleteOtherBackups
                    | Op::RestoreCloudWallet
                    | Op::RestoreAllCloudWallets
                    | Op::DeleteCloudWallet
                    | Op::DeleteUndecryptableWalletBackups => return Err(unstartable(self)),
                });
            }
            (Self::PendingEnableRecovery { op, .. }, NewOp::Restore(run))
                if operation == Op::Restore =>
            {
                *op = Some(RecoveryOp::Restore(run));
            }
            (
                Self::Disabled { .. }
                | Self::Corrupted { .. }
                | Self::Configured(_)
                | Self::FinishingDisable { .. }
                | Self::DriveAccountSwitch { .. }
                | Self::PendingEnableRecovery { .. }
                | Self::LoadFailed { .. }
                | Self::Resetting { .. },
                NewOp::Standard(_) | NewOp::Restore(_),
            ) => return Err(unstartable(self)),
        }

        Ok(())
    }

    /// The Turn Off with `claim` committed: its body becomes the committed disable, and the same
    /// operation keeps running there as the run that deletes, so nothing configured can start
    /// beside it
    pub(crate) fn commit_turn_off(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        committed: CommittedDisable,
    ) -> Option<FinishRun> {
        let Self::Configured(body) = self else { return None };
        if !matches!(body, ConfiguredBody::Running(ConfiguredOp::TurnOff(run)) if run.claim == claim)
        {
            return None;
        }

        let ConfiguredBody::Running(ConfiguredOp::TurnOff(run)) =
            std::mem::replace(body, ConfiguredBody::IDLE)
        else {
            return None;
        };

        let start = FinishStart::of(committed.disposition, DeleteAuthority::Commit);
        *self = Self::FinishingDisable {
            committed,
            op: Some(ResumeDisable(run)),
            failure: None,
            wake: None,
        };

        Some(FinishRun { claim, start })
    }

    /// Consent to the review `review_id`: takes it out of the idle configured body and starts
    /// Turn Off in its place, in one step
    ///
    /// The review and the claim travel together from here, so a confirmed Turn Off always has
    /// its operation and an operation never runs beside a review
    pub(crate) fn admit_confirm(
        &mut self,
        review_id: ReviewId,
    ) -> Result<ConfirmedTurnOff, Rejection> {
        let Self::Configured(ConfiguredBody::Idle { review }) = self else {
            return Err(Rejection::ReviewStale);
        };
        let shown = review.take_if(|held| held.reviewing_id() == Some(review_id));
        let Some(TurnOffReview::Reviewing(review)) = shown else {
            return Err(Rejection::ReviewStale);
        };

        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            OperationId::next(),
        );
        *self = Self::Configured(ConfiguredBody::Running(ConfiguredOp::TurnOff(OpRun::new(claim))));
        Ok(ConfirmedTurnOff { claim, review })
    }

    /// Starts a finishing run of the committed Turn Off this body holds, with `authority`
    ///
    /// The run's start comes from the record's disposition as well: a committed local-only
    /// finish starts as one whoever resumes it. The armed retry ends, since this run is the
    /// retry
    pub(crate) fn start_finish(
        &mut self,
        authority: DeleteAuthority,
    ) -> Result<AdmittedFinish, Rejection> {
        let Self::FinishingDisable { committed, op, wake, .. } = self else {
            return Err(Rejection::NothingToContinue);
        };
        if let Some(running) = op {
            return Err(Rejection::OperationRunning(running.0.claim.operation()));
        }

        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            OperationId::next(),
        );
        let start = FinishStart::of(committed.disposition, authority);
        *op = Some(ResumeDisable(OpRun::new(claim)));
        *wake = None;

        Ok(AdmittedFinish { run: FinishRun { claim, start }, committed: committed.clone() })
    }

    /// The finishing run `run` stopped for `reason`; the committed Turn Off waits for the next
    /// resume
    ///
    /// Returns the failure the body now holds, or `None` when `run` is no longer running
    pub(crate) fn fail_finish(
        &mut self,
        run: FinishRun,
        reason: CloudBackupTurnOffFailure,
    ) -> Option<FinishFailure> {
        let Self::FinishingDisable { op, failure, .. } = self else { return None };
        if !op.as_ref().is_some_and(|op| op.0.claim == run.claim) {
            return None;
        }

        *op = None;
        *failure = Some(FinishFailure::after(*failure, run.start, reason));
        *failure
    }

    /// Arms the automatic retry for `at`, the idle committed Turn Off's `retry_after`
    ///
    /// Returns the wake to schedule, or `None` when the same one is already armed or nothing is
    /// waiting to finish; a wake for an earlier deadline is replaced, so its timer finds nothing
    pub(crate) fn arm_retry_wake(&mut self, at: u64) -> Option<RetryWake> {
        let Self::FinishingDisable { committed, op: None, wake, .. } = self else { return None };
        let armed = RetryWake { attempt_id: committed.attempt_id, at };
        if *wake == Some(armed) {
            return None;
        }

        *wake = Some(armed);
        Some(armed)
    }

    /// Disarms `fired` if it is still the retry this idle body keeps armed
    pub(crate) fn take_retry_wake(&mut self, fired: RetryWake) -> bool {
        let Self::FinishingDisable { op: None, wake, .. } = self else { return false };
        wake.take_if(|armed| *armed == fired).is_some()
    }

    /// The Turn Off review an idle configured body holds
    pub(crate) fn turn_off_review(&self) -> Option<&TurnOffReview> {
        match self {
            Self::Configured(ConfiguredBody::Idle { review }) => review.as_ref(),
            _ => None,
        }
    }

    /// Replaces the Turn Off review; only an idle configured body holds one
    ///
    /// Admission starts a review only in an idle configured body, and a review that completes
    /// late finds its own `Preparing` there first, so only a clear reaches another body, where
    /// there is no review to clear
    pub(crate) fn set_turn_off_review(&mut self, next: Option<TurnOffReview>) {
        let Self::Configured(ConfiguredBody::Idle { review }) = self else {
            debug_assert!(next.is_none(), "Cloud Backup showed a Turn Off review in {self:?}");
            return;
        };

        *review = next;
    }

    /// A review button left the review `review_id`: the review ends, and this returns the work
    /// the button names, which the caller starts in the same turn
    pub(crate) fn leave_review(
        &mut self,
        review_id: ReviewId,
        to: ReviewExit,
    ) -> Result<UserIntent, Rejection> {
        let Self::Configured(ConfiguredBody::Idle { review }) = self else {
            return Err(Rejection::ReviewStale);
        };
        let Some(TurnOffReview::Reviewing(held)) =
            review.as_ref().filter(|held| held.reviewing_id() == Some(review_id))
        else {
            return Err(Rejection::ReviewStale);
        };

        // admission rejects Restore Them First on a review that names nothing to restore
        let target = held.impact.exit_intent(to).ok_or(Rejection::RestoreAllUnavailable)?;
        *review = None;
        Ok(target)
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
        facts: Result<IdleFacts<'_>, DatabaseError>,
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
            | Self::PendingEnableRecovery { op: Some(RecoveryOp::Restore(run)), .. }
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
            Self::Configured(ConfiguredBody::Running(ConfiguredOp::RestoreAll(run)))
                if run.claim == claim =>
            {
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
            Self::Configured(ConfiguredBody::Running(ConfiguredOp::RestoreAll(run)))
                if run.claim == claim =>
            {
                Some(run)
            }
            _ => None,
        }
    }

    /// Closes admission for a local reset
    ///
    /// A cancellable operation ends here: its claim is revoked, so its later completions are
    /// stale and the owner refuses the writes it asks for, and a restore worker also sees its
    /// cancellation. An operation in an `Effecting` stage is held until its completion arrives,
    /// and `ready` is answered then, so the reset never waits inside a handler
    pub(crate) fn begin_reset(&mut self, ready: oneshot::Sender<()>) -> ResetStart {
        let recovery = self.pending_enable_recovery().cloned();
        let taken = self.take_op();
        let (held, start) = match taken {
            None => (None, ResetStart::Idle),
            Some(TakenOp::Restore(run)) => {
                run.cancellation.store(true, Ordering::Release);
                (None, ResetStart::Cancelled(run.claim))
            }
            Some(TakenOp::Standard(run)) => match run.stage {
                Stage::Effecting { .. } => {
                    let claim = run.claim;
                    (Some(run), ResetStart::AwaitingEffect(claim))
                }
                Stage::Running => (None, ResetStart::Cancelled(run.claim)),
            },
        };

        *self = Self::Resetting { held, recovery, keys: ResetKeys::Kept };
        self.when_effects_settle(ready);
        start
    }

    /// The key state of a ready reset; `None` when no ready reset holds the body, so nothing may
    /// clear the local keys
    pub(crate) fn ready_reset_keys(&mut self) -> Option<&mut ResetKeys> {
        let Self::Resetting { held: None, keys, .. } = self else { return None };
        Some(keys)
    }

    /// What the running reset did to the local keys; `None` when no reset runs
    pub(crate) fn reset_keys(&self) -> Option<ResetKeys> {
        match self {
            Self::Resetting { keys, .. } => Some(*keys),
            _ => None,
        }
    }

    /// Replaces a finished reset with the body it left
    pub(crate) fn finish_reset(&mut self, after: Self) {
        if let Self::Resetting { held: Some(run), .. } = self {
            // the reset only continues once its held effect reported, so this is a bug
            debug_assert!(false, "Cloud Backup reset finished while {:?} was in flight", run.claim);
        }

        *self = after;
    }

    /// The body when nothing has been loaded or committed yet
    pub(crate) fn off() -> Self {
        Self::Disabled { op: None }
    }

    fn take_op(&mut self) -> Option<TakenOp> {
        match self {
            Self::Disabled { op } => op.take().map(|op| match op {
                DisabledOp::Enable(run) | DisabledOp::DiscardEnable(run) => TakenOp::Standard(run),
                DisabledOp::Restore(run) => TakenOp::Restore(run),
            }),
            Self::Configured(body) => match std::mem::replace(body, ConfiguredBody::IDLE) {
                ConfiguredBody::Idle { .. } => None,
                ConfiguredBody::Running(op) => Some(match op {
                    ConfiguredOp::RestoreAll(run) => TakenOp::Restore(run),
                    ConfiguredOp::TurnOff(run)
                    | ConfiguredOp::Verify(run)
                    | ConfiguredOp::RepairPasskey(run)
                    | ConfiguredOp::RestoreCloudWallet(run)
                    | ConfiguredOp::DeleteRecords(run)
                    | ConfiguredOp::RecoverOtherBackups(run)
                    | ConfiguredOp::DeleteOtherBackups(run)
                    | ConfiguredOp::Reinitialize(run)
                    | ConfiguredOp::RecreateManifest(run)
                    | ConfiguredOp::DiscardReinitialize(run) => TakenOp::Standard(run),
                }),
            },
            Self::FinishingDisable { op, .. } => op.take().map(|op| TakenOp::Standard(op.0)),
            Self::DriveAccountSwitch { op } => op.take().map(|op| TakenOp::Standard(op.0)),
            Self::PendingEnableRecovery { op, .. } => op.take().map(|op| match op {
                RecoveryOp::Continue(run) | RecoveryOp::Discard(run) => TakenOp::Standard(run),
                RecoveryOp::Restore(run) => TakenOp::Restore(run),
            }),
            Self::Resetting { held, .. } => held.take().map(TakenOp::Standard),
            Self::Corrupted { .. } | Self::LoadFailed { .. } => None,
        }
    }

    /// Ends the operation `claim` names, if it is the one running; the body stays until the next
    /// start rebases it
    ///
    /// A finish for an operation that no longer runs changes nothing, so a late or repeated
    /// finish can never end the operation that replaced it
    pub(crate) fn finish(&mut self, claim: CloudBackupExclusiveOperationClaim) -> Finish {
        if !self.is_current(claim) {
            return Finish::NotCurrent { running: self.claim() };
        }

        self.clear();
        Finish::Ended
    }

    /// Ends the running operation, whichever it is
    fn clear(&mut self) {
        match self {
            Self::Disabled { op } => *op = None,
            // an idle body keeps its review; a running one has none to keep
            Self::Configured(body @ ConfiguredBody::Running(_)) => *body = ConfiguredBody::IDLE,
            Self::Configured(ConfiguredBody::Idle { .. }) => {}
            Self::FinishingDisable { op, .. } => *op = None,
            Self::DriveAccountSwitch { op } => *op = None,
            Self::PendingEnableRecovery { op, .. } => *op = None,
            Self::Resetting { held, .. } => {
                if let Some(mut run) = held.take() {
                    run.effect_reported();
                }
            }
            Self::Corrupted { .. } | Self::LoadFailed { .. } => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn take_claim(&mut self) -> Option<CloudBackupExclusiveOperationClaim> {
        let claim = self.claim();
        self.clear();
        claim
    }
}

impl Runtime {
    /// The write policy this runtime gives, per command class; see `actors/write/policy.rs`
    ///
    /// Derived, never stored: the supervisor publishes it after every runtime change
    pub(crate) fn write_policy(&self, generation: ConfigGeneration) -> WritePolicy {
        let body = match self {
            Self::Corrupted { .. } => PolicyBody::Off { op: None },
            Self::Disabled { op } => PolicyBody::Off {
                op: op.as_ref().map(|op| match op {
                    DisabledOp::Enable(run) => OffWrites::Enable(run.claim),
                    DisabledOp::DiscardEnable(run) => OffWrites::Discard(run.claim),
                    DisabledOp::Restore(run) => OffWrites::Restore(run.claim),
                }),
            },
            Self::PendingEnableRecovery { op, .. } => PolicyBody::Off {
                op: op.as_ref().map(|op| match op {
                    RecoveryOp::Continue(run) => OffWrites::Enable(run.claim),
                    RecoveryOp::Discard(run) => OffWrites::Discard(run.claim),
                    RecoveryOp::Restore(run) => OffWrites::Restore(run.claim),
                }),
            },
            Self::Configured(body) => PolicyBody::Configured {
                op: match body {
                    ConfiguredBody::Idle { .. } => None,
                    ConfiguredBody::Running(op) => {
                        Some(ConfiguredWrites { claim: op.claim(), kind: op.writer() })
                    }
                },
            },
            Self::FinishingDisable { committed, op, .. } => PolicyBody::FinishingDisable {
                namespace_id: committed.namespace_id.clone(),
                op: op.as_ref().map(|op| op.0.claim),
            },
            Self::DriveAccountSwitch { op } => {
                PolicyBody::AccountSwitch { op: op.as_ref().map(|op| op.0.claim) }
            }
            Self::LoadFailed { .. } => PolicyBody::Closed(PauseReason::LoadFailed),
            Self::Resetting { .. } => PolicyBody::Closed(PauseReason::LocalReset),
        };

        WritePolicy { generation, body }
    }

    /// The claim of a running operation that replaces the configuration in place
    ///
    /// A reinitialize writes a new master key and namespace while the body stays configured, so
    /// the configuration generation changes when it ends
    pub(crate) fn replacing_configuration(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        match self {
            Self::Configured(ConfiguredBody::Running(ConfiguredOp::Reinitialize(run))) => {
                Some(run.claim)
            }
            _ => None,
        }
    }
}

/// What the runtime shows through the reducer's status
///
/// Read-only: it never feeds back into `Runtime`. `None` leaves the status to the reducer's
/// record projection; a committed Turn Off's failure is the runtime's own, read by the screen
/// projection through [`Runtime::finishing_disable_stage`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportedRuntime {
    pub(crate) status: Option<CloudBackupStatus>,
}

impl Runtime {
    /// The status only the runtime can give: a failed load, and a committed Turn Off finishing
    pub(crate) fn exported(&self) -> ExportedRuntime {
        let status = match self {
            Self::LoadFailed { error } => {
                debug!("Cloud Backup runtime is LoadFailed: {error}");
                Some(CloudBackupStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into()))
            }
            Self::FinishingDisable { op: Some(_), .. } => Some(CloudBackupStatus::Disabling),
            // the record alone gives these statuses, and the record projection is their one writer;
            // repeating them here would let a late projection overwrite a status set after it
            Self::Corrupted { error, .. } => {
                debug!("Cloud Backup runtime is Corrupted: {error}");
                None
            }
            Self::Disabled { .. }
            | Self::FinishingDisable { op: None, .. }
            | Self::Configured(_)
            | Self::DriveAccountSwitch { .. }
            | Self::PendingEnableRecovery { .. }
            // the reset's own projection shows the wipe
            | Self::Resetting { .. } => None,
        };

        ExportedRuntime { status }
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
    /// How the operation writes through the lane
    fn writer(&self) -> ConfiguredWriter {
        match self {
            Self::TurnOff(_) => ConfiguredWriter::TurnOff,
            Self::DeleteRecords(_) => ConfiguredWriter::RowDelete,
            Self::RecoverOtherBackups(_) | Self::DeleteOtherBackups(_) => {
                ConfiguredWriter::OtherBackups
            }
            Self::Reinitialize(_) => ConfiguredWriter::Reinitialize,
            Self::DiscardReinitialize(_) => ConfiguredWriter::Discard,
            Self::Verify(_)
            | Self::RepairPasskey(_)
            | Self::RestoreCloudWallet(_)
            | Self::RestoreAll(_)
            | Self::RecreateManifest(_) => ConfiguredWriter::Other,
        }
    }

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
            | Self::RecreateManifest(run)
            | Self::DiscardReinitialize(run) => run.claim,
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
