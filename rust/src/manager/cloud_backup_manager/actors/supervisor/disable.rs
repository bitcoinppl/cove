//! Turn Off: review, consent, drain, recheck, commit, then a committed Turn Off that only goes
//! forward
//!
//! Before the commit nothing is persisted. The review is presentation the configured body holds;
//! the Turn Off operation starts only when the user confirms the review on screen, pauses
//! ordinary writes through the derived policy while it drains, and rechecks the impact on fresh
//! evidence before it commits. Any failure before the commit ends the operation with
//! `DisableNotStarted` and leaves backup on. After the commit the body is `FinishingDisable`,
//! whose runs delete the namespace, clean up this device and write `Disabled`

use super::runtime::ConfiguredOp;
use super::*;
use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::admission::TurnOffAdmission;
use crate::manager::cloud_backup_manager::inventory::InventoryFacts;
use crate::manager::cloud_backup_manager::ops::next_disable_generation;
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommitDisable, CommittedDisable, DisableCommitReview,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableNotStartedReason, CloudBackupDisableScreen, CloudBackupFinishingDisableStage,
    CloudBackupLastOutcome, CloudBackupOutcome, CloudBackupOutcomeTarget, CloudBackupRejection,
    CloudBackupScreenAction, CloudBackupTurnOffFailure,
};
use crate::manager::cloud_backup_manager::turn_off::{
    AutomaticResume, DisableEvidence, DisableImpact, DisableReview, EvidenceUnavailable,
    FinishFailure, FinishStart, ImpactFacts, ReviewId, TurnOffReview,
};

impl CloudBackupSupervisor {
    /// Turn Off tapped: builds the review the dialogs show
    ///
    /// Nothing runs and nothing pauses until the user confirms; a tap while a review is on
    /// screen builds a fresh one, which is the dialogs' Check Again. Admission accepts Turn Off
    /// only in an idle configured body, which is the one body that holds a review
    pub(crate) fn begin_turn_off_review(&mut self, manager: Arc<RustCloudBackupManager>) {
        let review_id = ReviewId::next();
        self.runtime.set_turn_off_review(Some(TurnOffReview::Preparing { review_id }));
        manager.clear_turn_off_outcome();
        self.addr.send_fut_with(move |addr| async move {
            let evidence = manager.gather_disable_evidence().await;
            send!(addr.complete_turn_off_review(review_id, evidence));
        });
    }

    pub async fn complete_turn_off_review(
        &mut self,
        review_id: ReviewId,
        evidence: Result<DisableEvidence, EvidenceUnavailable>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        // an operation start, a body change, Cancel and a newer tap each replace this review
        let preparing = matches!(
            self.runtime.turn_off_review(),
            Some(TurnOffReview::Preparing { review_id: held }) if *held == review_id
        );
        if !preparing {
            debug!("Cloud Backup discarded the stale Turn Off review {review_id:?}");
            return Produces::ok(());
        }

        let next = match evidence {
            Ok(evidence) => {
                let impact = assess(&manager, &evidence);
                Some(TurnOffReview::Reviewing(DisableReview {
                    review_id,
                    namespace_id: evidence.namespace_id,
                    impact,
                    changed_since_confirmation: false,
                }))
            }
            Err(EvidenceUnavailable::Offline) => Some(TurnOffReview::Offline),
            Err(EvidenceUnavailable::Local(message)) => {
                manager.record_turn_off_outcome(CloudBackupOutcome::Failed { message });
                None
            }
        };

        self.runtime.set_turn_off_review(next);
        Produces::ok(())
    }

    /// The user consented to the review `review_id`: Turn Off starts and drains
    ///
    /// Admission accepted it only while that review is on screen, so the review is taken here
    /// and travels with the operation to its recheck
    pub(crate) fn confirm_disable(&mut self, manager: &RustCloudBackupManager, review_id: u64) {
        let Some(review) = self.runtime.take_reviewed(ReviewId::from(review_id)) else {
            warn!("Cloud Backup confirmation found no review {review_id}");
            return;
        };

        let Some(claim) =
            self.begin_exclusive_operation(manager, CloudBackupExclusiveOperation::Disable)
        else {
            warn!("Cloud Backup could not start the confirmed Turn Off");
            return;
        };

        // the running Turn Off already pauses ordinary writes through the derived policy; this
        // barrier waits out the write in flight and those queued before it, so an upload that
        // already started can't recreate remote data after the delete. The reply comes back as
        // a message, so the supervisor never waits on the write supervisor
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await;
            send!(addr.complete_turn_off_drain(claim, review, drained));
        });
    }

    pub async fn complete_turn_off_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        review: DisableReview,
        drained: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };

        if let Err(error) = drained {
            error.logged_reader_message("Failed to drain cloud backup writes before Turn Off");
            let reason = CloudBackupDisableNotStartedReason::WritesDidNotDrain;
            self.end_turn_off_not_started(
                &manager,
                claim,
                CloudBackupOutcome::DisableNotStarted { reason },
            );
            return Produces::ok(());
        }

        self.addr.send_fut_with(move |addr| async move {
            let evidence = manager.gather_disable_evidence().await;
            send!(addr.complete_turn_off_recheck(claim, review, evidence));
        });

        Produces::ok(())
    }

    /// Consent is bound to evidence: the commit runs only when the fresh impact puts nothing
    /// more at risk than the review the user confirmed
    pub async fn complete_turn_off_recheck(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        review: DisableReview,
        evidence: Result<DisableEvidence, EvidenceUnavailable>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };

        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(EvidenceUnavailable::Offline) => {
                let reason = CloudBackupDisableNotStartedReason::Offline;
                let outcome = CloudBackupOutcome::DisableNotStarted { reason };
                self.end_turn_off_not_started(&manager, claim, outcome);
                return Produces::ok(());
            }
            Err(EvidenceUnavailable::Local(message)) => {
                self.end_turn_off_not_started(
                    &manager,
                    claim,
                    CloudBackupOutcome::Failed { message },
                );
                return Produces::ok(());
            }
        };

        // the delete can't run without the provider, so these end before the commit
        let unreachable = match evidence.listing {
            Err(CloudStorageIssue::Offline) => Some(CloudBackupDisableNotStartedReason::Offline),
            Err(CloudStorageIssue::AuthorizationRequired) => {
                Some(CloudBackupDisableNotStartedReason::AuthorizationRequired)
            }
            _ => None,
        };
        if let Some(reason) = unreachable {
            let outcome = CloudBackupOutcome::DisableNotStarted { reason };
            self.end_turn_off_not_started(&manager, claim, outcome);
            return Produces::ok(());
        }

        if evidence.namespace_id != review.namespace_id {
            let reason = CloudBackupRejection::ReviewStale;
            self.end_turn_off_not_started(&manager, claim, CloudBackupOutcome::Rejected { reason });
            return Produces::ok(());
        }

        let rechecked = assess(&manager, &evidence);
        if rechecked.grew_since(&review.impact) {
            info!("Cloud Backup changed since the Turn Off review, so it asks again");
            self.end_turn_off(&manager, claim);
            self.runtime.set_turn_off_review(Some(TurnOffReview::Reviewing(DisableReview {
                review_id: ReviewId::next(),
                namespace_id: review.namespace_id,
                impact: rechecked,
                changed_since_confirmation: true,
            })));
            return Produces::ok(());
        }

        self.commit_confirmed_turn_off(manager, claim, review).await
    }

    /// The commit point: from here on Turn Off only goes forward
    async fn commit_confirmed_turn_off(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        review: DisableReview,
    ) -> ActorResult<()> {
        let commit_review = DisableCommitReview { namespace_id: review.namespace_id };
        let attempt_id = next_disable_generation();
        let committed =
            self.runtime.commit(|lifecycle| lifecycle.commit_disable(&commit_review, attempt_id));

        match committed {
            Ok(CommitDisable::Committed(committed)) => {
                self.runtime.commit_turn_off(claim, committed.clone());
                self.finish_committed(&manager, claim, committed);
            }
            // the record moved on without this device's backup, so there is nothing to turn off
            Ok(CommitDisable::NotConfigured) => self.end_turn_off(&manager, claim),
            Ok(CommitDisable::StaleReview) => {
                let reason = CloudBackupRejection::ReviewStale;
                self.end_turn_off_not_started(
                    &manager,
                    claim,
                    CloudBackupOutcome::Rejected { reason },
                );
            }
            Err(error) => {
                error.logged_reader_message("Failed to commit cloud backup Turn Off");
                let reason = CloudBackupDisableNotStartedReason::SaveFailed;
                let outcome = CloudBackupOutcome::DisableNotStarted { reason };
                self.end_turn_off_not_started(&manager, claim, outcome);
            }
        }

        Produces::ok(())
    }

    /// Ends Turn Off before the review is confirmed, or before the confirmed Turn Off commits
    ///
    /// Before the commit nothing was persisted, so ending the operation is all a cancel needs:
    /// its later completions are stale, and the derived policy runs ordinary writes again
    pub(crate) fn cancel_disable(&mut self, manager: &RustCloudBackupManager) {
        match self.runtime.claim() {
            Some(claim)
                if matches!(
                    &*self.runtime,
                    Runtime::Configured { op: Some(ConfiguredOp::TurnOff(_)), .. }
                ) =>
            {
                info!("Cloud Backup Turn Off cancelled before it committed");
                self.end_turn_off(manager, claim);
            }
            _ => {
                self.runtime.set_turn_off_review(None);
            }
        }
    }

    /// Leaving the detail screen ends a review left on screen; a confirmed Turn Off keeps going
    pub(crate) fn leave_turn_off_review(&mut self) {
        if self.runtime.turn_off_review().is_some() {
            self.runtime.set_turn_off_review(None);
        }
    }

    fn end_turn_off(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
    }

    fn end_turn_off_not_started(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        outcome: CloudBackupOutcome,
    ) {
        warn!("Cloud Backup Turn Off ended before it deleted anything: {outcome:?}");
        self.end_turn_off(manager, claim);
        manager.record_turn_off_outcome(outcome);
    }

    /// A committed Turn Off resumes silently at launch, and on reconnect or a cloud change once
    /// the last failure's `retry_after` has passed
    pub async fn resume_disable_automatically(
        &mut self,
        trigger: AutomaticResume,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Runtime::FinishingDisable { committed, op: None, .. } = &*self.runtime else {
            return Produces::ok(());
        };

        let now = cove_util::time::unix_timestamp_secs_or_zero();
        let retry_after = match trigger {
            AutomaticResume::Launch => None,
            AutomaticResume::CloudReachable => committed.retry_after,
        };
        if let Some(retry_after) = retry_after.filter(|after| now < *after) {
            debug!("Cloud Backup Turn Off resumes after {retry_after}, not at {now}");
            return Produces::ok(());
        }

        self.begin_finish(&manager, FinishStart::Automatic);
        Produces::ok(())
    }

    /// Starts a finishing run of the committed Turn Off: Try Again, an automatic resume, or
    /// the local-only finish that leaves the remote backups
    pub(crate) fn begin_finish(&mut self, manager: &RustCloudBackupManager, start: FinishStart) {
        let Runtime::FinishingDisable { committed, op: None, .. } = &*self.runtime else {
            warn!("Cloud Backup has no committed Turn Off to finish with {start:?}");
            return;
        };

        let committed = committed.clone();
        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            OperationId::next(),
        );
        if let Err(unstartable) = self.runtime.start_finish(claim, start) {
            warn!("Cloud Backup could not start {unstartable:?}");
            return;
        }

        manager.project_exclusive_operation_started(claim);

        // writes queued before this run, such as a relaunch's, finish before the delete
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await;
            send!(addr.complete_finish_drain(claim, committed, drained));
        });
    }

    pub async fn complete_finish_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        committed: CommittedDisable,
        drained: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };

        let Some(committed) = manager.committed_disable_if_current(&committed) else {
            // the record moved on, so this run has nothing left to finish
            self.end_turn_off(&manager, claim);
            return Produces::ok(());
        };

        if let Err(error) = drained {
            self.fail_finish(&manager, claim, committed, error, "drain writes before Turn Off");
            return Produces::ok(());
        }

        self.finish_committed(&manager, claim, committed);
        Produces::ok(())
    }

    /// Clears the runtime caches of a backup that is going away, then deletes it, or only
    /// cleans up this device for the local-only finish
    fn finish_committed(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        committed: CommittedDisable,
    ) {
        self.drain_disable_runtime(manager);
        match self.runtime.finish_start(claim) {
            Some(FinishStart::LocalOnly) => {
                info!("Cloud Backup stops on this device and leaves its remote backups");
                self.schedule_disable_local_cleanup(claim, committed);
            }
            Some(FinishStart::Commit | FinishStart::Automatic | FinishStart::TryAgain) => {
                self.schedule_disable_namespace_delete(claim, committed);
            }
            None => {}
        }
    }

    fn drain_disable_runtime(&mut self, manager: &RustCloudBackupManager) {
        self.clear_worker_runtime_state();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_state(SyncState::Idle);
    }

    fn schedule_disable_namespace_delete(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
    ) {
        let write = self.write.clone();
        self.addr.send_fut_with(move |addr| async move {
            let result = CloudBackupWriteClient::for_operation(write, claim)
                .delete_committed_namespace(
                    CloudStorage::global_explicit_client(),
                    disabling.namespace_id.clone(),
                )
                .await;
            send!(addr.complete_disable_namespace_delete(claim, disabling, result));
        });
    }

    pub async fn complete_disable_namespace_delete(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };

        match result {
            Ok(()) | Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                self.schedule_disable_local_cleanup(claim, disabling);
            }
            Err(CloudBackupError::CloudStorage(error)) => {
                let error =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error);
                self.fail_finish(&manager, claim, disabling, error, "delete the backup namespace");
            }
            Err(error) => {
                self.fail_finish(&manager, claim, disabling, error, "delete the backup namespace");
            }
        }

        Produces::ok(())
    }

    /// Starts Turn Off's local cleanup as the operation's `Effecting` stage
    ///
    /// The keychain clear can't be taken back once it starts, so a reset that begins meanwhile
    /// waits for `complete_disable_local_cleanup` instead of wiping beside it
    fn schedule_disable_local_cleanup(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
    ) {
        if !self.runtime.begin_effect(claim, Effect::LocalCleanup) {
            return;
        }

        let owner = OwnerToken::mint();
        self.addr.send_fut_with(move |addr| async move {
            // a platform keychain call blocks its thread, so it runs off the runtime
            let result = cove_tokio::unblock::run_blocking(move || {
                RustCloudBackupManager::run_disable_local_cleanup(&LocalStateOwner::Lifecycle(
                    &owner,
                ))
            })
            .await;
            send!(addr.complete_disable_local_cleanup(claim, disabling, result));
        });
    }

    pub async fn complete_disable_local_cleanup(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };

        if let Err(error) = result {
            self.fail_finish(&manager, claim, disabling, error, "clean up Cloud Backup locally");
            return Produces::ok(());
        }

        manager.apply_disable_local_cleanup();

        if let Err(error) = self
            .runtime
            .commit(|lifecycle| manager.persist_disabled_after_remote_delete(lifecycle, &disabling))
        {
            self.fail_finish(&manager, claim, disabling, error, "save Cloud Backup as off");
            return Produces::ok(());
        }

        info!("Cloud Backup is off");
        self.end_turn_off(&manager, claim);
        Produces::ok(())
    }

    /// A finishing run stopped; the committed Turn Off waits for the next resume
    ///
    /// The failure is saved with its `retry_after`, which gates the automatic resume; Try Again
    /// stays available whenever no run is going
    fn fail_finish(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
        error: CloudBackupError,
        step: &str,
    ) {
        let reason = CloudBackupTurnOffFailure::from(CloudStorageIssue::from(&error));
        let message =
            error.logged_reader_message(format!("Cloud Backup Turn Off failed to {step}"));

        let persisted = self
            .runtime
            .commit(|lifecycle| manager.persist_disabling_failure(lifecycle, &disabling, message));
        if let Err(error) = persisted {
            error.logged_reader_message("Failed to save the Cloud Backup Turn Off failure");
        }

        self.runtime.fail_finish(claim, reason);
        manager.project_exclusive_operation_finished(claim);
    }
}

/// The impact of `evidence`, classified by the inventory's facts as they stand now
fn assess(manager: &RustCloudBackupManager, evidence: &DisableEvidence) -> DisableImpact {
    DisableImpact::assess(evidence, &impact_facts(manager))
}

/// The inventory's facts a Turn Off impact reads, as they stand now
pub(crate) fn impact_facts(manager: &RustCloudBackupManager) -> ImpactFacts {
    // the reducer's lock is taken first, as every reader holding both does
    let reducer = manager.state.read();
    let inputs = InventoryFacts::new(&manager.inventory.read(), &reducer).view().impact_inputs();

    ImpactFacts::from(inputs)
}

impl Runtime {
    /// The Turn Off flow a configured screen shows
    pub(crate) fn disable_screen(&self) -> CloudBackupDisableScreen {
        match self {
            Self::Configured { op: Some(ConfiguredOp::TurnOff(_)), .. } => {
                CloudBackupDisableScreen::Draining
            }
            Self::Configured { op: None, review: Some(review) } => match review {
                TurnOffReview::Preparing { .. } => CloudBackupDisableScreen::Preparing,
                TurnOffReview::Offline => CloudBackupDisableScreen::Offline,
                TurnOffReview::Reviewing(review) => {
                    CloudBackupDisableScreen::Reviewing(review.screen())
                }
            },
            _ => CloudBackupDisableScreen::Idle,
        }
    }

    /// Where a committed Turn Off stands, for the Finishing Turn Off screen
    ///
    /// A committed Turn Off that hasn't failed in this runtime is about to resume, so it shows
    /// as resuming; a failure saved by an earlier launch has no typed reason
    pub(crate) fn finishing_disable_stage(&self) -> CloudBackupFinishingDisableStage {
        match self {
            Self::FinishingDisable { op: Some(_), .. }
            | Self::FinishingDisable {
                op: None,
                failure: None,
                committed: CommittedDisable { last_error: None, .. },
            } => CloudBackupFinishingDisableStage::Resuming,
            Self::FinishingDisable { op: None, failure: Some(failure), .. } => {
                CloudBackupFinishingDisableStage::Failed { reason: failure.reason() }
            }
            _ => CloudBackupFinishingDisableStage::Failed {
                reason: CloudBackupTurnOffFailure::Other,
            },
        }
    }

    /// Where Turn Off stands outside a running operation, as admission reads it
    pub(crate) fn turn_off_admission(&self) -> TurnOffAdmission {
        match self {
            Self::Configured { op: None, review: Some(review) } => match review {
                TurnOffReview::Preparing { .. } => TurnOffAdmission::Preparing,
                TurnOffReview::Offline => TurnOffAdmission::Offline,
                TurnOffReview::Reviewing(review) => {
                    TurnOffAdmission::Reviewing { review_id: review.review_id.get() }
                }
            },
            Self::FinishingDisable { failure: Some(FinishFailure::TryAgainFailed(_)), .. } => {
                TurnOffAdmission::TryAgainFailed
            }
            _ => TurnOffAdmission::None,
        }
    }
}

impl RustCloudBackupManager {
    /// Records how a Turn Off that never committed ended, next to the Turn Off control
    pub(crate) fn record_turn_off_outcome(&self, outcome: CloudBackupOutcome) {
        let target = CloudBackupOutcomeTarget::Action(CloudBackupScreenAction::TurnOff);
        self.record_last_outcome(CloudBackupLastOutcome { target, outcome });
    }

    /// A new Turn Off replaces what the last one left next to the control
    fn clear_turn_off_outcome(&self) {
        let target = CloudBackupOutcomeTarget::Action(CloudBackupScreenAction::TurnOff);
        if self.state.read().last_outcome().is_some_and(|last| last.target == target) {
            self.dismiss_last_outcome();
        }
    }
}
