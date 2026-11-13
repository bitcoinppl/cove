//! The supervisor's runtime and durable record, with the write policy published on every change
//!
//! [`OwnedRuntime`] is the only way to change the stored [`Runtime`] or the durable lifecycle
//! record: it derefs for reads and has no `DerefMut`, and it holds the only [`LifecycleWriter`],
//! which callers reach only through [`OwnedRuntime::commit`]. Every runtime mutation and every
//! record write therefore ends in the same publish, which rebases an idle body on the owner's
//! record and pushes the derived [`WritePolicy`] to the write supervisor when it changed. The
//! compiler, not each call site, guarantees the write lane sees every change of either
//!
//! One record write skips that publish: [`OwnedRuntime::clear_keys_in_reset`] commits Cloud
//! Backup off straight through the writer before a ready reset clears the keys. The resetting
//! body's policy is already `Closed(LocalReset)` and that write can't change it, and
//! [`OwnedRuntime::finish_reset`] publishes the policy the reset ends with

use std::ops::Deref;
use std::sync::Weak;

use act_zero::{Addr, send};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use super::runtime::{
    Effect, Finish, IdleFacts, ResetKeys, ResetStart, RestoreRun, Runtime, Unstartable,
};
use super::{AbandonedReset, KeyClearRefused, LocalResetOutcome, ResetEnd};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::actors::write::{
    CloudBackupWriteSupervisor, ConfigGeneration, VersionedPolicy, WritePolicy,
};
use crate::manager::cloud_backup_manager::admission::{Rejection, ReviewExit, UserIntent};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperationClaim, CloudBackupPendingEnableRecovery,
};
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
#[cfg(test)]
use crate::manager::cloud_backup_manager::owner::lifecycle::CommittedDisable;
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    DisableCommitReview, LifecycleLoad, LifecycleWriter,
};
use crate::manager::cloud_backup_manager::screen::CloudBackupTurnOffFailure;
use crate::manager::cloud_backup_manager::turn_off::{
    AdmittedFinish, ConfirmedTurnOff, DeleteAuthority, FinishFailure, FinishRun, RetryWake,
    ReviewId, TurnOffCommit, TurnOffReview,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupError, CloudBackupStatus, InventoryScope, RustCloudBackupManager,
};

/// The stored runtime, the only lifecycle writer, and the write policy last published for them
#[derive(Debug)]
pub(crate) struct OwnedRuntime {
    runtime: Runtime,
    lifecycle: LifecycleWriter,
    write: Addr<CloudBackupWriteSupervisor>,
    manager: Weak<RustCloudBackupManager>,
    /// The configuration generation commands with no operation are matched against
    generation: ConfigGeneration,
    /// The running reinitialize at the last publish, which replaces the configuration when it ends
    replacing: Option<CloudBackupExclusiveOperationClaim>,
    published: VersionedPolicy,
    /// Every operation start and end since the harness last read them
    #[cfg(test)]
    transitions: Vec<OpTransition>,
}

impl Deref for OwnedRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Runtime {
        &self.runtime
    }
}

impl OwnedRuntime {
    /// The body bootstrap left: `LoadFailed` when normalization failed, otherwise the idle body
    /// of the record the writer read
    pub(crate) fn at_load(
        load_error: Option<DatabaseError>,
        lifecycle: LifecycleWriter,
        write: Addr<CloudBackupWriteSupervisor>,
        manager: Weak<RustCloudBackupManager>,
    ) -> Self {
        let record = match load_error {
            Some(error) => Err(error),
            None => lifecycle.record(),
        };

        let runtime = Runtime::idle(IdleFacts::of(&record, None));
        let generation = ConfigGeneration::next();
        let mut owned = Self {
            published: VersionedPolicy { version: 0, policy: WritePolicy::starting() },
            replacing: runtime.replacing_configuration(),
            runtime,
            lifecycle,
            write,
            manager,
            generation,
            #[cfg(test)]
            transitions: Vec::new(),
        };

        owned.publish_policy();
        owned
    }

    /// The configuration generation of the current body
    pub(crate) fn generation(&self) -> ConfigGeneration {
        self.generation
    }

    /// The policy last pushed to the write supervisor, with its version
    pub(crate) fn published_policy(&self) -> &VersionedPolicy {
        &self.published
    }

    fn mutate<T>(&mut self, change: impl FnOnce(&mut Runtime) -> T) -> T {
        #[cfg(test)]
        let before = self.runtime.claim();

        let result = change(&mut self.runtime);

        #[cfg(test)]
        self.record_transition(before);

        self.publish();
        result
    }

    /// Rebases an idle body on the owner's record, then publishes the policy if it changed and
    /// asks for the screen to be projected again
    ///
    /// The screen reads the runtime, so a runtime change is a screen change as much as a reducer
    /// change is; the request queues behind the handler making this change
    fn publish(&mut self) {
        let record = self.lifecycle.record();
        let pending_enable_recovery = self.held_pending_enable_recovery();
        self.runtime.rebase_idle(IdleFacts::of(&record, pending_enable_recovery));
        self.publish_policy();
        self.project_pending_enable_recovery(&record);

        if let Some(manager) = self.manager.upgrade() {
            manager.request_screen_publish();
        }
    }

    /// The status the owner's body and record give
    ///
    /// A body with a status of its own, such as `LoadFailed`, gives that; otherwise the record
    /// does. The record is the owner's, so a failed read never shows as another status
    pub(crate) fn status(&self) -> CloudBackupStatus {
        Self::status_on(&self.runtime, &self.lifecycle.record())
    }

    fn status_on(
        runtime: &Runtime,
        record: &Result<PersistedCloudBackupState, DatabaseError>,
    ) -> CloudBackupStatus {
        if let Some(status) = runtime.exported().status {
            return status;
        }

        // only `LoadFailed` has no record, and it exports its own status above
        record
            .as_ref()
            .ok()
            .and_then(RustCloudBackupManager::runtime_status_for)
            .unwrap_or_else(|| CloudBackupStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into()))
    }

    /// The interrupted Enable this runtime holds, while the keychain still holds its evidence
    ///
    /// The keychain is read only while the body is in recovery, and an unreadable keychain counts
    /// as evidence, so unknown state never ends a recovery
    fn held_pending_enable_recovery(&self) -> Option<CloudBackupPendingEnableRecovery> {
        let recovery = self.runtime.pending_enable_recovery()?;
        RustCloudBackupManager::pending_enable_evidence_remains().then(|| recovery.clone())
    }

    /// Shows the recovery an idle body holds, and ends the recovery projection once the body left
    ///
    /// While an operation runs in recovery its own projection shows; Discard keeps the recovery
    /// screen, and Enable shows its progress
    fn project_pending_enable_recovery(
        &self,
        record: &Result<PersistedCloudBackupState, DatabaseError>,
    ) {
        let Some(manager) = self.manager.upgrade() else { return };
        let projected = manager.state.read().projected_pending_enable_recovery().cloned();

        match &self.runtime {
            Runtime::PendingEnableRecovery { recovery, op: None } => {
                if projected.as_ref() != Some(recovery) {
                    manager.project_pending_enable_recovery(recovery.clone());
                }
            }
            Runtime::PendingEnableRecovery { op: Some(_), .. } => {}

            // a reset set the recovery aside, and it shows until the wipe ends its evidence or an
            // abandoned reset returns to it
            Runtime::Resetting { recovery: Some(_), .. } => {}
            _ if projected.is_some() => {
                manager.clear_pending_enable_recovery(Self::status_on(&self.runtime, record));
            }
            _ => {}
        }
    }

    fn publish_policy(&mut self) {
        let replacing = self.runtime.replacing_configuration();
        let mut policy = self.runtime.write_policy(self.generation);
        let body_changed = std::mem::discriminant(&policy.body)
            != std::mem::discriminant(&self.published.policy.body);
        let replaced = self.replacing.is_some() && replacing != self.replacing;
        self.replacing = replacing;

        // a new body kind, or a reinitialize that ended, starts a new configuration, and the
        // inventory of the old one is dropped with every result still in flight for it
        if self.published.version > 0 && (body_changed || replaced) {
            self.generation = ConfigGeneration::next();
            policy.generation = self.generation;

            if let Some(manager) = self.manager.upgrade() {
                manager.enter_inventory_scope(InventoryScope::of(self.generation));
            }
        }

        if self.published.version > 0 && policy == self.published.policy {
            return;
        }

        self.published = VersionedPolicy { version: self.published.version + 1, policy };
        debug!("Cloud Backup published write policy {:?}", self.published);
        send!(self.write.apply_policy(self.published.clone()));
    }

    /// Writes the durable lifecycle record, then rebases an idle body on it and publishes
    ///
    /// The writer is reachable only inside `write`, so no record write can skip the publish that
    /// keeps the write lane's policy in step with the record
    pub(crate) fn commit<T>(&mut self, write: impl FnOnce(&LifecycleWriter) -> T) -> T {
        let result = write(&self.lifecycle);
        self.publish();
        result
    }

    /// Rebases after a change outside the record that an idle body derives from
    ///
    /// Only the pending Enable evidence in the keychain is such a fact: the record has one writer,
    /// [`Self::commit`], which publishes on its own
    pub(crate) fn rebase_on_keychain(&mut self) {
        self.publish();
    }

    /// Loads an idle body again from a record a test seeded without the owner, and publishes
    #[cfg(test)]
    pub(crate) fn rebase_after_test_write(&mut self) {
        let record = self.lifecycle.record();
        let pending_enable_recovery = self.held_pending_enable_recovery();
        self.runtime.reload_idle(IdleFacts::of(&record, pending_enable_recovery));
        self.publish();
    }

    /// Reads the record again after a failed load and leaves `LoadFailed` once it reads
    pub(crate) fn retry_load(&mut self) -> Result<(), DatabaseError> {
        if let LifecycleLoad::Failed(error) = self.lifecycle.retry_load() {
            return Err(error);
        }

        let record = self.lifecycle.record();
        self.mutate(|runtime| runtime.loaded(IdleFacts::of(&record, None)));

        Ok(())
    }

    /// Ends the operation `claim` names; the one place an operation finishes
    ///
    /// Finishing an operation that no longer runs changes nothing, and the harness ledger
    /// records the attempt, so a second finish of one operation is visible to invariant 5
    pub(crate) fn finish(&mut self, claim: CloudBackupExclusiveOperationClaim) -> Finish {
        let finish = self.mutate(|runtime| runtime.finish(claim));
        if let Finish::NotCurrent { running } = finish {
            warn!(
                "Cloud Backup refused to finish {claim:?}, which no longer runs; running {running:?}"
            );
        }

        #[cfg(test)]
        self.transitions.push(OpTransition::FinishAttempt { claim, finish });

        finish
    }

    pub(crate) fn start_standard(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_standard(claim))
    }

    pub(crate) fn start_restore(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_restore(run))
    }

    pub(crate) fn start_restore_all(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_restore_all(run))
    }

    /// Commits the confirmed Turn Off `claim` runs and moves its body to the committed disable
    /// in one step, then publishes
    ///
    /// The record write and the body change are one call, so the record can't move to Disabling
    /// without the body that deletes, and a Turn Off that no longer runs writes nothing
    pub(crate) fn commit_turn_off(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        review: &DisableCommitReview,
        attempt_id: u64,
    ) -> Result<TurnOffCommit, CloudBackupError> {
        #[cfg(test)]
        let before = self.runtime.claim();

        let lifecycle = &self.lifecycle;
        let commit =
            self.runtime.commit_turn_off(claim, || lifecycle.commit_disable(review, attempt_id));

        #[cfg(test)]
        self.record_transition(before);

        self.publish();
        commit
    }

    /// Replaces the Turn Off review an idle configured body holds
    pub(crate) fn set_turn_off_review(&mut self, review: Option<TurnOffReview>) {
        self.mutate(|runtime| runtime.set_turn_off_review(review));
    }

    /// Consent to the review on screen: takes it and starts Turn Off in one step
    pub(crate) fn admit_confirm(
        &mut self,
        review_id: ReviewId,
    ) -> Result<ConfirmedTurnOff, Rejection> {
        self.mutate(|runtime| runtime.admit_confirm(review_id))
    }

    /// Ends the review on screen for the work the button `to` names
    pub(crate) fn leave_review(
        &mut self,
        review_id: ReviewId,
        to: ReviewExit,
    ) -> Result<UserIntent, Rejection> {
        self.mutate(|runtime| runtime.leave_review(review_id, to))
    }

    pub(crate) fn start_finish(
        &mut self,
        authority: DeleteAuthority,
    ) -> Result<AdmittedFinish, Rejection> {
        self.mutate(|runtime| runtime.start_finish(authority))
    }

    pub(crate) fn fail_finish(
        &mut self,
        run: FinishRun,
        reason: CloudBackupTurnOffFailure,
    ) -> Option<FinishFailure> {
        self.mutate(|runtime| runtime.fail_finish(run, reason))
    }

    pub(crate) fn arm_retry_wake(&mut self, at: u64) -> Option<RetryWake> {
        self.mutate(|runtime| runtime.arm_retry_wake(at))
    }

    pub(crate) fn take_retry_wake(&mut self, fired: RetryWake) -> bool {
        self.mutate(|runtime| runtime.take_retry_wake(fired))
    }

    pub(crate) fn hand_off(
        &mut self,
        from: CloudBackupExclusiveOperationClaim,
        to: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), Unstartable> {
        let record = self.lifecycle.record();
        let pending_enable_recovery = self.held_pending_enable_recovery();
        let facts = IdleFacts::of(&record, pending_enable_recovery);
        self.mutate(|runtime| runtime.hand_off(from, to, facts))
    }

    /// Enters or updates the recovery for an interrupted Enable; false when the body outranks it
    pub(crate) fn enter_pending_enable_recovery(
        &mut self,
        recovery: CloudBackupPendingEnableRecovery,
    ) -> bool {
        self.mutate(|runtime| runtime.enter_pending_enable_recovery(recovery))
    }

    pub(crate) fn begin_effect(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        effect: Effect,
    ) -> bool {
        self.mutate(|runtime| runtime.begin_effect(claim, effect))
    }

    pub(crate) fn accepts_completion(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: &'static str,
    ) -> bool {
        self.mutate(|runtime| runtime.accepts_completion(claim, completion))
    }

    pub(crate) fn begin_reset(&mut self, ready: oneshot::Sender<()>) -> ResetStart {
        self.mutate(|runtime| runtime.begin_reset(ready))
    }

    /// Commits Cloud Backup off, then records that the ready reset is clearing the local keys
    ///
    /// The off record comes first because the clear is final even when partial: once it starts,
    /// the record must already say what a later load may give. A failed commit refuses the clear,
    /// so the keys stay with the configuration that needs them
    pub(crate) fn clear_keys_in_reset(&mut self) -> Result<(), KeyClearRefused> {
        let Some(keys) = self.runtime.ready_reset_keys() else {
            return Err(KeyClearRefused::NoReadyReset);
        };

        // the only record write outside `commit`: the resetting body's policy stays
        // `Closed(LocalReset)` whatever the record says, and `finish_reset` publishes the ending one
        self.lifecycle.disable_before_local_key_clear()?;
        *keys = ResetKeys::Cleared;
        Ok(())
    }

    /// Ends a local reset with the body it left; `None` when no reset runs
    ///
    /// After a wipe Cloud Backup is off whatever the record said, and the writer's copy of the
    /// record is the default one. An abandoned reset that kept the local keys goes back to the
    /// owner's record. One that began clearing them committed Cloud Backup off before the clear,
    /// so it ends off
    pub(crate) fn finish_reset(&mut self, outcome: LocalResetOutcome) -> Option<ResetEnd> {
        let keys = self.runtime.reset_keys()?;
        let (after, end) = match (outcome, keys) {
            (LocalResetOutcome::Wiped, _) => {
                self.lifecycle.forget_wiped_record();
                (Runtime::off(), ResetEnd::Wiped)
            }

            // the wipe never ran, so a recovery the reset set aside goes on while its evidence
            // remains; evidence the abandoned wipe did clear ends it
            (LocalResetOutcome::Abandoned, ResetKeys::Kept) => {
                let pending_enable_recovery = self
                    .runtime
                    .recovery_held_by_reset()
                    .cloned()
                    .filter(|_| RustCloudBackupManager::pending_enable_evidence_remains());

                let after =
                    Runtime::idle(IdleFacts::of(&self.lifecycle.record(), pending_enable_recovery));
                (after, ResetEnd::Abandoned(AbandonedReset::Resumed))
            }

            (LocalResetOutcome::Abandoned, ResetKeys::Cleared) => {
                (Runtime::off(), ResetEnd::Abandoned(AbandonedReset::Disabled))
            }
        };

        self.mutate(|runtime| runtime.finish_reset(after));
        Some(end)
    }

    /// Effects don't change the policy, so waiting on them publishes nothing
    #[cfg(test)]
    pub(crate) fn when_effects_settle(&mut self, settled: oneshot::Sender<()>) {
        self.runtime.when_effects_settle(settled);
    }

    /// Restore All's run flags don't change the policy
    pub(crate) fn restore_all_mut(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Option<&mut RestoreRun> {
        self.runtime.restore_all_mut(claim)
    }

    #[cfg(test)]
    pub(crate) fn take_claim(&mut self) -> Option<CloudBackupExclusiveOperationClaim> {
        self.mutate(Runtime::take_claim)
    }
}

/// An operation starting or ending in the runtime, as the harness invariants read it
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpTransition {
    Started(CloudBackupExclusiveOperationClaim),
    Ended(CloudBackupExclusiveOperationClaim),
    /// A finish named this operation, with what it did; it follows the `Ended` it caused
    FinishAttempt {
        claim: CloudBackupExclusiveOperationClaim,
        finish: Finish,
    },
}

/// What the harness invariants read from the owner at a settled boundary
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct OwnerView {
    pub(crate) claim: Option<CloudBackupExclusiveOperationClaim>,
    /// The committed disable the body holds
    pub(crate) committed: Option<CommittedDisable>,
    pub(crate) load_failed: bool,
    pub(crate) published: VersionedPolicy,
    /// The policy the body gives now
    pub(crate) body_policy: WritePolicy,
    /// For an idle body, the policy of the body a rebase on the owner's record would give
    pub(crate) record_policy: Option<WritePolicy>,
    pub(crate) transitions: Vec<OpTransition>,
}

#[cfg(test)]
impl OwnedRuntime {
    fn record_transition(&mut self, before: Option<CloudBackupExclusiveOperationClaim>) {
        let after = self.runtime.claim();
        if before == after {
            return;
        }

        self.transitions.extend(before.map(OpTransition::Ended));
        self.transitions.extend(after.map(OpTransition::Started));
    }

    /// Reads the owner for the harness invariants, taking the transitions recorded since the
    /// last read
    pub(crate) fn take_view(&mut self) -> OwnerView {
        let record = self.lifecycle.record();
        let pending_enable_recovery = self.held_pending_enable_recovery();
        let facts = IdleFacts::of(&record, pending_enable_recovery);

        OwnerView {
            claim: self.runtime.claim(),
            committed: match &self.runtime {
                Runtime::FinishingDisable { committed, .. } => Some(committed.clone()),
                _ => None,
            },
            load_failed: matches!(self.runtime, Runtime::LoadFailed { .. }),
            published: self.published.clone(),
            body_policy: self.runtime.write_policy(self.generation),
            record_policy: self.runtime.idle_policy_on(facts, self.generation),
            transitions: std::mem::take(&mut self.transitions),
        }
    }
}

/// Tests compare the running claim directly, as they do for the bare runtime
#[cfg(test)]
impl PartialEq<Option<CloudBackupExclusiveOperationClaim>> for OwnedRuntime {
    fn eq(&self, other: &Option<CloudBackupExclusiveOperationClaim>) -> bool {
        self.runtime == *other
    }
}
