use cove_device::cloud_storage::CloudStorageClient;

use super::*;
use crate::manager::cloud_backup_manager::{
    CloudOnlyFetch, CloudOnlyFetchApplication, CloudOnlyState,
};

impl CloudBackupSupervisor {
    /// Starts a fetch when the cache is unloaded and the detail inventory can say what to fetch
    pub async fn ensure_cloud_only_fetch(&mut self) -> ActorResult<()> {
        if let Some(manager) = self.manager() {
            self.begin_cloud_only_fetch_if_needed(manager);
        }

        Produces::ok(())
    }

    pub(crate) fn begin_cloud_only_fetch_if_needed(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
    ) {
        if self.active_cloud_only_fetch_request.is_some()
            || !manager.detail_inventory_is_ready()
            || !matches!(
                manager.state.read().cloud_only(),
                CloudOnlyState::NotFetched | CloudOnlyState::Failed { .. }
            )
        {
            return;
        }

        let cloud = CloudStorage::global_silent_client();
        self.begin_cloud_only_fetch(manager, cloud);
    }

    fn begin_cloud_only_fetch(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        cloud: CloudStorageClient,
    ) {
        let request_id = self.next_request_id();
        self.active_cloud_only_fetch_request = Some(request_id);
        // read before the fetch is spawned so a listing applied while it runs makes it stale
        let listing_generation = manager.state.read().cloud_only_listing_generation();
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Started);

        self.addr.send_fut_with(move |addr| async move {
            let result =
                manager.do_fetch_cloud_only_wallets_with_client(cloud, listing_generation).await;
            send!(addr.complete_cloud_only_fetch_request(request_id, result));
        });
    }

    pub(crate) fn begin_restore_cloud_wallet_operation(&mut self, record_id: String) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        else {
            return;
        };

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: record_id.clone(),
        });

        // read before the restore reads the local wallets, so a later change can discard its result
        let listing_generation = manager.state.read().cloud_only_listing_generation();
        addr.send_fut_with(move |addr| async move {
            let result = manager.do_restore_cloud_wallet(&record_id).await;
            send!(addr.complete_restore_cloud_wallet(claim, record_id, listing_generation, result));
        });
    }

    pub(crate) fn begin_delete_cloud_wallet_operation(&mut self, record_id: String) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
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
        result: Result<CloudOnlyFetch, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_cloud_only_fetch_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_cloud_only_fetch_request = None;
            return Produces::ok(());
        };

        match result {
            Ok(fetch) => {
                // a discarded fetch already asked for its replacement, and its rows prove nothing
                let application = manager.apply_cloud_only_fetch_outcome(
                    CloudBackupCloudOnlyFetchOutcome::Loaded(fetch),
                );
                if application == CloudOnlyFetchApplication::Applied {
                    self.reconcile_restore_all_after_cloud_only_fetch(&manager);
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

    pub async fn complete_delete_cloud_wallet_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedCloudWalletDelete, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
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
        let Some(manager) = self.current(claim) else {
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
        listing_generation: u64,
        result: Result<CloudOnlyRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let (outcome, committed_local_wallet) = match result {
            Ok(CloudOnlyRestoreOutcome::Restored { labels_warning }) => (
                CloudBackupCloudOnlyWalletOutcome::Restored {
                    record_id,
                    warning: labels_warning.map(cloud_only_restore_warning),
                },
                true,
            ),
            Ok(CloudOnlyRestoreOutcome::SecretAdded { wallet_id }) => {
                manager.handle_restored_secret_added(wallet_id);
                (
                    CloudBackupCloudOnlyWalletOutcome::AlreadyOnDevice {
                        record_id,
                        listing_generation,
                    },
                    true,
                )
            }
            Ok(CloudOnlyRestoreOutcome::AlreadyOnDevice) => (
                CloudBackupCloudOnlyWalletOutcome::AlreadyOnDevice {
                    record_id,
                    listing_generation,
                },
                false,
            ),
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::RestoreFailed {
                        record_id,
                        error: error.reader_message(),
                    },
                );
                self.active_operation.clear();
                manager.project_exclusive_operation_finished(claim);

                return Produces::ok(());
            }
        };

        manager.apply_cloud_only_wallet_outcome(outcome);

        // a restored wallet can turn other listed records into copies of it, which matching record
        // IDs cannot show, so the rows are read again
        if committed_local_wallet {
            manager.apply_local_wallets_changed();
        }

        manager.refresh_sync_health();
        let detail_claim = self.detail_workflow.start_operation_result();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.refresh_cloud_backup_detail().await;
            send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
        });

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
