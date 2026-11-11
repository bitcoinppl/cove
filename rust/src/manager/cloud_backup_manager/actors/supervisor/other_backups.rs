use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn begin_recover_other_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = Self::begin_other_backups_operation(
            self,
            &manager,
            CloudBackupExclusiveOperation::RecoverOtherBackups,
            CloudBackupOtherBackupsOutcome::Recovering,
        ) else {
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.do_recover_other_backups(&writes).await;
            send!(addr.complete_recover_other_backups(claim, result));
        });
    }

    pub(crate) fn begin_delete_other_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = Self::begin_other_backups_operation(
            self,
            &manager,
            CloudBackupExclusiveOperation::DeleteOtherBackups,
            CloudBackupOtherBackupsOutcome::Deleting,
        ) else {
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.do_delete_other_backups(&writes).await;
            send!(addr.complete_delete_other_backups(claim, result));
        });
    }

    pub(crate) fn begin_other_backups_operation(
        supervisor: &mut CloudBackupSupervisor,
        manager: &RustCloudBackupManager,
        operation: CloudBackupExclusiveOperation,
        outcome: CloudBackupOtherBackupsOutcome,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if !matches!(
            manager.inventory.read().other_backups_operation(),
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return None;
        }

        let claim = supervisor.begin_exclusive_operation(manager, operation)?;
        manager.apply_other_backups_outcome(outcome);
        Some(claim)
    }

    pub async fn complete_recover_other_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupRestoreReport, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(report) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Recovered {
                    wallets_restored: report.wallets_restored,
                    wallets_failed: report.wallets_failed,
                    failed_wallet_errors: report.failed_wallet_errors,
                });
                manager.apply_sync_state(SyncState::Syncing);
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.sync_unsynced_wallets_with(&writes).await;
                    send!(addr.complete_operation_sync(claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.logged_reader_message("Failed to recover other cloud backups"),
                ));
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_delete_other_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Deleted);
                manager.refresh_sync_health();
                let detail_claim = self.detail_workflow.start_operation_result();
                let config = self.runtime.generation();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail(config).await;
                    send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.logged_reader_message("Failed to delete other cloud backups"),
                ));
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_sync(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.refresh_sync_health();
                let detail_claim = self.detail_workflow.start_operation_result();
                let config = self.runtime.generation();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail(config).await;
                    send!(addr.complete_operation_sync_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_sync_state(SyncState::Failed(
                    error.logged_reader_message("Cloud backup sync failed"),
                ));
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_sync_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        detail_claim: DetailResultClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        if self.detail_workflow.is_latest_result(detail_claim)
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, &result);
        }

        manager.apply_sync_state(SyncState::Idle);
        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}
