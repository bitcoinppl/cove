use std::collections::HashSet;

use cove_cspp::backup_data::{MASTER_KEY_RECORD_ID, wallet_record_id};
use cove_device::cloud_storage::{
    CloudBackupUploadStatus, CloudStorage, CloudStorageError, CloudSyncHealth,
};
use futures::TryStreamExt as _;
use futures::stream::{self, StreamExt as _};

use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupStore,
    GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, RustCloudBackupManager,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudStorageIssue, PersistedCloudBlobState, PersistedCloudBlobSyncState,
};

// Sync Now uploads only wallets, so it can't fix the key; Verify rebuilds a wrapper the provider
// doesn't have from the local master key
pub(crate) const SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE: &str = "Cove couldn't confirm the key for this cloud backup. Open Cloud Backup and run Verify to fix it.";

// only iCloud Drive reports an upload as pending; the system's iCloud process runs it, so Cove
// needn't stay open, and Verify reads the local copy back and has nothing to repair. iCloud also
// reports an upload it couldn't finish, such as with full storage, as pending
const SYNC_HEALTH_UPLOADING_MASTER_KEY_MESSAGE: &str = "iCloud hasn't finished uploading the key for this cloud backup. Check that you're online, iCloud Drive is on for Cove and your iCloud storage isn't full, then try again.";

/// Why the provider can't confirm Cloud Backup's master key wrapper, which decides the fix
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MasterKeyIssue {
    /// The provider doesn't have the wrapper, which Verify rebuilds from the local master key
    Missing,
    /// iCloud has the wrapper locally but hasn't finished uploading it
    Uploading,
}

impl MasterKeyIssue {
    fn message(self) -> &'static str {
        match self {
            Self::Missing => SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE,
            Self::Uploading => SYNC_HEALTH_UPLOADING_MASTER_KEY_MESSAGE,
        }
    }
}

/// The provider can't confirm the master key wrapper of the backup in `namespace`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnconfirmedMasterKey {
    pub(crate) namespace: String,
    pub(crate) issue: MasterKeyIssue,
}

/// What a sync health check learned about the master key wrapper at the provider
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MasterKeyFinding {
    /// The check stopped before asking the provider, for example offline, so it learned nothing
    /// and the last answer still stands
    NotAsked,
    /// The provider has the wrapper uploaded
    Confirmed,
    Unconfirmed(UnconfirmedMasterKey),
}

/// What a sync health check found, for Settings and for the code that acts on it
///
/// Settings only shows the health; the master key answer stays typed so no caller has to match
/// the message it shows
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncHealthCheck {
    health: CloudSyncHealth,
    master_key: MasterKeyFinding,
}

impl SyncHealthCheck {
    /// The provider can't confirm the master key wrapper, so no cloud copy can be decrypted
    pub(crate) fn master_key_unconfirmed(namespace: String, issue: MasterKeyIssue) -> Self {
        Self {
            health: CloudSyncHealth::Failed(issue.message().into()),
            master_key: MasterKeyFinding::Unconfirmed(UnconfirmedMasterKey { namespace, issue }),
        }
    }

    /// A health found after the provider confirmed the master key wrapper
    fn key_confirmed(health: CloudSyncHealth) -> Self {
        Self { health, master_key: MasterKeyFinding::Confirmed }
    }

    pub(crate) fn health(&self) -> &CloudSyncHealth {
        &self.health
    }

    pub(crate) fn into_parts(self) -> (CloudSyncHealth, MasterKeyFinding) {
        (self.health, self.master_key)
    }
}

impl From<CloudSyncHealth> for SyncHealthCheck {
    fn from(health: CloudSyncHealth) -> Self {
        Self { health, master_key: MasterKeyFinding::NotAsked }
    }
}

impl RustCloudBackupManager {
    pub(crate) async fn check_sync_health(
        &self,
        master_key_upload_grace_namespace: Option<&str>,
    ) -> SyncHealthCheck {
        if !Self::load_persisted_state().is_configured() {
            return CloudSyncHealth::Unknown.into();
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => return CloudSyncHealth::Failed(error.reader_message()).into(),
        };
        let expected_wallet_record_ids = match self.expected_wallet_record_ids().await {
            Ok(record_ids) => record_ids,
            Err(error) => return CloudSyncHealth::Failed(error.reader_message()).into(),
        };
        let sync_states = match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => {
                if let Some(sync_health) = Self::sync_health_from_corrupt_sync_state(&states) {
                    return sync_health.into();
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
                tracing::error!("Failed to read cloud backup sync states: {error}");
                return CloudSyncHealth::Failed(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into()).into();
            }
        };

        let local_failure = Self::sync_health_from_local_failures(&sync_states);
        let master_key_uploading = master_key_upload_grace_namespace == Some(namespace.as_str())
            || Self::sync_health_has_pending_master_key_upload(&sync_states);
        if master_key_uploading {
            return local_failure.unwrap_or(CloudSyncHealth::Uploading).into();
        }

        // a failed row, the wrapper's own included, still asks the provider for the key: the
        // Wipe Data PIN counts cloud copies only while it confirms the key, and a failed wrapper
        // row's fix depends on whether the provider lost the file or is still uploading it
        let cloud = CloudStorage::global_silent_client();
        let master_key_issue = match cloud
            .is_backup_uploaded(namespace.clone(), MASTER_KEY_RECORD_ID.to_string())
            .await
        {
            Ok(CloudBackupUploadStatus::Uploaded) => None,
            Ok(CloudBackupUploadStatus::Pending) => Some(MasterKeyIssue::Uploading),
            Ok(CloudBackupUploadStatus::NotFound) | Err(CloudStorageError::NotFound(_)) => {
                Some(MasterKeyIssue::Missing)
            }
            Err(error) => {
                return local_failure
                    .unwrap_or_else(|| Self::sync_health_from_cloud_error(error))
                    .into();
            }
        };

        // Settings keeps showing the failed row, which may ask the user to reconnect, while the
        // key answer still reaches the Wipe Data PIN check
        if let Some(health) = local_failure {
            let master_key = match master_key_issue {
                Some(issue) => {
                    MasterKeyFinding::Unconfirmed(UnconfirmedMasterKey { namespace, issue })
                }
                None => MasterKeyFinding::Confirmed,
            };

            return SyncHealthCheck { health, master_key };
        }

        if let Some(issue) = master_key_issue {
            if !expected_wallet_record_ids.is_empty() {
                return SyncHealthCheck::master_key_unconfirmed(namespace, issue);
            }

            // with no wallets Settings shows no files, but a wallet added before the next check
            // still has no key to decrypt its copy
            return SyncHealthCheck {
                health: CloudSyncHealth::NoFiles,
                master_key: MasterKeyFinding::Unconfirmed(UnconfirmedMasterKey {
                    namespace,
                    issue,
                }),
            };
        }

        if expected_wallet_record_ids.is_empty() {
            return SyncHealthCheck::key_confirmed(CloudSyncHealth::AllUploaded);
        }

        if Self::sync_health_has_pending_wallet_upload(&sync_states) {
            return SyncHealthCheck::key_confirmed(CloudSyncHealth::Uploading);
        }

        let remote_wallet_record_ids = match cloud.list_wallet_backups(namespace).await {
            Ok(record_ids) => record_ids.into_iter().collect::<HashSet<_>>(),
            Err(CloudStorageError::NotFound(_)) => HashSet::new(),
            Err(error) => {
                return SyncHealthCheck::key_confirmed(Self::sync_health_from_cloud_error(error));
            }
        };

        let missing_wallet_count = expected_wallet_record_ids
            .iter()
            .filter(|record_id| !remote_wallet_record_ids.contains(*record_id))
            .count();
        if missing_wallet_count > 0 {
            return SyncHealthCheck::key_confirmed(CloudSyncHealth::Failed(
                sync_health_missing_wallet_message(missing_wallet_count),
            ));
        }

        SyncHealthCheck::key_confirmed(CloudSyncHealth::AllUploaded)
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

            if failed_state.issue == Some(CloudStorageIssue::AuthorizationRequired) {
                return Some(CloudSyncHealth::AuthorizationRequired(sync_health_reader_message(
                    failed_state.issue,
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

            Some(CloudSyncHealth::Failed(sync_health_reader_message(failed_state.issue)))
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
                return Some(CloudSyncHealth::Failed(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into()));
            };

            Some(CloudSyncHealth::Failed(sync_health_reader_message(failed_state.issue)))
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
        let authorization_required = matches!(error, CloudStorageError::AuthorizationRequired(_));
        let unavailable = matches!(error, CloudStorageError::NotAvailable(_));
        let message = CloudBackupError::from(error).reader_message();

        if authorization_required {
            CloudSyncHealth::AuthorizationRequired(message)
        } else if unavailable {
            CloudSyncHealth::Unavailable
        } else {
            CloudSyncHealth::Failed(message)
        }
    }
}

fn sync_health_reader_message(issue: Option<CloudStorageIssue>) -> String {
    let error = match issue {
        Some(CloudStorageIssue::AuthorizationRequired) => {
            CloudStorageError::AuthorizationRequired(String::new())
        }
        Some(CloudStorageIssue::Offline) => CloudStorageError::Offline(String::new()),
        Some(CloudStorageIssue::SyncPending) => CloudStorageError::SyncPending(String::new()),
        Some(CloudStorageIssue::Unavailable) => CloudStorageError::NotAvailable(String::new()),
        Some(CloudStorageIssue::NotFound) => CloudStorageError::NotFound(String::new()),
        Some(CloudStorageIssue::QuotaExceeded) => CloudStorageError::QuotaExceeded,
        Some(CloudStorageIssue::Other) | None => {
            return GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into();
        }
    };

    CloudBackupError::from(error).reader_message()
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
                issue: Some(CloudStorageIssue::AuthorizationRequired),
                failed_at: 2,
            }),
        );

        assert_eq!(
            RustCloudBackupManager::sync_health_from_local_failures(&[
                generic_failure,
                authorization_failure
            ]),
            Some(CloudSyncHealth::AuthorizationRequired(sync_health_reader_message(Some(
                CloudStorageIssue::AuthorizationRequired
            )),)),
        );
    }

    #[test]
    fn sync_health_does_not_publish_persisted_or_provider_diagnostics() {
        let diagnostic = "account=user@example.com namespace=secret record_id=private";
        let failed = PersistedCloudBlobSyncState::wallet(
            "secret".into(),
            "private".into(),
            "wallet".into(),
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                retryable: true,
                error: diagnostic.into(),
                issue: None,
                failed_at: 1,
            }),
        );

        assert_eq!(
            RustCloudBackupManager::sync_health_from_local_failures(&[failed]),
            Some(CloudSyncHealth::Failed(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into()))
        );
        assert_eq!(
            RustCloudBackupManager::sync_health_from_cloud_error(
                CloudStorageError::DownloadFailed(diagnostic.into())
            ),
            CloudSyncHealth::Failed(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into())
        );
    }
}
