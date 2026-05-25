use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn start_disable_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let disabling = match result {
            Ok(CloudBackupDisablePreparation::AlreadyDisabled) => {
                self.finish_disable_operation(&manager, claim);
                return Produces::ok(());
            }
            Ok(CloudBackupDisablePreparation::Ready(disabling)) => *disabling,
            Err(error) => {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
                return Produces::ok(());
            }
        };

        let blocker =
            CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };

        // wait for the active write lane to drain before deleting the namespace so an upload
        // that already started cannot recreate remote data after disable deletes it
        if let Err(error) =
            call!(self.write.block_until_drained(blocker, self.addr.clone(), claim)).await
        {
            self.fail_disable_operation(
                &manager,
                claim,
                format!("install cloud backup disable fence: {error}"),
                manager.disable_can_keep_enabled(),
            );
            return Produces::ok(());
        }

        self.pending_disable_write_drain =
            Some(PendingDisableWriteDrain { claim, blocker, disabling });

        Produces::ok(())
    }

    pub async fn complete_disable_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        blocker: CloudBackupWriteBlocker,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
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

        let Some(disabling) = manager.current_disabling_if_current(&pending.disabling) else {
            self.finish_disable_operation(&manager, claim);
            return Produces::ok(());
        };

        manager.apply_disable_outcome(CloudBackupDisableOutcome::Started);
        if let Err(error) = self.drain_disable_runtime(&manager).await {
            self.fail_disable_before_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.to_string(),
            );
            return Produces::ok(());
        }

        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        if disabling.delete_started_at.is_some() {
            self.schedule_disable_namespace_delete(claim, disabling);
        } else {
            self.schedule_disable_blocker_check(manager, claim, disabling);
        }

        Produces::ok(())
    }

    async fn drain_disable_runtime(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        self.pending_enable_session = None;
        self.pending_verification_completion = None;
        self.runtime_passkey_authorization = None;
        call!(self.sync_health.clear_upload_runtime_state())
            .await
            .map_err(|error| CloudBackupError::Internal(error.to_string()))?;
        call!(self.uploads.clear_upload_runtime_state())
            .await
            .map_err(|error| CloudBackupError::Internal(error.to_string()))?;

        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        Ok(())
    }

    fn schedule_disable_blocker_check(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let cloud = CloudStorage::global_explicit_client();
            let result = manager.check_disable_blockers(&cloud, &disabling).await;
            send!(addr.complete_disable_blocker_check(claim, disabling, result));
        });
    }

    pub async fn complete_disable_blocker_check(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };
        let Some(disabling) = manager.current_disabling_if_current(&disabling) else {
            self.finish_disable_operation(&manager, claim);
            return Produces::ok(());
        };

        if let Err(error) = result {
            let message = error.to_string();
            if let Err(error) = manager.rollback_disable_before_delete(&disabling, message.clone())
            {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
            } else {
                self.finish_disable_operation(&manager, claim);
            }
            return Produces::ok(());
        }

        match manager.mark_disable_delete_started_if_current(&disabling) {
            Ok(Some(disabling)) => self.schedule_disable_namespace_delete(claim, disabling),
            Ok(None) => self.finish_disable_operation(&manager, claim),
            Err(error) => {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
            }
        }

        Produces::ok(())
    }

    fn schedule_disable_namespace_delete(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
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
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) | Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                self.schedule_disable_local_cleanup(manager, claim, disabling);
            }
            Err(CloudBackupError::CloudStorage(error)) => {
                let message =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error)
                        .to_string();
                self.fail_disable_after_namespace_delete_started(
                    &manager, claim, disabling, message,
                );
            }
            Err(error) => {
                self.fail_disable_after_namespace_delete_started(
                    &manager,
                    claim,
                    disabling,
                    error.to_string(),
                );
            }
        }

        Produces::ok(())
    }

    fn schedule_disable_local_cleanup(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.finish_disable_local_cleanup();
            send!(addr.complete_disable_local_cleanup(claim, disabling, result));
        });
    }

    pub async fn complete_disable_local_cleanup(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        if let Err(error) = result {
            self.fail_disable_after_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.to_string(),
            );
            return Produces::ok(());
        }

        if let Err(error) = manager.persist_disabled_after_remote_delete() {
            self.fail_disable_after_namespace_delete_started(
                &manager,
                claim,
                disabling,
                error.to_string(),
            );
            return Produces::ok(());
        }

        let blocker =
            CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };
        if let Err(error) = call!(self.write.unblock(blocker)).await {
            warn!("Failed to lift cloud backup disable fence: {error}");
        }

        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        info!("Disabled cloud backup and deleted active namespace");
        self.finish_disable_operation(&manager, claim);
        Produces::ok(())
    }

    fn fail_disable_before_namespace_delete_started(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        message: String,
    ) {
        if let Err(error) = manager.rollback_disable_before_delete(&disabling, message) {
            self.fail_disable_operation(
                manager,
                claim,
                error.to_string(),
                manager.disable_can_keep_enabled(),
            );
        } else {
            self.finish_disable_operation(manager, claim);
        }
    }

    fn fail_disable_after_namespace_delete_started(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        message: String,
    ) {
        let message = match manager.persist_disabling_failure(disabling, message.clone()) {
            Ok(()) => message,
            Err(error) => error.to_string(),
        };
        self.fail_disable_operation(manager, claim, message, manager.disable_can_keep_enabled());
    }

    fn fail_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
        can_keep_enabled: bool,
    ) {
        error!("disable_cloud_backup failed: {message}");
        manager
            .apply_disable_outcome(CloudBackupDisableOutcome::Failed { message, can_keep_enabled });
        self.finish_disable_operation(manager, claim);
    }

    fn finish_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.pending_disable_write_drain = None;
        self.active_operation = None;
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
                let Some(restored) =
                    Self::restore_configured_cloud_backup_after_disable(&manager, &disabling)
                else {
                    return Produces::ok(());
                };

                if !restored {
                    return Produces::ok(());
                }

                if let Err(error) = self
                    .unblock_cloud_backup_writes(CloudBackupWriteBlocker::Disabling {
                        operation_id: disabling.disable_generation,
                    })
                    .await
                {
                    warn!("Failed to lift cloud backup disable fence: {error}");
                }

                manager.finish_keep_cloud_backup_enabled();
                if let Some(claim) = self.active_operation
                    && claim.operation() == CloudBackupExclusiveOperation::Disable
                {
                    self.finish_disable_operation(&manager, claim);
                } else {
                    self.pending_disable_write_drain = None;
                }
            }
            Err(error) => {
                manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.to_string(),
                    can_keep_enabled: false,
                });
            }
        }

        Produces::ok(())
    }

    fn restore_configured_cloud_backup_after_disable(
        manager: &RustCloudBackupManager,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Option<bool> {
        match manager.restore_configured_cloud_backup_after_disable(disabling) {
            Ok(restored) => Some(restored),
            Err(error) => {
                manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.to_string(),
                    can_keep_enabled: false,
                });
                None
            }
        }
    }
}
