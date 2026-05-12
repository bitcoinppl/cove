use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use tracing::{info, warn};

use crate::database::Database;
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupDetailResult, CloudBackupError, CloudBackupStatus,
    RustCloudBackupManager, blocking_cloud_error, cloud_inventory::RemoteWalletTruth,
    offline_error_for_step,
};

impl RustCloudBackupManager {
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
            Err(CloudStorageError::NotFound(_)) => Vec::new(),
            Err(error) => {
                let error = blocking_cloud_error(
                    BlockingCloudStep::DetailRefresh,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                );

                return Some(CloudBackupDetailResult::AccessError(error));
            }
        };

        let remote_wallet_truth =
            match self.load_remote_wallet_truth(&wallet_record_ids, cloud.clone()).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => return Some(CloudBackupDetailResult::AccessError(error)),
            };

        self.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

        match self
            .build_cloud_backup_detail_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
            .await
        {
            Ok(detail) => Some(CloudBackupDetailResult::Success(detail)),
            Err(error) => Some(CloudBackupDetailResult::AccessError(error)),
        }
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

        let confirmed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
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
                        "cleanup_confirmed_pending_blobs: persist confirmed record_id={} failed: {error}",
                        confirmed_state.record_id()
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
