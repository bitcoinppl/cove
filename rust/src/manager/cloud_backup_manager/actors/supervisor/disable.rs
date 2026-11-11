use super::*;
use crate::manager::cloud_backup_manager::ops::DisableAttempt;
use crate::manager::cloud_backup_manager::owner::lifecycle::{CommitDisable, CommittedDisable};

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

        // the running Turn Off already pauses background writes through the published policy;
        // this barrier waits out the write in flight and those queued before it, so an upload
        // that already started can't recreate remote data after the namespace delete. The
        // supervisor never waits on the write supervisor: the barrier's reply comes back as
        // `complete_disable_write_drain`
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await;
            send!(addr.complete_disable_write_drain(claim, attempt, drained));
        });

        Produces::ok(())
    }

    pub async fn complete_disable_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: DisableAttempt,
        drained: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let Some(attempt) = manager.disable_attempt_if_current(&attempt) else {
            // nothing was deleted, and ending the operation lets background writes run again
            self.finish_disable_operation(&manager, claim);
            return Produces::ok(());
        };

        manager.apply_disable_outcome(CloudBackupDisableOutcome::Started);
        if let Err(error) = drained {
            let message = error
                .logged_reader_message("Failed to clear cloud backup runtime state before disable");
            match attempt {
                DisableAttempt::Uncommitted { .. } => {
                    self.fail_uncommitted_disable(&manager, claim, message);
                }
                DisableAttempt::Committed(committed) => {
                    self.fail_disable_after_namespace_delete_started(
                        &manager, claim, committed, message,
                    );
                }
            }
            return Produces::ok(());
        }

        self.drain_disable_runtime(&manager);
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

    fn drain_disable_runtime(&mut self, manager: &RustCloudBackupManager) {
        self.detail_workflow.clear_authorization();
        self.clear_worker_runtime_state();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_state(SyncState::Idle);
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
            // nothing was deleted, and ending the operation lets background writes run again
            self.finish_disable_operation(&manager, claim);
            return Produces::ok(());
        };

        if let Err(error) = result {
            let message = error.logged_reader_message("Cloud backup disable blocker check failed");
            self.fail_uncommitted_disable(&manager, claim, message);
            return Produces::ok(());
        }

        // the commit point: from here on the disable only goes forward
        match self
            .runtime
            .commit(|lifecycle| manager.commit_disable_attempt(lifecycle, &review, attempt_id))
        {
            Ok(CommitDisable::Committed(committed)) => {
                self.runtime.commit_turn_off(claim, committed.clone());
                self.schedule_disable_namespace_delete(claim, committed);
            }
            Ok(CommitDisable::NotConfigured) => {
                // nothing was deleted, and ending the operation lets background writes run again
                self.finish_disable_operation(&manager, claim);
            }
            Ok(CommitDisable::StaleReview) => {
                let message = CLOUD_BACKUP_DISABLE_STALE_REVIEW_MESSAGE.to_string();
                self.fail_uncommitted_disable(&manager, claim, message);
            }
            Err(error) => {
                let message = error.logged_reader_message("Failed to commit cloud backup disable");
                self.fail_uncommitted_disable(&manager, claim, message);
            }
        }

        Produces::ok(())
    }

    fn fail_uncommitted_disable(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
    ) {
        // callers log the cause before building the reader message, so logging it here again
        // would add a second line with less detail
        manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
            message,
            can_keep_enabled: false,
        });
        self.finish_disable_operation(manager, claim);
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

        if let Err(error) = self
            .runtime
            .commit(|lifecycle| manager.persist_disabled_after_remote_delete(lifecycle, &disabling))
        {
            self.fail_disable_after_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.logged_reader_message("Failed to persist disabled cloud backup state"),
            );
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
        let persisted = self.runtime.commit(|lifecycle| {
            manager.persist_disabling_failure(lifecycle, &disabling, message.clone())
        });
        let message = match persisted {
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
        error!("disable_cloud_backup failed: {message}");

        // nothing uncommitted is persisted and a committed disable only goes forward
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
        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }
}
