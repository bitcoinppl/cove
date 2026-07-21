use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn begin_cloud_only_fetch_request(&mut self) {
        let Some(manager) = self.manager() else { return };
        let request_id = self.next_request_id();
        self.active_cloud_only_fetch_request = Some(request_id);
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Started);

        self.addr.send_fut_with(move |addr| async move {
            let result = manager.do_fetch_cloud_only_wallets().await;
            send!(addr.complete_cloud_only_fetch_request(request_id, result));
        });
    }

    pub(crate) fn begin_restore_cloud_wallet_operation(&mut self, record_id: String) {
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

    pub(crate) fn begin_delete_cloud_wallet_operation(&mut self, record_id: String) {
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
                let eligible_count = items
                    .iter()
                    .filter(|wallet| {
                        wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice
                    })
                    .count();
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(
                    items,
                ));
                self.reconcile_restore_all_after_cloud_only_fetch(&manager, eligible_count);
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

    pub async fn complete_delete_cloud_wallet_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedCloudWalletDelete, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation.claim() != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
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
                self.active_operation.clear();
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
        if self.active_operation.claim() != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Deleted { record_id },
                );
                manager.refresh_sync_health();
                let detail_claim = self.detail_workflow.start_operation_result();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.reader_message(),
                ));
                self.active_operation.clear();
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
        if self.active_operation.claim() != Some(claim) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_operation.clear();
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
                let detail_claim = self.detail_workflow.start_operation_result();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
                });
            }
            Ok(WalletRestoreOutcome::SkippedDuplicate) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::SkippedDuplicate { record_id },
                );
                manager.refresh_sync_health();
                let detail_claim = self.detail_workflow.start_operation_result();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::RestoreFailed {
                        record_id,
                        error: error.reader_message(),
                    },
                );
                self.active_operation.clear();
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }
}

pub(crate) fn cloud_only_restore_warning(
    warning: crate::backup::import::LabelRestoreWarning,
) -> CloudBackupCloudOnlyOperationWarning {
    CloudBackupCloudOnlyOperationWarning {
        message: format!(
            "{} was restored, but its labels could not be imported",
            warning.wallet_name
        ),
        error: CLOUD_BACKUP_LABELS_WARNING_MESSAGE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::import::LabelRestoreWarning;

    #[test]
    fn cloud_only_restore_warning_does_not_publish_label_diagnostics() {
        let warning = cloud_only_restore_warning(LabelRestoreWarning {
            wallet_name: "Primary".into(),
            error: "record=secret account=person@example.com".into(),
        });

        assert_eq!(warning.message, "Primary was restored, but its labels could not be imported");
        assert_eq!(warning.error, CLOUD_BACKUP_LABELS_WARNING_MESSAGE);
        assert!(!warning.error.contains("secret"));
    }
}
