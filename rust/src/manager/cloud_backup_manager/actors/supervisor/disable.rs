//! Turn Off: review, consent, drain, recheck, commit, then a committed Turn Off that only goes
//! forward
//!
//! Before the commit nothing is persisted. The review is presentation the configured body holds;
//! the Turn Off operation starts only when the user confirms the review on screen, pauses
//! ordinary writes through the derived policy while it drains, and rechecks the impact on fresh
//! evidence before it commits. Any failure before the commit ends the operation with
//! `DisableNotStarted` and leaves backup on. After the commit the body is `FinishingDisable`,
//! whose runs delete the namespace, clean up this device and write `Disabled`

use super::runtime::{ConfiguredBody, ConfiguredOp};
use super::*;
use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::admission::TurnOffAdmission;
use crate::manager::cloud_backup_manager::ops::{
    DISABLE_EVIDENCE_DEADLINE, next_disable_generation,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommittedDisable, DisableCommitReview,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableNotStartedReason, CloudBackupDisableScreen, CloudBackupFinishingDisableStage,
    CloudBackupOutcome, CloudBackupRejection, CloudBackupScreenAction, CloudBackupTurnOffFailure,
};
use crate::manager::cloud_backup_manager::turn_off::{
    AdmittedFinish, AutomaticResume, ConfirmedTurnOff, DeleteAuthority, DisableEvidence,
    DisableImpact, DisableReview, EvidenceUnavailable, FinishFailure, FinishRun, FinishStart,
    ImpactFacts, ListingEvidence, PreparingStage, RetryWake, ReviewId, TurnOffCommit,
    TurnOffReview,
};
use crate::manager::cloud_backup_manager::{ClassificationRequestId, InventoryInput};

impl CloudBackupSupervisor {
    /// Turn Off tapped: builds the review the dialogs show
    ///
    /// Nothing runs and nothing pauses until the user confirms. A tap while a review is on
    /// screen is the dialogs' Check Again: it first asks the inventory owner for a fresh
    /// classification, joining a refresh already running, so a provider that answers again
    /// replaces the facts the last review was missing. Admission accepts Turn Off only in an
    /// idle configured body, which is the one body that holds a review
    pub(crate) fn begin_turn_off_review(&mut self, manager: Arc<RustCloudBackupManager>) {
        let review_id = ReviewId::next();
        let check_again = self.runtime.turn_off_review().is_some();
        if !check_again {
            self.gather_turn_off_review(manager, review_id);
            return;
        }

        // a review lives only on the open detail screen, so a closed refresh owner runs no
        // refresh to wait for, and the review goes on with the inventory as it stands
        let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Tap);
        if plan == DetailRefreshPlan::Ignored {
            self.gather_turn_off_review(manager, review_id);
            return;
        }

        let stage = PreparingStage::Classifying;
        self.runtime.set_turn_off_review(Some(TurnOffReview::Preparing { review_id, stage }));
        manager.apply_inventory(InventoryInput::ClassificationRequested(review_id.into()));
        self.handle_detail_refresh_plan(manager, plan);

        // the classification can fail to settle, such as when a newer result displaces the
        // refresh; the review then goes on at the deadline with the inventory as it stands
        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep(DISABLE_EVIDENCE_DEADLINE).await;
            send!(addr.turn_off_review_classified(review_id));
        });
    }

    /// The inventory settled the classification a Check Again asked for
    pub async fn inventory_classification_settled(
        &mut self,
        request: ClassificationRequestId,
    ) -> ActorResult<()> {
        let review_id = match self.runtime.turn_off_review() {
            Some(TurnOffReview::Preparing { review_id, stage: PreparingStage::Classifying })
                if ClassificationRequestId::from(*review_id) == request =>
            {
                *review_id
            }
            _ => return Produces::ok(()),
        };

        self.turn_off_review_classified(review_id).await
    }

    /// Check Again's classification settled or its deadline passed: the review gathers its
    /// evidence against the inventory as it stands now
    ///
    /// Only the review that asked moves on, so the later of the two, or one for a review that
    /// was replaced, is discarded
    pub async fn turn_off_review_classified(&mut self, review_id: ReviewId) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let classifying = matches!(
            self.runtime.turn_off_review(),
            Some(TurnOffReview::Preparing { review_id: held, stage: PreparingStage::Classifying })
                if *held == review_id
        );
        if classifying {
            self.gather_turn_off_review(manager, review_id);
        }

        Produces::ok(())
    }

    fn gather_turn_off_review(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        review_id: ReviewId,
    ) {
        let stage = PreparingStage::Gathering;
        self.runtime.set_turn_off_review(Some(TurnOffReview::Preparing { review_id, stage }));
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
            Some(TurnOffReview::Preparing { review_id: held, stage: PreparingStage::Gathering })
                if *held == review_id
        );
        if !preparing {
            debug!("Cloud Backup discarded the stale Turn Off review {review_id:?}");
            return Produces::ok(());
        }

        let next = match evidence {
            Ok(evidence) => {
                let impact = assess(&manager, &evidence);
                Some(TurnOffReview::Reviewing(Box::new(DisableReview {
                    review_id,
                    namespace_id: evidence.namespace_id,
                    impact,
                    changed_since_confirmation: false,
                })))
            }
            Err(EvidenceUnavailable::Offline) => Some(TurnOffReview::Offline),
            Err(EvidenceUnavailable::Local(message)) => {
                self.record_turn_off_outcome(&manager, CloudBackupOutcome::Failed { message });
                None
            }
        };

        self.runtime.set_turn_off_review(next);
        Produces::ok(())
    }

    /// The user consented to the review on screen: the Turn Off admission started drains
    ///
    /// Admission took the review out of the body as it allocated the operation, so the review
    /// travels with the operation to its recheck
    pub(crate) fn confirm_disable(
        &mut self,
        manager: &RustCloudBackupManager,
        confirmed: ConfirmedTurnOff,
    ) {
        let ConfirmedTurnOff { claim, review } = confirmed;
        manager.project_exclusive_operation_started(claim);

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
            ListingEvidence::Failed(CloudStorageIssue::Offline) => {
                Some(CloudBackupDisableNotStartedReason::Offline)
            }
            ListingEvidence::Failed(CloudStorageIssue::AuthorizationRequired) => {
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
            self.runtime.set_turn_off_review(Some(TurnOffReview::Reviewing(Box::new(
                DisableReview {
                    review_id: ReviewId::next(),
                    namespace_id: review.namespace_id,
                    impact: rechecked,
                    changed_since_confirmation: true,
                },
            ))));
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

        match self.runtime.commit_turn_off(claim, &commit_review, attempt_id) {
            Ok(TurnOffCommit::Committed { run, committed }) => {
                self.finish_committed(&manager, run, committed);
            }
            // a completion for a Turn Off that ended wrote nothing, and its end already
            // published; the recheck found this claim current in this same handler, so this is
            // only reachable if that changes
            Ok(TurnOffCommit::NotCurrent) => {
                debug!("Cloud Backup dropped a Turn Off commit whose operation ended");
            }
            // the record moved on without this device's backup, so there is nothing to turn off
            Ok(TurnOffCommit::NotConfigured) => self.end_turn_off(&manager, claim),
            Ok(TurnOffCommit::StaleReview) => {
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
                    Runtime::Configured(ConfiguredBody::Running(ConfiguredOp::TurnOff(_)))
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
        self.runtime.finish(claim);
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
        self.record_turn_off_outcome(manager, outcome);
    }

    /// Shows how a Turn Off that never committed ended, next to the Turn Off control
    fn record_turn_off_outcome(
        &mut self,
        manager: &RustCloudBackupManager,
        outcome: CloudBackupOutcome,
    ) {
        if self.screen_outcome.action_ended(CloudBackupScreenAction::TurnOff, outcome) {
            manager.request_screen_publish();
        }
    }

    /// A committed Turn Off resumes silently at launch, and on reconnect or a cloud change once
    /// the last failure's `retry_after` has passed
    ///
    /// A reconnect before `retry_after` arms the one retry wake instead, so the resume it asked
    /// for still runs at the deadline when nothing else wakes the supervisor
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
            self.arm_retry_wake(retry_after, now);
            return Produces::ok(());
        }

        match self.runtime.start_finish(DeleteAuthority::Automatic) {
            Ok(finish) => self.begin_finish(&manager, finish),
            Err(rejection) => warn!("Cloud Backup could not resume Turn Off: {rejection:?}"),
        }

        Produces::ok(())
    }

    /// Schedules the automatic retry for `at`, the committed Turn Off's `retry_after`
    ///
    /// A timer, because a reconnect that came before the deadline is the last event that may
    /// arrive: without it the committed Turn Off would wait for the next launch. It is bounded:
    /// the body keeps one wake for its attempt and deadline, a finishing run or a new body ends
    /// it, and a timer whose wake is no longer armed does nothing
    fn arm_retry_wake(&mut self, at: u64, now: u64) {
        let Some(wake) = self.runtime.arm_retry_wake(at) else { return };

        // whole seconds from the floored clock always reach `at`
        let delay = std::time::Duration::from_secs(at.saturating_sub(now));
        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep(delay).await;
            send!(addr.retry_wake_fired(wake));
        });
    }

    pub async fn retry_wake_fired(&mut self, wake: RetryWake) -> ActorResult<()> {
        if !self.runtime.take_retry_wake(wake) {
            debug!("Cloud Backup dropped the Turn Off retry {wake:?}, which no longer applies");
            return Produces::ok(());
        }

        self.resume_disable_automatically(AutomaticResume::CloudReachable).await
    }

    /// Starts the finishing run admission or an automatic resume allocated: Try Again, Reconnect,
    /// an automatic resume, or the local-only finish that leaves the remote backups
    pub(crate) fn begin_finish(
        &mut self,
        manager: &RustCloudBackupManager,
        finish: AdmittedFinish,
    ) {
        let AdmittedFinish { run, committed } = finish;
        manager.project_exclusive_operation_started(run.claim);

        // writes queued before this run, such as a relaunch's, finish before the delete
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), run.claim);
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await;
            send!(addr.complete_finish_drain(run, committed, drained));
        });
    }

    pub async fn complete_finish_drain(
        &mut self,
        run: FinishRun,
        committed: CommittedDisable,
        drained: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else { return Produces::ok(()) };

        let Some(committed) = manager.committed_disable_if_current(&committed) else {
            // the record moved on, so this run has nothing left to finish
            self.end_turn_off(&manager, run.claim);
            return Produces::ok(());
        };

        if let Err(error) = drained {
            self.fail_finish(&manager, run, committed, error, "drain writes before Turn Off");
            return Produces::ok(());
        }

        self.finish_committed(&manager, run, committed);
        Produces::ok(())
    }

    /// Clears the runtime caches of a backup that is going away, then deletes it with the
    /// provider access its start allows, or only cleans up this device for the local-only finish
    fn finish_committed(
        &mut self,
        manager: &RustCloudBackupManager,
        run: FinishRun,
        committed: CommittedDisable,
    ) {
        self.drain_disable_runtime(manager);
        match run.start {
            FinishStart::LocalOnly => {
                info!("Cloud Backup stops on this device and leaves its remote backups");
                self.schedule_disable_local_cleanup(run, committed);
            }
            FinishStart::Delete(authority) => {
                self.schedule_disable_namespace_delete(run, authority, committed);
            }
        }
    }

    fn drain_disable_runtime(&mut self, manager: &RustCloudBackupManager) {
        self.clear_worker_runtime_state();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_state(SyncState::Idle);
    }

    fn schedule_disable_namespace_delete(
        &self,
        run: FinishRun,
        authority: DeleteAuthority,
        disabling: CommittedDisable,
    ) {
        let write = self.write.clone();
        self.addr.send_fut_with(move |addr| async move {
            let result = CloudBackupWriteClient::for_operation(write, run.claim)
                .delete_committed_namespace(
                    authority.cloud_client(),
                    disabling.namespace_id.clone(),
                )
                .await;
            send!(addr.complete_disable_namespace_delete(run, disabling, result));
        });
    }

    pub async fn complete_disable_namespace_delete(
        &mut self,
        run: FinishRun,
        disabling: CommittedDisable,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else { return Produces::ok(()) };

        match result {
            Ok(()) | Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                self.schedule_disable_local_cleanup(run, disabling);
            }
            Err(CloudBackupError::CloudStorage(error)) => {
                let error =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error);
                self.fail_finish(&manager, run, disabling, error, "delete the backup namespace");
            }
            Err(error) => {
                self.fail_finish(&manager, run, disabling, error, "delete the backup namespace");
            }
        }

        Produces::ok(())
    }

    /// Starts Turn Off's local cleanup as the operation's `Effecting` stage
    ///
    /// The keychain clear can't be taken back once it starts, so a reset that begins meanwhile
    /// waits for `complete_disable_local_cleanup` instead of wiping beside it
    fn schedule_disable_local_cleanup(&mut self, run: FinishRun, disabling: CommittedDisable) {
        if !self.runtime.begin_effect(run.claim, Effect::LocalCleanup) {
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
            send!(addr.complete_disable_local_cleanup(run, disabling, result));
        });
    }

    pub async fn complete_disable_local_cleanup(
        &mut self,
        run: FinishRun,
        disabling: CommittedDisable,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else { return Produces::ok(()) };

        if let Err(error) = result {
            self.fail_finish(&manager, run, disabling, error, "clean up Cloud Backup locally");
            return Produces::ok(());
        }

        manager.apply_disable_local_cleanup();

        if let Err(error) = self
            .runtime
            .commit(|lifecycle| manager.persist_disabled_after_remote_delete(lifecycle, &disabling))
        {
            self.fail_finish(&manager, run, disabling, error, "save Cloud Backup as off");
            return Produces::ok(());
        }

        info!("Cloud Backup is off");
        self.end_turn_off(&manager, run.claim);
        Produces::ok(())
    }

    /// A finishing run stopped; the committed Turn Off waits for the next resume
    ///
    /// The failure is saved with its `retry_after`, which gates the automatic resume; Try Again
    /// stays available whenever no run is going
    fn fail_finish(
        &mut self,
        manager: &RustCloudBackupManager,
        run: FinishRun,
        disabling: CommittedDisable,
        error: CloudBackupError,
        step: &str,
    ) {
        let reason = CloudBackupTurnOffFailure::from(CloudStorageIssue::from(&error));
        let message =
            error.logged_reader_message(format!("Cloud Backup Turn Off failed to {step}"));

        let failure = self.runtime.fail_finish(run, reason);
        manager.project_exclusive_operation_finished(run.claim);

        // a failed Try Again is saved with the failure, so Stop Using stays offered after a
        // relaunch
        let try_again_failed = failure.is_some_and(FinishFailure::try_again_failed);
        let persisted = self.runtime.commit(|lifecycle| {
            manager.persist_disabling_failure(lifecycle, &disabling, message, try_again_failed)
        });
        if let Err(error) = persisted {
            error.logged_reader_message("Failed to save the Cloud Backup Turn Off failure");
        }
    }
}

/// The impact of `evidence`, classified by the inventory's facts as they stand now
fn assess(manager: &RustCloudBackupManager, evidence: &DisableEvidence) -> DisableImpact {
    DisableImpact::assess(evidence, &impact_facts(manager))
}

/// The inventory's facts a Turn Off impact reads, as they stand now
///
/// The impact pairs them with the evidence's own read of this device's wallets, which the
/// facts' local wallet epoch tells stale from current
pub(crate) fn impact_facts(manager: &RustCloudBackupManager) -> ImpactFacts {
    ImpactFacts::from(manager.inventory_facts().view().impact_inputs())
}

impl Runtime {
    /// The Turn Off flow a configured screen shows
    pub(crate) fn disable_screen(&self) -> CloudBackupDisableScreen {
        match self {
            Self::Configured(ConfiguredBody::Running(ConfiguredOp::TurnOff(_))) => {
                CloudBackupDisableScreen::Draining
            }
            Self::Configured(ConfiguredBody::Idle { review: Some(review) }) => match review {
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
                ..
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
            Self::Configured(ConfiguredBody::Idle { review: Some(review) }) => match review {
                TurnOffReview::Preparing { .. } => TurnOffAdmission::Preparing,
                TurnOffReview::Offline => TurnOffAdmission::Offline,
                TurnOffReview::Reviewing(review) => TurnOffAdmission::Reviewing {
                    review_id: review.review_id.get(),
                    restore_first: review.impact.restore_target(),
                },
            },
            Self::FinishingDisable { failure: Some(failure), .. } => {
                TurnOffAdmission::FinishFailed(*failure)
            }
            _ => TurnOffAdmission::None,
        }
    }
}
