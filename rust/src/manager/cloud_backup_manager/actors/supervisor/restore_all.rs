use super::cloud_only::cloud_only_restore_warning;
use super::*;

pub(crate) fn restore_all_marker_matches_active_namespace(
    manager: &RustCloudBackupManager,
) -> bool {
    let Ok(state) = Database::global().cloud_backup_state.get() else {
        return false;
    };
    let Some(marker) = state.pending_restore_all() else {
        return false;
    };

    manager.current_namespace_id().ok().as_deref() == Some(marker.namespace_id.as_str())
}

pub(crate) fn reconcile_restore_all_after_cloud_only_fetch(
    manager: &RustCloudBackupManager,
    eligible_count: usize,
) {
    if !restore_all_marker_matches_active_namespace(manager) {
        return;
    }

    if eligible_count != 0 {
        manager.apply_restore_all_retry_required();
        return;
    }

    match CloudBackupStore::global().clear_restore_all_marker() {
        Ok(_) => manager.reset_restore_all(),
        Err(error) => {
            manager.apply_cloud_only_operation(CloudOnlyOperation::Failed {
                error: error.reader_message(),
            });
            manager.apply_restore_all_retry_required();
        }
    }
}

impl CloudBackupSupervisor {
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

        let Some(claim) = self.begin_restore_all_exclusive_operation() else {
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

        manager.clear_cloud_only_restore_failures(
            &frozen_wallets.iter().map(|wallet| wallet.record_id.clone()).collect::<Vec<_>>(),
        );
        manager.apply_cloud_only_operation(CloudOnlyOperation::Idle);
        manager.apply_restore_all_started(claim, frozen_count);

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_restore_all_cloud_wallets(frozen_wallets).await;
            send!(addr.complete_restore_all_preparation(claim, result));
        });
    }

    pub(crate) fn request_restore_all_cancellation(&mut self) {
        let Some(claim) = self.active_operation.claim() else { return };
        let Some(run) = self.active_operation.restore_all(claim) else { return };
        let cancellation = &run.cancellation;
        if cancellation.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Some(manager) = self.manager() {
            manager.apply_restore_all_cancellation_requested(claim);
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
        self.active_operation.clear();
        manager.project_exclusive_operation_finished(claim);
    }

    pub async fn complete_restore_all_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedRestoreAll, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(cancellation) =
            self.active_operation.restore_all(claim).map(|run| run.cancellation.clone())
        else {
            return Produces::ok(());
        };
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
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

        manager.apply_restore_all_started(claim, ordered_queue.len() as u32);
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
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(false);
        };

        manager.clear_cloud_only_restore_failures(std::slice::from_ref(&wallet.record_id));
        manager.apply_restore_all_progress(claim, completed, Some(wallet.name));
        Produces::ok(true)
    }

    pub async fn complete_restore_all_record(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        wallet: CloudBackupWalletItem,
        result: Result<WalletRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<bool> {
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
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

        manager.apply_restore_all_progress(claim, completed, None);
        Produces::ok(true)
    }

    pub async fn complete_restore_all_record_refresh(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        detail_claim: DetailResultClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<bool> {
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(false);
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(false);
        };

        if self.detail_workflow.is_latest_result(detail_claim)
            && let Some(result) = result
        {
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
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(());
        };

        manager.apply_restore_all_progress(claim, completed, None);
        self.finish_restore_all_retry_required(&manager, claim, error);
        Produces::ok(())
    }

    pub async fn complete_restore_all_cancelled_with_remaining(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(());
        };

        self.finish_restore_all_retry_remaining(&manager, claim);
        Produces::ok(())
    }

    pub async fn complete_restore_all_queue_finished(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        if self.active_operation.restore_all(claim).is_none() {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
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

        self.finish_restore_all_claim(manager, claim, false);
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
        self.finish_restore_all_claim(manager, claim, true);
    }

    fn finish_restore_all_retry_remaining(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.finish_restore_all_claim(manager, claim, true);
    }

    fn finish_restore_all_claim(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        retry_remaining: bool,
    ) {
        self.active_operation.clear();
        manager.finish_restore_all_run(claim, retry_remaining);
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
            let Ok(detail_claim) = call!(addr.start_detail_result_claim()).await else {
                return;
            };
            let result = manager.refresh_cloud_backup_detail().await;
            let Ok(true) =
                call!(addr.complete_restore_all_record_refresh(claim, detail_claim, result)).await
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
