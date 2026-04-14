use cove_cspp::backup_data::EncryptedWalletBackup;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use super::super::RustCloudBackupManager;
use super::super::wallets::WalletBackupReader;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobConfirmedState, CloudBlobDirtyState, CloudBlobFailedState,
    CloudBlobUploadedPendingConfirmationState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};

enum BlobCheckResult {
    Confirmed,
    NotYetUploaded,
    Stale(String),
    Failed { error: String, retryable: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingUploadRunOutcome {
    Idle,
    StillPending,
    Confirmed,
    Failed,
}

pub(super) struct PendingUploadVerifier(pub(super) RustCloudBackupManager);

const MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS: u32 = 3;

impl PendingUploadVerifier {
    pub(super) fn run_once(&self) -> bool {
        let table = &Database::global().cloud_blob_sync_states;
        let states = match table.list() {
            Ok(states) => states,
            Err(error) => {
                error!("Pending upload verification: failed to read sync states: {error}");
                return true;
            }
        };

        let mut had_pending = false;
        let mut any_failed = false;
        for sync_state in &states {
            let PersistedCloudBlobState::UploadedPendingConfirmation(state) = &sync_state.state
            else {
                continue;
            };
            let current_state = state.clone();

            had_pending = true;
            let result = self.check_blob(sync_state, &current_state);
            let next_state = Self::apply_blob_result(sync_state, &current_state, &result);
            let persisted = match table.set_if_current(sync_state, &next_state) {
                Ok(persisted) => persisted,
                Err(error) => {
                    error!("Pending upload verification: failed to persist state: {error}");
                    return true;
                }
            };
            if !persisted {
                continue;
            }

            if matches!(next_state.state, PersistedCloudBlobState::Failed(_)) {
                any_failed = true;
            }

            self.log_blob_result(&next_state, &result);
            self.schedule_retry_if_needed(&next_state);
        }

        self.0.finalize_pending_verification_if_ready();
        let has_pending = self.0.has_pending_cloud_upload_verification();
        self.send_pending_state(has_pending);
        self.0.refresh_sync_health();
        match Self::run_outcome(had_pending, has_pending, any_failed) {
            PendingUploadRunOutcome::Idle => {
                info!("Pending upload verification: no pending blobs");
            }
            PendingUploadRunOutcome::StillPending => {
                info!("Pending upload verification: still pending");
            }
            PendingUploadRunOutcome::Confirmed => {
                info!("Pending upload verification: all blobs confirmed");
            }
            PendingUploadRunOutcome::Failed => {
                warn!("Pending upload verification: completed with failures");
            }
        }

        has_pending
    }

    fn run_outcome(
        had_pending: bool,
        has_pending: bool,
        any_failed: bool,
    ) -> PendingUploadRunOutcome {
        if has_pending {
            PendingUploadRunOutcome::StillPending
        } else if any_failed {
            PendingUploadRunOutcome::Failed
        } else if had_pending {
            PendingUploadRunOutcome::Confirmed
        } else {
            PendingUploadRunOutcome::Idle
        }
    }

    fn check_blob(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
    ) -> BlobCheckResult {
        if sync_state.wallet_id.is_none() {
            return self.check_master_key_wrapper(&sync_state.namespace_id);
        }

        self.check_wallet_blob(sync_state, current)
    }

    fn check_master_key_wrapper(&self, namespace_id: &str) -> BlobCheckResult {
        let cloud = CloudStorage::global();
        match cloud.download_master_key_backup(namespace_id.to_string()) {
            Ok(_) => BlobCheckResult::Confirmed,
            Err(CloudStorageError::NotFound(_)) => BlobCheckResult::NotYetUploaded,
            Err(error) => cloud_storage_failure_result(error),
        }
    }

    fn check_wallet_blob(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
    ) -> BlobCheckResult {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = match cspp.load_master_key_from_store() {
            Ok(Some(master_key)) => master_key,
            Ok(None) => return terminal_failure("no local master key available".into()),
            Err(error) => {
                return terminal_failure(format!("load local master key: {error}"));
            }
        };

        let reader = WalletBackupReader::new(
            CloudStorage::global().clone(),
            sync_state.namespace_id.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let wallet_json = match CloudStorage::global()
            .download_wallet_backup(sync_state.namespace_id.clone(), sync_state.record_id.clone())
        {
            Ok(wallet_json) => wallet_json,
            Err(CloudStorageError::NotFound(_)) => return BlobCheckResult::NotYetUploaded,
            Err(error) => return cloud_storage_failure_result(error),
        };

        let encrypted: EncryptedWalletBackup = match serde_json::from_slice(&wallet_json) {
            Ok(encrypted) => encrypted,
            Err(error) => {
                return terminal_failure(format!("deserialize wallet backup: {error}"));
            }
        };
        if encrypted.version != 1 {
            return terminal_failure(format!(
                "unsupported wallet backup version {}",
                encrypted.version
            ));
        }

        match reader.decrypt_entry(&encrypted) {
            Ok(entry) => {
                if entry.content_revision_hash == current.revision_hash {
                    BlobCheckResult::Confirmed
                } else {
                    BlobCheckResult::Stale(entry.content_revision_hash.clone())
                }
            }
            Err(error) => terminal_failure(format!("decrypt wallet backup: {error}")),
        }
    }

    fn apply_blob_result(
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
        result: &BlobCheckResult,
    ) -> PersistedCloudBlobSyncState {
        let checked_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let next_attempt_count = current.attempt_count + 1;

        PersistedCloudBlobSyncState {
            state: match result {
                BlobCheckResult::Confirmed => {
                    PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                        revision_hash: current.revision_hash.clone(),
                        confirmed_at: checked_at,
                    })
                }
                BlobCheckResult::NotYetUploaded | BlobCheckResult::Stale(_)
                    if should_retry_wallet_upload(sync_state, next_attempt_count) =>
                {
                    PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: checked_at })
                }
                BlobCheckResult::NotYetUploaded
                | BlobCheckResult::Stale(_)
                | BlobCheckResult::Failed { retryable: true, .. } => {
                    PersistedCloudBlobState::UploadedPendingConfirmation(
                        CloudBlobUploadedPendingConfirmationState {
                            revision_hash: current.revision_hash.clone(),
                            uploaded_at: current.uploaded_at,
                            attempt_count: next_attempt_count,
                            last_checked_at: Some(checked_at),
                        },
                    )
                }
                BlobCheckResult::Failed { error, retryable: false } => {
                    PersistedCloudBlobState::Failed(CloudBlobFailedState {
                        revision_hash: Some(current.revision_hash.clone()),
                        retryable: false,
                        error: error.clone(),
                        failed_at: checked_at,
                    })
                }
            },
            ..sync_state.clone()
        }
    }

    fn schedule_retry_if_needed(&self, sync_state: &PersistedCloudBlobSyncState) {
        let Some(wallet_id) = sync_state.wallet_id.clone() else {
            return;
        };

        if !matches!(sync_state.state, PersistedCloudBlobState::Dirty(_)) {
            return;
        }

        self.0.schedule_wallet_upload(wallet_id, true);
    }

    fn log_blob_result(&self, sync_state: &PersistedCloudBlobSyncState, result: &BlobCheckResult) {
        match (&sync_state.state, result) {
            (PersistedCloudBlobState::Confirmed(_), BlobCheckResult::Confirmed) => {
                info!("Pending upload verification: confirmed record_id={}", sync_state.record_id);
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::NotYetUploaded,
            ) => {
                info!(
                    "Pending upload verification: not yet uploaded record_id={} attempts={} checked_at={}",
                    sync_state.record_id,
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default()
                );
            }
            (PersistedCloudBlobState::Dirty(_), BlobCheckResult::NotYetUploaded) => {
                warn!(
                    "Pending upload verification: retrying wallet upload after repeated missing remote confirmation record_id={}",
                    sync_state.record_id,
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::Stale(remote_revision),
            ) => {
                info!(
                    "Pending upload verification: stale remote revision record_id={} attempts={} checked_at={} expected_revision={} remote_revision={remote_revision}",
                    sync_state.record_id,
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default(),
                    state.revision_hash
                );
            }
            (PersistedCloudBlobState::Dirty(_), BlobCheckResult::Stale(remote_revision)) => {
                warn!(
                    "Pending upload verification: retrying wallet upload after repeated stale remote revision record_id={} remote_revision={remote_revision}",
                    sync_state.record_id,
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::Failed { error, .. },
            ) => {
                warn!(
                    "Pending upload verification: check failed record_id={} attempts={} checked_at={} error={error}",
                    sync_state.record_id,
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default()
                );
            }
            (PersistedCloudBlobState::Failed(_), BlobCheckResult::Failed { error, .. }) => {
                warn!(
                    "Pending upload verification: terminal failure record_id={} error={error}",
                    sync_state.record_id,
                );
            }
            _ => {}
        }
    }

    fn send_pending_state(&self, pending: bool) {
        self.0.set_pending_upload_verification(pending);
    }
}

fn should_retry_wallet_upload(
    sync_state: &PersistedCloudBlobSyncState,
    next_attempt_count: u32,
) -> bool {
    sync_state.wallet_id.is_some()
        && next_attempt_count >= MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS
}

fn terminal_failure(error: String) -> BlobCheckResult {
    BlobCheckResult::Failed { error, retryable: false }
}

fn cloud_storage_failure_result(error: CloudStorageError) -> BlobCheckResult {
    let retryable = matches!(
        error,
        CloudStorageError::Offline(_)
            | CloudStorageError::NotAvailable(_)
            | CloudStorageError::DownloadFailed(_)
    );

    BlobCheckResult::Failed { error: error.to_string(), retryable }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{CloudUploadKind, PersistedCloudBlobSyncState};

    #[test]
    fn apply_blob_result_confirms_blob() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: None,
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob =
            PendingUploadVerifier::apply_blob_result(&blob, &current, &BlobCheckResult::Confirmed);

        assert!(matches!(blob.state, PersistedCloudBlobState::Confirmed(_)));
    }

    #[test]
    fn apply_blob_result_tracks_pending_blob() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: None,
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::NotYetUploaded,
        );

        let PersistedCloudBlobState::UploadedPendingConfirmation(state) = &blob.state else {
            panic!("expected uploaded pending confirmation state");
        };

        assert_eq!(state.attempt_count, 1);
        assert!(state.last_checked_at.is_some());
    }

    #[test]
    fn apply_blob_result_keeps_pending_blob_when_remote_revision_is_stale() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: Some("wallet-a".into()),
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::Stale("rev-0".into()),
        );

        let PersistedCloudBlobState::UploadedPendingConfirmation(state) = &blob.state else {
            panic!("expected uploaded pending confirmation state");
        };

        assert_eq!(state.revision_hash, "rev-1");
        assert_eq!(state.attempt_count, 1);
        assert!(state.last_checked_at.is_some());
    }

    #[test]
    fn apply_blob_result_retries_wallet_upload_after_threshold() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: Some("wallet-a".into()),
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS - 1,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::NotYetUploaded,
        );

        assert!(matches!(blob.state, PersistedCloudBlobState::Dirty(_)));
    }

    #[test]
    fn apply_blob_result_keeps_retryable_failures_pending() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: Some("wallet-a".into()),
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::Failed { error: "offline".into(), retryable: true },
        );

        assert!(matches!(blob.state, PersistedCloudBlobState::UploadedPendingConfirmation(_)));
    }

    #[test]
    fn apply_blob_result_marks_terminal_failures_failed() {
        let blob = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: Some("wallet-a".into()),
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => unreachable!(),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::Failed { error: "bad data".into(), retryable: false },
        );

        assert!(matches!(blob.state, PersistedCloudBlobState::Failed(_)));
    }

    #[test]
    fn cloud_storage_failure_result_retries_offline_errors() {
        let result = cloud_storage_failure_result(CloudStorageError::Offline("offline".into()));

        assert!(matches!(result, BlobCheckResult::Failed { retryable: true, .. }));
    }

    #[test]
    fn run_outcome_treats_failures_as_distinct_from_confirmed() {
        assert_eq!(
            PendingUploadVerifier::run_outcome(true, false, true),
            PendingUploadRunOutcome::Failed
        );
        assert_eq!(
            PendingUploadVerifier::run_outcome(true, false, false),
            PendingUploadRunOutcome::Confirmed
        );
        assert_eq!(
            PendingUploadVerifier::run_outcome(false, false, false),
            PendingUploadRunOutcome::Idle
        );
    }
}
