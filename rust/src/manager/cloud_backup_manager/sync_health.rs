use std::collections::HashSet;

use cove_cspp::backup_data::{MASTER_KEY_RECORD_ID, wallet_record_id};
use cove_device::cloud_storage::{CloudStorage, CloudStorageError, CloudSyncHealth};
use futures::TryStreamExt as _;
use futures::stream::{self, StreamExt as _};

use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupStore, RustCloudBackupManager,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobFailureIssue, PersistedCloudBlobState, PersistedCloudBlobSyncState,
};

pub(crate) const SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE: &str =
    "master key backup is missing from cloud storage";

impl RustCloudBackupManager {
    pub(crate) async fn compute_sync_health(&self) -> CloudSyncHealth {
        self.compute_sync_health_with_master_key_grace(None).await
    }

    pub(crate) async fn compute_sync_health_with_master_key_grace(
        &self,
        master_key_upload_grace_namespace: Option<&str>,
    ) -> CloudSyncHealth {
        if !Self::load_persisted_state().is_configured() {
            return CloudSyncHealth::Unknown;
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => return CloudSyncHealth::Failed(error.to_string()),
        };
        let expected_wallet_record_ids = match self.expected_wallet_record_ids().await {
            Ok(record_ids) => record_ids,
            Err(error) => return CloudSyncHealth::Failed(error.to_string()),
        };
        let sync_states = match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => {
                if let Some(sync_health) = Self::sync_health_from_corrupt_sync_state(&states) {
                    return sync_health;
                }

                states
                    .into_iter()
                    .filter(|state| {
                        state.namespace_id == namespace
                            && (state.wallet_id().is_none()
                                || expected_wallet_record_ids.contains(state.record_id()))
                    })
                    .collect::<Vec<_>>()
            }
            Err(error) => {
                return CloudSyncHealth::Failed(format!(
                    "failed to read cloud backup sync states: {error}",
                ));
            }
        };

        if let Some(sync_health) = Self::sync_health_from_local_failures(&sync_states) {
            return sync_health;
        }

        if master_key_upload_grace_namespace == Some(namespace.as_str()) {
            return CloudSyncHealth::Uploading;
        }

        if Self::sync_health_has_pending_master_key_upload(&sync_states) {
            return CloudSyncHealth::Uploading;
        }

        let cloud = CloudStorage::global_silent_client();
        let master_key_uploaded = match cloud
            .is_backup_uploaded(namespace.clone(), MASTER_KEY_RECORD_ID.to_string())
            .await
        {
            Ok(is_uploaded) => is_uploaded,
            Err(CloudStorageError::NotFound(_)) => false,
            Err(error) => return Self::sync_health_from_cloud_error(error),
        };

        if expected_wallet_record_ids.is_empty() {
            if master_key_uploaded {
                return CloudSyncHealth::AllUploaded;
            }

            return CloudSyncHealth::NoFiles;
        }

        if !master_key_uploaded {
            return CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into());
        }

        if Self::sync_health_has_pending_wallet_upload(&sync_states) {
            return CloudSyncHealth::Uploading;
        }

        let remote_wallet_record_ids = match cloud.list_wallet_backups(namespace).await {
            Ok(record_ids) => record_ids.into_iter().collect::<HashSet<_>>(),
            Err(CloudStorageError::NotFound(_)) => HashSet::new(),
            Err(error) => return Self::sync_health_from_cloud_error(error),
        };

        let missing_wallet_count = expected_wallet_record_ids
            .iter()
            .filter(|record_id| !remote_wallet_record_ids.contains(*record_id))
            .count();
        if missing_wallet_count > 0 {
            return CloudSyncHealth::Failed(sync_health_missing_wallet_message(
                missing_wallet_count,
            ));
        }

        CloudSyncHealth::AllUploaded
    }

    pub(crate) async fn expected_wallet_record_ids(
        &self,
    ) -> Result<HashSet<String>, CloudBackupError> {
        let local_wallets = CloudBackupStore::global().all_wallets()?;
        let record_ids =
            stream::iter(local_wallets)
                .map(|wallet| async move {
                    Ok::<_, CloudBackupError>(wallet_record_id(wallet.id.as_ref()))
                })
                .buffered(CLOUD_BACKUP_IO_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .await?;

        Ok(record_ids.into_iter().collect())
    }

    fn sync_health_from_local_failures(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Option<CloudSyncHealth> {
        if let Some(sync_health) = sync_states.iter().find_map(|sync_state| {
            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return None;
            };

            if failed_state.issue == Some(CloudBlobFailureIssue::AuthorizationRequired) {
                return Some(CloudSyncHealth::AuthorizationRequired(sync_health_failed_message(
                    sync_state,
                    failed_state,
                )));
            }

            None
        }) {
            return Some(sync_health);
        }

        sync_states.iter().find_map(|sync_state| {
            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return None;
            };

            Some(CloudSyncHealth::Failed(sync_health_failed_message(sync_state, failed_state)))
        })
    }

    fn sync_health_from_corrupt_sync_state(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Option<CloudSyncHealth> {
        sync_states.iter().find_map(|sync_state| {
            if !sync_state.is_corrupted() {
                return None;
            }

            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return Some(CloudSyncHealth::Failed(
                    "cloud backup sync state could not be decoded".into(),
                ));
            };

            Some(CloudSyncHealth::Failed(sync_health_failed_message(sync_state, failed_state)))
        })
    }

    fn sync_health_has_pending_wallet_upload(sync_states: &[PersistedCloudBlobSyncState]) -> bool {
        sync_states.iter().any(|sync_state| {
            sync_state.is_wallet_record()
                && matches!(
                    sync_state.state,
                    PersistedCloudBlobState::Dirty(_)
                        | PersistedCloudBlobState::Uploading(_)
                        | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                )
        })
    }

    fn sync_health_has_pending_master_key_upload(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> bool {
        sync_states.iter().any(|sync_state| {
            sync_state.is_master_key_wrapper()
                && sync_state.record_id() == MASTER_KEY_RECORD_ID
                && matches!(
                    sync_state.state,
                    PersistedCloudBlobState::Dirty(_)
                        | PersistedCloudBlobState::Uploading(_)
                        | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                )
        })
    }

    fn sync_health_from_cloud_error(error: CloudStorageError) -> CloudSyncHealth {
        match error {
            CloudStorageError::AuthorizationRequired(message) => {
                CloudSyncHealth::AuthorizationRequired(message)
            }
            CloudStorageError::NotAvailable(_) => CloudSyncHealth::Unavailable,
            CloudStorageError::Offline(message) => CloudSyncHealth::Failed(message),
            CloudStorageError::QuotaExceeded => {
                CloudSyncHealth::Failed("cloud storage quota was exceeded".into())
            }
            CloudStorageError::UploadFailed(message)
            | CloudStorageError::DownloadFailed(message)
            | CloudStorageError::NotFound(message)
            | CloudStorageError::InvalidNamespace(message) => CloudSyncHealth::Failed(message),
        }
    }
}

fn sync_health_failed_message(
    sync_state: &PersistedCloudBlobSyncState,
    failed_state: &crate::database::cloud_backup::CloudBlobFailedState,
) -> String {
    if failed_state.error.is_empty() {
        return format!("cloud backup upload failed for record_id={}", sync_state.record_id());
    }

    failed_state.error.clone()
}

fn sync_health_missing_wallet_message(missing_wallet_count: usize) -> String {
    if missing_wallet_count == 1 {
        return "1 wallet backup is missing from cloud storage".into();
    }

    format!("{missing_wallet_count} wallet backups are missing from cloud storage")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::CloudBlobFailedState;

    #[test]
    fn sync_health_from_local_failures_prefers_authorization_required() {
        let generic_failure = PersistedCloudBlobSyncState::wallet(
            "namespace".into(),
            "generic".into(),
            "generic".into(),
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                retryable: true,
                error: "generic failure".into(),
                issue: None,
                failed_at: 1,
            }),
        );
        let authorization_failure = PersistedCloudBlobSyncState::wallet(
            "namespace".into(),
            "authorization".into(),
            "authorization".into(),
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                retryable: true,
                error: "authorization required".into(),
                issue: Some(CloudBlobFailureIssue::AuthorizationRequired),
                failed_at: 2,
            }),
        );

        assert_eq!(
            RustCloudBackupManager::sync_health_from_local_failures(&[
                generic_failure,
                authorization_failure
            ]),
            Some(CloudSyncHealth::AuthorizationRequired("authorization required".into())),
        );
    }
}
