use super::*;
use crate::manager::cloud_backup_manager::ops::DisableAttempt;
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommitDisable, CommittedDisable, LifecycleWriter,
};

/// Shown when the namespace changed between the blocker check and the commit
const CLOUD_BACKUP_DISABLE_STALE_REVIEW_MESSAGE: &str =
    "Cloud Backup changed before it could be turned off. Nothing was deleted. Try again.";

impl CloudBackupSupervisor {
    pub(crate) fn begin_disable_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        else {
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_disable_cloud_backup().await;
            send!(addr.complete_disable_preparation(claim, result));
        });
    }

    pub async fn complete_disable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupDisablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let attempt = match result {
            Ok(CloudBackupDisablePreparation::AlreadyDisabled) => {
                self.finish_disable_operation(&manager, claim);
                return Produces::ok(());
            }
            Ok(CloudBackupDisablePreparation::Ready(attempt)) => *attempt,
            Err(error) => {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.logged_reader_message("Failed to prepare cloud backup disable"),
                );
                return Produces::ok(());
            }
        };

        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: attempt.attempt_id() };

        // the active write lane drains before the namespace delete so an upload that already
        // started cannot recreate remote data after disable deletes it; the supervisor never
        // waits on the write supervisor, which replies with `complete_disable_write_drain`
        self.pending_disable_write_drain =
            Some(PendingDisableWriteDrain { claim, blocker, attempt });
        send!(self.write.block_until_drained(blocker, self.addr.clone(), claim));

        Produces::ok(())
    }

    pub async fn complete_disable_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        blocker: CloudBackupWriteBlocker,
    ) -> ActorResult<()> {
        if !self.runtime.accepts_completion(claim, "disable write drain") {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.runtime.clear();
            self.pending_disable_write_drain = None;
            return Produces::ok(());
        };
        let Some(pending) = self.pending_disable_write_drain.take() else {
            return Produces::ok(());
        };
        if pending.claim != claim || pending.blocker != blocker {
            self.pending_disable_write_drain = Some(pending);
            return Produces::ok(());
        }

        let Some(attempt) = manager.disable_attempt_if_current(&pending.attempt) else {
            self.release_uncommitted_disable(&manager, claim, &pending.attempt).await;
            return Produces::ok(());
        };

        manager.apply_disable_outcome(CloudBackupDisableOutcome::Started);
        if let Err(error) = self.drain_disable_runtime(&manager).await {
            let message = error
                .logged_reader_message("Failed to clear cloud backup runtime state before disable");
            match attempt {
                DisableAttempt::Uncommitted { .. } => {
                    self.fail_uncommitted_disable(&manager, claim, &attempt, message).await;
                }
                DisableAttempt::Committed(committed) => {
                    self.fail_disable_after_namespace_delete_started(
                        &manager, claim, committed, message,
                    );
                }
            }
            return Produces::ok(());
        }

        if !self.runtime.accepts_completion(claim, "disable write drain") {
            return Produces::ok(());
        }

        match attempt {
            DisableAttempt::Committed(committed) => {
                self.schedule_disable_namespace_delete(claim, committed);
            }
            DisableAttempt::Uncommitted { .. } => {
                self.schedule_disable_blocker_check(manager, claim, attempt);
            }
        }

        Produces::ok(())
    }

    async fn drain_disable_runtime(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        self.pending_enable_session = None;
        self.detail_workflow.clear_pending_completion();
        self.detail_workflow.clear_authorization();
        call!(self.sync_health.clear_upload_runtime_state())
            .await
            .map_err(CloudBackupError::internal)?;
        call!(self.uploads.clear_upload_runtime_state())
            .await
            .map_err(CloudBackupError::internal)?;

        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_state(SyncState::Idle);
        Ok(())
    }

    fn schedule_disable_blocker_check(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: DisableAttempt,
    ) {
        let DisableAttempt::Uncommitted { review, .. } = &attempt else { return };
        let namespace_id = review.namespace_id.clone();
        self.addr.send_fut_with(move |addr| async move {
            let cloud = CloudStorage::global_explicit_client();
            let result = manager.check_disable_blockers(&cloud, &namespace_id).await;
            send!(addr.complete_disable_blocker_check(claim, attempt, result));
        });
    }

    pub async fn complete_disable_blocker_check(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: DisableAttempt,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };
        let Some(DisableAttempt::Uncommitted { review, attempt_id }) =
            manager.disable_attempt_if_current(&attempt)
        else {
            self.release_uncommitted_disable(&manager, claim, &attempt).await;
            return Produces::ok(());
        };

        if let Err(error) = result {
            let message = error.logged_reader_message("Cloud backup disable blocker check failed");
            self.fail_uncommitted_disable(&manager, claim, &attempt, message).await;
            return Produces::ok(());
        }

        // the commit point: from here on the disable only goes forward
        match manager.commit_disable_attempt(&self.lifecycle, &review, attempt_id) {
            Ok(CommitDisable::Committed(committed)) => {
                self.runtime.commit_turn_off(claim, committed.clone());
                self.schedule_disable_namespace_delete(claim, committed);
            }
            Ok(CommitDisable::NotConfigured) => {
                self.release_uncommitted_disable(&manager, claim, &attempt).await;
            }
            Ok(CommitDisable::StaleReview) => {
                let message = CLOUD_BACKUP_DISABLE_STALE_REVIEW_MESSAGE.to_string();
                self.fail_uncommitted_disable(&manager, claim, &attempt, message).await;
            }
            Err(error) => {
                let message = error.logged_reader_message("Failed to commit cloud backup disable");
                self.fail_uncommitted_disable(&manager, claim, &attempt, message).await;
            }
        }

        Produces::ok(())
    }

    /// Ends an attempt that never committed: nothing was deleted, so writes resume
    async fn release_uncommitted_disable(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: &DisableAttempt,
    ) {
        if matches!(attempt, DisableAttempt::Uncommitted { .. }) {
            let blocker = CloudBackupWriteBlocker::Disabling { operation_id: attempt.attempt_id() };
            send!(self.write.unblock(blocker));
        }

        self.finish_disable_operation(manager, claim);
    }

    async fn fail_uncommitted_disable(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: &DisableAttempt,
        message: String,
    ) {
        // callers log the cause before building the reader message, so logging it here again
        // would add a second line with less detail
        manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
            message,
            can_keep_enabled: false,
        });
        self.release_uncommitted_disable(manager, claim, attempt).await;
    }

    fn schedule_disable_namespace_delete(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
    ) {
        let write = self.write.clone();
        self.addr.send_fut_with(move |addr| async move {
            let result = Self::delete_cloud_backup_namespace_for_operation(
                write,
                disabling.namespace_id.clone(),
                claim,
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(()) | Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                self.schedule_disable_local_cleanup(claim, disabling);
            }
            Err(CloudBackupError::CloudStorage(error)) => {
                let message =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error)
                        .logged_reader_message("Failed to delete cloud backup namespace");
                self.fail_disable_after_namespace_delete_started(
                    &manager, claim, disabling, message,
                );
            }
            Err(error) => {
                self.fail_disable_after_namespace_delete_started(
                    &manager,
                    claim,
                    disabling,
                    error.logged_reader_message("Failed to delete cloud backup namespace"),
                );
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        if let Err(error) = result {
            self.fail_disable_after_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.logged_reader_message("Cloud backup disable local cleanup failed"),
            );
            return Produces::ok(());
        }

        manager.apply_disable_local_cleanup();

        if let Err(error) =
            manager.persist_disabled_after_remote_delete(&self.lifecycle, &disabling)
        {
            self.fail_disable_after_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.logged_reader_message("Failed to persist disabled cloud backup state"),
            );
            return Produces::ok(());
        }

        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: disabling.attempt_id };
        send!(self.write.unblock(blocker));

        if !self.runtime.accepts_completion(claim, "Keep Enabled") {
            return Produces::ok(());
        }

        info!("Disabled cloud backup and deleted active namespace");
        self.finish_disable_operation(&manager, claim);
        Produces::ok(())
    }

    fn fail_disable_after_namespace_delete_started(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: CommittedDisable,
        message: String,
    ) {
        let message =
            match manager.persist_disabling_failure(&self.lifecycle, &disabling, message.clone()) {
                Ok(()) => message,
                Err(error) => {
                    error.logged_reader_message("Failed to persist cloud backup disable failure")
                }
            };
        self.fail_disable_operation(manager, claim, message);
    }

    fn fail_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
    ) {
        // callers log the cause before building the reader message, so logging it here again
        // would add a second line with less detail
        //
        // nothing uncommitted is persisted and a committed disable only goes forward, so a
        // failure never offers Keep Enabled
        manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
            message,
            can_keep_enabled: false,
        });
        self.finish_disable_operation(manager, claim);
    }

    fn finish_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.pending_disable_write_drain = None;
        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
    }

    pub async fn keep_cloud_backup_enabled(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_keep_cloud_backup_enabled().await;
            send!(addr.complete_keep_cloud_backup_enabled(result));
        });
        Produces::ok(())
    }

    pub async fn complete_keep_cloud_backup_enabled(
        &mut self,
        result: Result<CloudBackupKeepEnabledPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        match result {
            Ok(CloudBackupKeepEnabledPreparation::AlreadyConfigured) => {
                manager.clear_stale_disable_failure_if_configured();
            }
            Ok(CloudBackupKeepEnabledPreparation::AlreadyDisabled) => {}
            Ok(CloudBackupKeepEnabledPreparation::Ready(disabling)) => {
                let Some(restored) = Self::restore_configured_cloud_backup_after_disable(
                    &manager,
                    &self.lifecycle,
                    &disabling,
                ) else {
                    return Produces::ok(());
                };

                if !restored {
                    return Produces::ok(());
                }

                send!(self.write.unblock(CloudBackupWriteBlocker::Disabling {
                    operation_id: disabling.attempt_id,
                }));

                manager.finish_keep_cloud_backup_enabled();
                if let Err(error) =
                    call!(self.uploads.resume_wallet_uploads_from_persisted_state()).await
                {
                    error!("Failed to resume cloud backup uploads after Keep Enabled: {error}");
                    manager.apply_sync_state(SyncState::Failed(
                        GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into(),
                    ));
                }
                if let Err(error) =
                    call!(self.uploads.ensure_pending_upload_verification_loop()).await
                {
                    error!(
                        "Failed to resume pending cloud backup verification after Keep Enabled: {error}"
                    );
                    manager.apply_sync_state(SyncState::Failed(
                        GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into(),
                    ));
                }
                manager.refresh_sync_health();

                if let Some(claim) = self.runtime.claim()
                    && claim.operation() == CloudBackupExclusiveOperation::Disable
                {
                    self.finish_disable_operation(&manager, claim);
                } else {
                    self.pending_disable_write_drain = None;
                }
            }
            Err(error) => {
                manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.logged_reader_message("Failed to keep cloud backup enabled"),
                    can_keep_enabled: false,
                });
            }
        }

        Produces::ok(())
    }

    fn restore_configured_cloud_backup_after_disable(
        manager: &RustCloudBackupManager,
        lifecycle: &LifecycleWriter,
        disabling: &CommittedDisable,
    ) -> Option<bool> {
        match manager.restore_configured_cloud_backup_after_disable(lifecycle, disabling) {
            Ok(restored) => Some(restored),
            Err(error) => {
                manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.logged_reader_message(
                        "Failed to restore configured cloud backup after disable",
                    ),
                    can_keep_enabled: false,
                });
                None
            }
        }
    }
}
