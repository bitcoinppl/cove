use cove_device::cloud_storage::CloudStorage;
use tracing::{info, warn};

use crate::database::Database;
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupDetailInventorySnapshot,
    CloudBackupDetailInventorySnapshotResult, CloudBackupDetailResult, CloudBackupError,
    CloudBackupOtherBackupsState, CloudBackupStatus, RustCloudBackupManager, blocking_cloud_error,
    cloud_inventory::CloudWalletInventory, cloud_inventory::RemoteWalletTruth,
    offline_error_for_step,
};

impl RustCloudBackupManager {
    pub(crate) async fn load_cloud_backup_detail_inventory_snapshot(
        &self,
    ) -> Option<CloudBackupDetailInventorySnapshotResult> {
        let status = self.state.read().status().clone();
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            info!("load_cloud_backup_detail_inventory_snapshot: skipping, status={status:?}");
            return None;
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => {
                return Some(CloudBackupDetailInventorySnapshotResult::AccessError(error));
            }
        };
        let cloud = CloudStorage::global_explicit_client();
        let snapshot = match cloud.list_wallet_backups_snapshot(namespace.clone()).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let error = blocking_cloud_error(
                    BlockingCloudStep::DetailRefresh,
                    CloudBackupError::cloud_storage_context(
                        "load wallet backup inventory snapshot",
                        error,
                    ),
                );

                return Some(CloudBackupDetailInventorySnapshotResult::AccessError(error));
            }
        };

        let provisional_detail = if snapshot.is_complete {
            None
        } else {
            match self.build_provisional_cloud_backup_detail(&snapshot.names).await {
                Ok(detail) => Some(detail),
                Err(error) => {
                    return Some(CloudBackupDetailInventorySnapshotResult::AccessError(error));
                }
            }
        };

        Some(CloudBackupDetailInventorySnapshotResult::Success(
            CloudBackupDetailInventorySnapshot {
                namespace,
                wallet_record_ids: snapshot.names,
                is_complete: snapshot.is_complete,
                provisional_detail,
            },
        ))
    }

    pub(crate) async fn complete_cloud_backup_detail_inventory_snapshot(
        &self,
        snapshot: CloudBackupDetailInventorySnapshot,
    ) -> Option<CloudBackupDetailResult> {
        let status = self.state.read().status().clone();
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            info!("complete_cloud_backup_detail_inventory_snapshot: skipping, status={status:?}");
            return None;
        }

        let current_namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => return Some(CloudBackupDetailResult::AccessError(error)),
        };
        if current_namespace != snapshot.namespace {
            return Some(CloudBackupDetailResult::AccessError(CloudBackupError::Internal(
                "cloud backup namespace changed during inventory refresh".into(),
            )));
        }

        if self.is_known_offline() && !snapshot.is_complete {
            return Some(CloudBackupDetailResult::AccessError(offline_error_for_step(
                BlockingCloudStep::DetailRefresh,
            )));
        }

        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids = if snapshot.is_complete {
            snapshot.wallet_record_ids
        } else {
            match cloud.list_wallet_backups(snapshot.namespace.clone()).await {
                Ok(record_ids) => record_ids,
                Err(error) => {
                    let error = blocking_cloud_error(
                        BlockingCloudStep::DetailRefresh,
                        CloudBackupError::cloud_storage_context("list wallet backups", error),
                    );

                    return Some(CloudBackupDetailResult::AccessError(error));
                }
            }
        };

        Some(self.finish_cloud_backup_detail_refresh(wallet_record_ids, cloud).await)
    }

    /// List wallet backups in the current namespace and build detail
    ///
    /// Returns None if disabled. On access errors, returns AccessError so the UI can
    /// surface an explicit recovery action instead of mutating backup state during refresh
    pub(crate) async fn refresh_cloud_backup_detail(&self) -> Option<CloudBackupDetailResult> {
        let status = self.state.read().status().clone();
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            info!("refresh_cloud_backup_detail: skipping, status={status:?}");
            return None;
        }

        let namespace = match self.current_namespace_id() {
            Ok(ns) => ns,
            Err(error) => return Some(CloudBackupDetailResult::AccessError(error)),
        };

        if self.is_known_offline() {
            return Some(CloudBackupDetailResult::AccessError(offline_error_for_step(
                BlockingCloudStep::DetailRefresh,
            )));
        }

        info!("refresh_cloud_backup_detail: listing wallets for namespace {namespace}");
        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace).await {
            Ok(ids) => ids,
            Err(error) => {
                let error = blocking_cloud_error(
                    BlockingCloudStep::DetailRefresh,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                );

                return Some(CloudBackupDetailResult::AccessError(error));
            }
        };

        Some(self.finish_cloud_backup_detail_refresh(wallet_record_ids, cloud).await)
    }

    async fn finish_cloud_backup_detail_refresh(
        &self,
        wallet_record_ids: Vec<String>,
        cloud: cove_device::cloud_storage::CloudStorageClient,
    ) -> CloudBackupDetailResult {
        let remote_wallet_truth =
            match self.load_remote_wallet_truth(&wallet_record_ids, cloud).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => return CloudBackupDetailResult::AccessError(error),
            };

        self.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

        match self
            .build_cloud_backup_detail_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
            .await
        {
            Ok(detail) => CloudBackupDetailResult::Success(detail),
            Err(error) => CloudBackupDetailResult::AccessError(error),
        }
    }

    async fn build_provisional_cloud_backup_detail(
        &self,
        wallet_record_ids: &[String],
    ) -> Result<crate::manager::cloud_backup_manager::CloudBackupDetail, CloudBackupError> {
        let mut unknown_record_ids = self.expected_wallet_record_ids().await?;
        unknown_record_ids.extend(wallet_record_ids.iter().cloned());

        let other_backups = self
            .state
            .read()
            .detail()
            .map(|detail| detail.other_backups)
            .unwrap_or(CloudBackupOtherBackupsState::Loaded { summary: Default::default() });
        let inventory = CloudWalletInventory::load_with_remote_truth(
            wallet_record_ids,
            RemoteWalletTruth { unknown_record_ids, ..RemoteWalletTruth::default() },
        )
        .await?;

        Ok(inventory.build_detail(other_backups))
    }

    pub(crate) fn cleanup_confirmed_pending_blobs(&self, remote_wallet_truth: &RemoteWalletTruth) {
        let namespace_id = match self.current_namespace_id() {
            Ok(namespace_id) => namespace_id,
            Err(_) => return,
        };

        let table = &Database::global().cloud_blob_sync_states;
        let states = match table.list() {
            Ok(states) => states,
            Err(error) => {
                warn!(
                    "cleanup_confirmed_pending_blobs: list cloud blob sync states failed: {error}"
                );
                return;
            }
        };

        let confirmed_at = crate::manager::cloud_backup_manager::current_timestamp();
        let mut updated = false;

        for state in states {
            if state.namespace_id != namespace_id {
                continue;
            }

            let pending_state = match &state.state {
                PersistedCloudBlobState::UploadedPendingConfirmation(pending_state) => {
                    pending_state
                }

                _ => continue,
            };

            if !remote_wallet_revision_matches(
                remote_wallet_truth,
                state.record_id(),
                &pending_state.revision_hash,
            ) {
                continue;
            }

            let confirmed_state =
                state.with_state(PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash: pending_state.revision_hash.clone(),
                    confirmed_at,
                }));

            let persisted = match table.set_if_current(&state, &confirmed_state) {
                Ok(persisted) => persisted,
                Err(error) => {
                    warn!(
                        "cleanup_confirmed_pending_blobs: persist confirmed state failed: {error}"
                    );
                    continue;
                }
            };

            if !persisted {
                continue;
            }

            updated = true;
        }

        if updated {
            self.refresh_pending_upload_verification_state();
        }
    }
}

pub(crate) fn remote_wallet_revision_matches(
    remote_wallet_truth: &RemoteWalletTruth,
    record_id: &str,
    expected_revision: &str,
) -> bool {
    matches!(
        remote_wallet_truth.summaries_by_record_id.get(record_id),
        Some(summary) if summary.revision_hash == expected_revision
    )
}
