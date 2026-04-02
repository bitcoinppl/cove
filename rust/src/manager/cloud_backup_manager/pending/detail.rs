use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use tracing::{info, warn};

use super::super::{
    BlockingCloudStep, CloudBackupDetailResult, CloudBackupStatus, RustCloudBackupManager,
    cloud_inventory::RemoteWalletTruth,
};
use crate::database::Database;
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};

impl RustCloudBackupManager {
    /// List wallet backups in the current namespace and build detail
    ///
    /// Returns None if disabled. On NotFound, re-uploads all wallets automatically.
    /// On other errors, returns AccessError so the UI can offer a re-upload button
    pub(crate) fn refresh_cloud_backup_detail(&self) -> Option<CloudBackupDetailResult> {
        let status = self.state.read().status.clone();
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            info!("refresh_cloud_backup_detail: skipping, status={status:?}");
            return None;
        }

        let namespace = match self.current_namespace_id() {
            Ok(ns) => ns,
            Err(error) => return Some(CloudBackupDetailResult::AccessError(error.to_string())),
        };

        if self.is_definitely_offline() {
            return Some(CloudBackupDetailResult::AccessError(
                self.offline_error_for_step(BlockingCloudStep::DetailRefresh).to_string(),
            ));
        }

        info!("refresh_cloud_backup_detail: listing wallets for namespace {namespace}");
        let cloud = CloudStorage::global();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace) {
            Ok(ids) => ids,
            Err(CloudStorageError::NotFound(_)) => {
                info!("No wallet backups found in namespace, re-uploading all wallets");
                if let Err(error) = self.do_reupload_all_wallets() {
                    return Some(CloudBackupDetailResult::AccessError(format!(
                        "Failed to re-upload wallets: {error}"
                    )));
                }

                match cloud.list_wallet_backups(self.current_namespace_id().unwrap_or_default()) {
                    Ok(ids) => ids,
                    Err(error) => {
                        return Some(CloudBackupDetailResult::AccessError(error.to_string()));
                    }
                }
            }
            Err(error) => {
                if RustCloudBackupManager::is_connectivity_related_issue(
                    RustCloudBackupManager::cloud_storage_issue(&error),
                ) {
                    return Some(CloudBackupDetailResult::AccessError(
                        self.offline_error_for_step(BlockingCloudStep::DetailRefresh).to_string(),
                    ));
                }
                return Some(CloudBackupDetailResult::AccessError(error.to_string()));
            }
        };

        let remote_wallet_truth = match self.load_remote_wallet_truth(&wallet_record_ids) {
            Ok(remote_wallet_truth) => remote_wallet_truth,
            Err(error) => return Some(CloudBackupDetailResult::AccessError(error.to_string())),
        };
        self.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

        match self
            .build_cloud_backup_detail_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
        {
            Ok(detail) => Some(CloudBackupDetailResult::Success(detail)),
            Err(error) => Some(CloudBackupDetailResult::AccessError(error.to_string())),
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

            let PersistedCloudBlobState::UploadedPendingConfirmation(pending_state) = &state.state
            else {
                continue;
            };
            if !remote_wallet_revision_matches(
                remote_wallet_truth,
                &state.record_id,
                &pending_state.revision_hash,
            ) {
                continue;
            }

            let confirmed_state = crate::database::cloud_backup::PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash: pending_state.revision_hash.clone(),
                    confirmed_at,
                }),
                ..state.clone()
            };

            let persisted = match table.set_if_current(&state, &confirmed_state) {
                Ok(persisted) => persisted,
                Err(error) => {
                    warn!(
                        "cleanup_confirmed_pending_blobs: persist confirmed record_id={} failed: {error}",
                        confirmed_state.record_id
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
            self.set_pending_upload_verification(self.has_pending_cloud_upload_verification());
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
