use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn start_cloud_only_fetch_request(&mut self) {
        let Some(manager) = self.manager() else { return };
        let request_id = self.next_request_id();
        self.active_cloud_only_fetch_request = Some(request_id);
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Started);

        self.addr.send_fut_with(move |addr| async move {
            let result = manager.do_fetch_cloud_only_wallets().await;
            send!(addr.complete_cloud_only_fetch_request(request_id, result));
        });
    }

    pub(crate) fn start_restore_cloud_wallet_operation(&mut self, record_id: String) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        else {
            return;
        };

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: record_id.clone(),
        });
        addr.send_fut_with(move |addr| async move {
            let result = manager.do_restore_cloud_wallet(&record_id).await;
            send!(addr.complete_restore_cloud_wallet(claim, record_id, result));
        });
    }

    pub(crate) fn start_delete_cloud_wallet_operation(&mut self, record_id: String) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
        else {
            return;
        };

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: record_id.clone(),
        });
        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_delete_cloud_wallet(&record_id).await;
            send!(addr.complete_delete_cloud_wallet_preparation(claim, result));
        });
    }

    pub(crate) fn start_recover_other_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = Self::begin_other_backups_operation(
            self,
            &manager,
            CloudBackupExclusiveOperation::RecoverOtherBackups,
            CloudBackupOtherBackupsOutcome::Recovering,
        ) else {
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.do_recover_other_backups().await;
            send!(addr.complete_recover_other_backups(claim, result));
        });
    }

    pub(crate) fn start_delete_other_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = Self::begin_other_backups_operation(
            self,
            &manager,
            CloudBackupExclusiveOperation::DeleteOtherBackups,
            CloudBackupOtherBackupsOutcome::Deleting,
        ) else {
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.do_delete_other_backups().await;
            send!(addr.complete_delete_other_backups(claim, result));
        });
    }

    pub async fn complete_cloud_only_fetch_request(
        &mut self,
        request_id: u64,
        result: Result<Vec<CloudBackupWalletItem>, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_cloud_only_fetch_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_cloud_only_fetch_request = None;
            return Produces::ok(());
        };

        match result {
            Ok(items) => {
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(
                    items,
                ));
            }
            Err(error) => {
                error!("Failed to fetch cloud-only wallets: {error}");
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Failed(
                    error.reader_message(),
                ));
            }
        }

        self.active_cloud_only_fetch_request = None;
        Produces::ok(())
    }

    pub(crate) fn begin_other_backups_operation(
        supervisor: &mut CloudBackupSupervisor,
        manager: &RustCloudBackupManager,
        operation: CloudBackupExclusiveOperation,
        outcome: CloudBackupOtherBackupsOutcome,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if !matches!(
            manager.state.read().other_backups_operation(),
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

    pub async fn complete_delete_cloud_wallet_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedCloudWalletDelete, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(prepared) => {
                let record_id = prepared.record_id.clone();
                let write = self.write.clone();
                self.addr.send_fut_with(move |addr| async move {
                    let result =
                        Self::delete_prepared_cloud_wallet_for_operation(write, prepared, claim)
                            .await;
                    send!(addr.complete_delete_cloud_wallet(claim, record_id, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_delete_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
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
            Ok(()) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Deleted { record_id },
                );
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_restore_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
        result: Result<WalletRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(WalletRestoreOutcome::Restored { labels_warning }) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Restored {
                        record_id,
                        warning: labels_warning.map(cloud_only_restore_warning),
                    },
                );
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Ok(WalletRestoreOutcome::SkippedDuplicate) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::SkippedDuplicate { record_id },
                );
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_recover_other_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupRestoreReport, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(report) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Recovered {
                    wallets_restored: report.wallets_restored,
                    wallets_failed: report.wallets_failed,
                    failed_wallet_errors: report.failed_wallet_errors,
                });
                manager.apply_sync_outcome(CloudBackupSyncOutcome::Started);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.do_sync_unsynced_wallets().await;
                    send!(addr.complete_operation_sync(claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation = None;
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Deleted);
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation = None;
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_sync_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(error.reader_message()));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_sync_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        if let Some(result) = result {
            apply_refresh_detail_result(&manager, &result);
        }

        manager.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}

fn cloud_only_restore_warning(
    warning: crate::backup::import::LabelRestoreWarning,
) -> CloudBackupCloudOnlyOperationWarning {
    CloudBackupCloudOnlyOperationWarning {
        message: format!(
            "{} was restored, but its labels could not be imported",
            warning.wallet_name
        ),
        error: warning.error,
    }
}
