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

    pub(crate) fn begin_restore_all_operation(&mut self, retry: bool) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let frozen_wallets = manager.restore_all_eligible_wallets();
        let frozen_count = frozen_wallets.len() as u32;
        let projected_state = manager.projected_restore_all_state();
        let can_start = match (retry, projected_state) {
            (false, CloudBackupRestoreAllState::StartAvailable { wallet_count }) => {
                wallet_count == frozen_count
            }
            (true, CloudBackupRestoreAllState::RetryAvailable { wallet_count }) => {
                wallet_count == frozen_count
            }
            _ => false,
        };
        if !can_start {
            return;
        }

        let Some(claim) = self.begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        ) else {
            return;
        };

        let namespace_id = match manager.current_namespace_id() {
            Ok(namespace_id) => namespace_id,
            Err(error) => {
                self.fail_restore_all_before_marker(&manager, claim, error);
                return;
            }
        };
        if let Err(error) = CloudBackupStore::global().persist_restore_all_marker(namespace_id) {
            self.fail_restore_all_before_marker(&manager, claim, error);
            return;
        }

        let cancellation = Arc::new(AtomicBool::new(false));
        self.active_restore_all_cancellation = Some(cancellation.clone());
        manager.clear_cloud_only_restore_failures(
            &frozen_wallets.iter().map(|wallet| wallet.record_id.clone()).collect::<Vec<_>>(),
        );
        manager.apply_cloud_only_operation(CloudOnlyOperation::Idle);
        manager.apply_restore_all_started(frozen_count);

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_restore_all_cloud_wallets(frozen_wallets).await;
            send!(addr.complete_restore_all_preparation(claim, cancellation, result));
        });
    }

    pub(crate) fn request_restore_all_cancellation(&mut self) {
        let Some(claim) = self.active_operation else { return };
        if claim.operation() != CloudBackupExclusiveOperation::RestoreAllCloudWallets {
            return;
        }
        let Some(cancellation) = &self.active_restore_all_cancellation else { return };
        if cancellation.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Some(manager) = self.manager() {
            manager.apply_restore_all_cancellation_requested();
        }
    }

    fn fail_restore_all_before_marker(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        error!("Failed to start Restore All: {error}");
        manager.apply_cloud_only_operation(CloudOnlyOperation::Failed {
            error: error.reader_message(),
        });
        manager.reset_restore_all();
        self.active_operation = None;
        self.active_restore_all_cancellation = None;
        manager.project_exclusive_operation_finished(claim);
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

    pub(crate) fn begin_recover_other_backups_operation(&mut self) {
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

    pub(crate) fn begin_delete_other_backups_operation(&mut self) {
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
                let eligible_count = items
                    .iter()
                    .filter(|wallet| {
                        wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice
                    })
                    .count();
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(
                    items,
                ));

                if restore_all_marker_matches_active_namespace(&manager) {
                    if eligible_count == 0 {
                        match CloudBackupStore::global().clear_restore_all_marker() {
                            Ok(_) => manager.reset_restore_all(),
                            Err(error) => {
                                manager.apply_cloud_only_operation(CloudOnlyOperation::Failed {
                                    error: error.reader_message(),
                                });
                                manager.apply_restore_all_retry_required();
                            }
                        }
                    } else {
                        manager.apply_restore_all_retry_required();
                    }
                }
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
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::RestoreFailed {
                        record_id,
                        error: error.reader_message(),
                    },
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_restore_all_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        cancellation: Arc<AtomicBool>,
        result: Result<CloudBackupPreparedRestoreAll, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim)
            || self
                .active_restore_all_cancellation
                .as_ref()
                .is_none_or(|active| !Arc::ptr_eq(active, &cancellation))
        {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(());
        };
        if cancellation.load(Ordering::Acquire) {
            self.finish_restore_all_clean(&manager, claim);
            return Produces::ok(());
        }

        let mut prepared = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.finish_restore_all_retry_required(&manager, claim, error);
                return Produces::ok(());
            }
        };
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(
            prepared.authoritative_wallets().to_vec(),
        ));

        let ordered_queue = prepared.ordered_queue().to_vec();
        if ordered_queue.is_empty() {
            self.finish_restore_all_clean(&manager, claim);
            return Produces::ok(());
        }

        manager.apply_restore_all_started(ordered_queue.len() as u32);
        let addr = self.addr.clone();
        addr.send_fut_with(move |addr| async move {
            run_restore_all_queue(addr, manager, claim, cancellation, &mut prepared, ordered_queue)
                .await;
        });

        Produces::ok(())
    }

    pub async fn begin_restore_all_record(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        wallet: CloudBackupWalletItem,
    ) -> ActorResult<bool> {
        if self.active_operation != Some(claim) {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(false);
        };

        manager.clear_cloud_only_restore_failures(std::slice::from_ref(&wallet.record_id));
        manager.apply_restore_all_progress(completed, Some(wallet.name));
        Produces::ok(true)
    }

    pub async fn complete_restore_all_record(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        wallet: CloudBackupWalletItem,
        result: Result<WalletRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<bool> {
        if self.active_operation != Some(claim) {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(false);
        };

        match result {
            Ok(WalletRestoreOutcome::Restored { labels_warning }) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Restored {
                        record_id: wallet.record_id,
                        warning: labels_warning.map(cloud_only_restore_warning),
                    },
                );
                manager.refresh_sync_health();
            }
            Ok(WalletRestoreOutcome::SkippedDuplicate) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::SkippedDuplicate {
                        record_id: wallet.record_id,
                    },
                );
                manager.refresh_sync_health();
            }
            Err(error) if restore_all_must_stop(&error) => {}
            Err(error) => {
                manager.apply_cloud_only_restore_failure(wallet.record_id, error.reader_message());
                manager.apply_cloud_only_operation(CloudOnlyOperation::Idle);
            }
        }

        manager.apply_restore_all_progress(completed, None);
        Produces::ok(true)
    }

    pub async fn complete_restore_all_record_refresh(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<bool> {
        if self.active_operation != Some(claim) {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(false);
        };

        if let Some(result) = result {
            apply_cloud_only_operation_refresh_detail_result(&manager, &result);
            if let CloudBackupDetailResult::AccessError(error) = result
                && restore_all_must_stop(&error)
            {
                self.finish_restore_all_retry_required(&manager, claim, error);
                return Produces::ok(false);
            }
        }

        Produces::ok(true)
    }

    pub async fn complete_restore_all_provider_stop(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        error: CloudBackupError,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(());
        };

        manager.apply_restore_all_progress(completed, None);
        self.finish_restore_all_retry_required(&manager, claim, error);
        Produces::ok(())
    }

    pub async fn complete_restore_all_cancelled_with_remaining(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(());
        };

        self.finish_restore_all_retry_remaining(&manager, claim);
        Produces::ok(())
    }

    pub async fn complete_restore_all_queue_finished(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            self.active_restore_all_cancellation = None;
            return Produces::ok(());
        };

        if manager.restore_all_eligible_wallets().is_empty() {
            self.finish_restore_all_clean(&manager, claim);
        } else {
            self.finish_restore_all_retry_remaining(&manager, claim);
        }
        Produces::ok(())
    }

    fn finish_restore_all_clean(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        if let Err(error) = CloudBackupStore::global().clear_restore_all_marker() {
            self.finish_restore_all_retry_required(manager, claim, error);
            return;
        }

        manager.reset_restore_all();
        self.finish_restore_all_claim(manager, claim);
    }

    fn finish_restore_all_retry_required(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        error!("Restore All stopped before completing: {error}");
        manager.apply_cloud_only_operation(CloudOnlyOperation::Failed {
            error: error.reader_message(),
        });
        manager.apply_restore_all_retry_required();
        self.finish_restore_all_claim(manager, claim);
    }

    fn finish_restore_all_retry_remaining(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        manager.apply_restore_all_retry_required();
        self.finish_restore_all_claim(manager, claim);
    }

    fn finish_restore_all_claim(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.active_operation = None;
        self.active_restore_all_cancellation = None;
        manager.project_exclusive_operation_finished(claim);
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
                manager.apply_sync_state(SyncState::Syncing);
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
                manager.apply_sync_state(SyncState::Failed(error.reader_message()));
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

        manager.apply_sync_state(SyncState::Idle);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}

async fn run_restore_all_queue(
    addr: WeakAddr<CloudBackupSupervisor>,
    manager: Arc<RustCloudBackupManager>,
    claim: CloudBackupExclusiveOperationClaim,
    cancellation: Arc<AtomicBool>,
    prepared: &mut CloudBackupPreparedRestoreAll,
    ordered_queue: Vec<CloudBackupWalletItem>,
) {
    let mut completed = 0_u32;
    for wallet in ordered_queue {
        if cancellation.load(Ordering::Acquire) {
            send!(addr.complete_restore_all_cancelled_with_remaining(claim));
            return;
        }

        let Ok(true) = call!(addr.begin_restore_all_record(claim, completed, wallet.clone())).await
        else {
            return;
        };
        let result = prepared.restore_record(&wallet.record_id).await;
        completed = completed.saturating_add(1);
        let result = match result {
            Err(error) if restore_all_must_stop(&error) => {
                send!(addr.complete_restore_all_provider_stop(claim, completed, error));
                return;
            }
            result => result,
        };
        let restored = result.is_ok();

        let Ok(true) =
            call!(addr.complete_restore_all_record(claim, completed, wallet, result)).await
        else {
            return;
        };

        if restored {
            let result = manager.refresh_cloud_backup_detail().await;
            let Ok(true) = call!(addr.complete_restore_all_record_refresh(claim, result)).await
            else {
                return;
            };
        }
    }

    send!(addr.complete_restore_all_queue_finished(claim));
}

fn restore_all_must_stop(error: &CloudBackupError) -> bool {
    matches!(
        CloudStorageIssue::from(error),
        CloudStorageIssue::AuthorizationRequired
            | CloudStorageIssue::Offline
            | CloudStorageIssue::Unavailable
    )
}

fn cloud_only_restore_warning(
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
