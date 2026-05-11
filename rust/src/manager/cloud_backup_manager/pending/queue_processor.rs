use cove_cspp::backup_data::EncryptedWalletBackup;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use super::{MASTER_KEY_UPLOAD_CONFIRMATION_GRACE, PendingUploadVerificationStatus};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobConfirmedState, CloudBlobDirtyState, CloudBlobFailedState,
    CloudBlobFailureIssue, CloudBlobUploadedPendingConfirmationState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::manager::cloud_backup_manager::wallets::WalletBackupReader;
use crate::manager::cloud_backup_manager::{
    PendingUploadVerificationState, RustCloudBackupManager, SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE,
    master_key_wrapper_revision_hash,
};

enum BlobCheckResult {
    Confirmed,
    NotYetUploaded,
    Stale(String),
    AuthorizationRequired { error: String },
    Failed { error: String, retryable: bool, issue: Option<CloudBlobFailureIssue> },
}

impl BlobCheckResult {
    fn terminal_failure(error: String) -> Self {
        Self::Failed { error, retryable: false, issue: None }
    }

    fn cloud_storage_failure(error: CloudStorageError) -> Self {
        if matches!(error, CloudStorageError::AuthorizationRequired(_)) {
            return Self::AuthorizationRequired { error: error.to_string() };
        }

        let retryable = matches!(
            error,
            CloudStorageError::Offline(_)
                | CloudStorageError::NotAvailable(_)
                | CloudStorageError::DownloadFailed(_)
        );

        let issue = RustCloudBackupManager::cloud_blob_failure_issue(error.clone().into());

        Self::Failed { error: error.to_string(), retryable, issue }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingUploadRunOutcome {
    Idle,
    StillPending,
    Confirmed,
    Failed,
    BlockedOnAuthorization,
}

const MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS: u32 = 3;

pub(crate) struct PendingUploadVerifier(pub(crate) RustCloudBackupManager);

impl PendingUploadVerifier {
    pub(crate) async fn run_once(&self) -> PendingUploadVerificationStatus {
        let table = &Database::global().cloud_blob_sync_states;
        let states = match table.list() {
            Ok(states) => states,
            Err(error) => {
                error!("Pending upload verification: failed to read sync states: {error}");
                return PendingUploadVerificationStatus::Pending;
            }
        };

        let mut had_pending = false;
        let mut any_failed = false;
        let mut blocked_on_authorization = false;

        for sync_state in &states {
            let current_state = match &sync_state.state {
                PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
                _ => continue,
            };

            had_pending = true;
            let result = self.check_blob(sync_state, &current_state).await;

            if let BlobCheckResult::AuthorizationRequired { error } = &result {
                warn!(
                    "Pending upload verification: paused until cloud authorization is restored record_id={} error={error}",
                    sync_state.record_id()
                );
                blocked_on_authorization = true;
                break;
            }

            let next_state = Self::apply_blob_result(sync_state, &current_state, &result);
            let persisted = match table.set_if_current(sync_state, &next_state) {
                Ok(persisted) => persisted,
                Err(error) => {
                    error!("Pending upload verification: failed to persist state: {error}");
                    return PendingUploadVerificationStatus::Pending;
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

        if !blocked_on_authorization {
            self.0.finalize_pending_verification_if_ready().await;
        }

        let has_pending = self.0.has_pending_cloud_upload_verification();
        self.send_pending_state(blocked_on_authorization, has_pending);
        self.0.refresh_sync_health();

        let outcome =
            Self::run_outcome(blocked_on_authorization, had_pending, has_pending, any_failed);
        match outcome {
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
            PendingUploadRunOutcome::BlockedOnAuthorization => {}
        }

        match outcome {
            PendingUploadRunOutcome::BlockedOnAuthorization => {
                PendingUploadVerificationStatus::BlockedOnAuthorization
            }
            PendingUploadRunOutcome::StillPending => PendingUploadVerificationStatus::Pending,
            PendingUploadRunOutcome::Idle
            | PendingUploadRunOutcome::Confirmed
            | PendingUploadRunOutcome::Failed => PendingUploadVerificationStatus::Idle,
        }
    }

    fn run_outcome(
        blocked_on_authorization: bool,
        had_pending: bool,
        has_pending: bool,
        any_failed: bool,
    ) -> PendingUploadRunOutcome {
        if blocked_on_authorization {
            return PendingUploadRunOutcome::BlockedOnAuthorization;
        }

        if has_pending {
            return PendingUploadRunOutcome::StillPending;
        }

        if any_failed {
            return PendingUploadRunOutcome::Failed;
        }

        if had_pending {
            return PendingUploadRunOutcome::Confirmed;
        }

        PendingUploadRunOutcome::Idle
    }

    async fn check_blob(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
    ) -> BlobCheckResult {
        if sync_state.is_master_key_wrapper() {
            return self.check_master_key_wrapper(&sync_state.namespace_id, current).await;
        }

        self.check_wallet_blob(sync_state, current).await
    }

    async fn check_master_key_wrapper(
        &self,
        namespace_id: &str,
        current: &CloudBlobUploadedPendingConfirmationState,
    ) -> BlobCheckResult {
        let cloud = CloudStorage::global_silent_client();
        match cloud.download_master_key_backup(namespace_id.to_string()).await {
            Ok(bytes) => {
                let remote_revision = master_key_wrapper_revision_hash(&bytes);
                if remote_revision == current.revision_hash {
                    BlobCheckResult::Confirmed
                } else {
                    warn!(
                        "master key wrapper hash mismatch expected_revision={} actual_revision={remote_revision}",
                        current.revision_hash
                    );
                    BlobCheckResult::NotYetUploaded
                }
            }
            Err(CloudStorageError::NotFound(_)) => BlobCheckResult::NotYetUploaded,
            Err(error) => BlobCheckResult::cloud_storage_failure(error),
        }
    }

    async fn check_wallet_blob(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
    ) -> BlobCheckResult {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = match cspp.load_master_key_from_store() {
            Ok(Some(master_key)) => master_key,
            Ok(None) => {
                return BlobCheckResult::terminal_failure("no local master key available".into());
            }
            Err(error) => {
                return BlobCheckResult::terminal_failure(format!(
                    "load local master key: {error}"
                ));
            }
        };

        let reader = WalletBackupReader::new(
            CloudStorage::global_silent_client(),
            sync_state.namespace_id.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );

        let wallt_download_result = CloudStorage::global_silent_client()
            .download_wallet_backup(
                sync_state.namespace_id.clone(),
                sync_state.record_id().to_string(),
            )
            .await;

        let wallet_json = match wallt_download_result {
            Ok(wallet_json) => wallet_json,
            Err(CloudStorageError::NotFound(_)) => return BlobCheckResult::NotYetUploaded,
            Err(error) => return BlobCheckResult::cloud_storage_failure(error),
        };

        let encrypted: EncryptedWalletBackup = match serde_json::from_slice(&wallet_json) {
            Ok(encrypted) => encrypted,
            Err(error) => {
                return BlobCheckResult::terminal_failure(format!(
                    "deserialize wallet backup: {error}"
                ));
            }
        };
        if encrypted.version != 1 {
            return BlobCheckResult::terminal_failure(format!(
                "unsupported wallet backup version {}",
                encrypted.version
            ));
        }
        if let Err(error) = encrypted.remote_metadata.normalized_wallet(
            &sync_state.namespace_id,
            sync_state.record_id(),
            None,
        ) {
            return BlobCheckResult::terminal_failure(format!("normalize wallet payload: {error}"));
        }

        match reader.decrypt_entry(&encrypted) {
            Ok(entry) => {
                if let Err(error) = encrypted.remote_metadata.normalized_wallet(
                    &sync_state.namespace_id,
                    sync_state.record_id(),
                    Some(entry.wallet_id.as_str()),
                ) {
                    return BlobCheckResult::terminal_failure(format!(
                        "normalize wallet payload: {error}"
                    ));
                }

                if entry.content_revision_hash == current.revision_hash {
                    BlobCheckResult::Confirmed
                } else {
                    BlobCheckResult::Stale(entry.content_revision_hash.clone())
                }
            }
            Err(error) => {
                BlobCheckResult::terminal_failure(format!("decrypt wallet backup: {error}"))
            }
        }
    }

    fn apply_blob_result(
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
        result: &BlobCheckResult,
    ) -> PersistedCloudBlobSyncState {
        let checked_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let next_attempt_count = current.attempt_count + 1;

        let state = match result {
            BlobCheckResult::Confirmed => {
                PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash: current.revision_hash.clone(),
                    confirmed_at: checked_at,
                })
            }
            BlobCheckResult::NotYetUploaded | BlobCheckResult::Stale(_)
                if Self::should_retry_wallet_upload(sync_state, next_attempt_count) =>
            {
                PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: checked_at })
            }
            BlobCheckResult::NotYetUploaded
                if Self::master_key_confirmation_expired(sync_state, current, checked_at) =>
            {
                PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: Some(current.revision_hash.clone()),
                    retryable: false,
                    error: SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into(),
                    issue: None,
                    failed_at: checked_at,
                })
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
            BlobCheckResult::Failed { error, retryable: false, issue } => {
                PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: Some(current.revision_hash.clone()),
                    retryable: false,
                    error: error.clone(),
                    issue: *issue,
                    failed_at: checked_at,
                })
            }
            BlobCheckResult::AuthorizationRequired { .. } => unreachable!(
                "authorization-required results should pause verification without persisting a new state"
            ),
        };

        sync_state.with_state(state)
    }

    fn schedule_retry_if_needed(&self, sync_state: &PersistedCloudBlobSyncState) {
        let wallet_id = match sync_state.record_key() {
            CloudBackupRecordKey::Wallet(wallet_id, _) => wallet_id.clone(),
            CloudBackupRecordKey::MasterKeyWrapper => {
                return;
            }
        };

        if !matches!(sync_state.state, PersistedCloudBlobState::Dirty(_)) {
            return;
        }

        self.0.schedule_wallet_upload(wallet_id, true);
    }

    fn log_blob_result(&self, sync_state: &PersistedCloudBlobSyncState, result: &BlobCheckResult) {
        match (&sync_state.state, result) {
            (PersistedCloudBlobState::Confirmed(_), BlobCheckResult::Confirmed) => {
                info!(
                    "Pending upload verification: confirmed record_id={}",
                    sync_state.record_id()
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::NotYetUploaded,
            ) => {
                info!(
                    "Pending upload verification: not yet uploaded record_id={} attempts={} checked_at={}",
                    sync_state.record_id(),
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default()
                );
            }
            (PersistedCloudBlobState::Dirty(_), BlobCheckResult::NotYetUploaded) => {
                warn!(
                    "Pending upload verification: retrying wallet upload after repeated missing remote confirmation record_id={}",
                    sync_state.record_id(),
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::Stale(remote_revision),
            ) => {
                info!(
                    "Pending upload verification: stale remote revision record_id={} attempts={} checked_at={} expected_revision={} remote_revision={remote_revision}",
                    sync_state.record_id(),
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default(),
                    state.revision_hash
                );
            }
            (PersistedCloudBlobState::Dirty(_), BlobCheckResult::Stale(remote_revision)) => {
                warn!(
                    "Pending upload verification: retrying wallet upload after repeated stale remote revision record_id={} remote_revision={remote_revision}",
                    sync_state.record_id(),
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::Failed { error, .. },
            ) => {
                warn!(
                    "Pending upload verification: check failed record_id={} attempts={} checked_at={} error={error}",
                    sync_state.record_id(),
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default()
                );
            }
            (PersistedCloudBlobState::Failed(_), BlobCheckResult::Failed { error, .. }) => {
                warn!(
                    "Pending upload verification: terminal failure record_id={} error={error}",
                    sync_state.record_id(),
                );
            }
            (
                PersistedCloudBlobState::UploadedPendingConfirmation(state),
                BlobCheckResult::AuthorizationRequired { error },
            ) => {
                warn!(
                    "Pending upload verification: authorization required record_id={} attempts={} checked_at={} error={error}",
                    sync_state.record_id(),
                    state.attempt_count,
                    state.last_checked_at.unwrap_or_default()
                );
            }
            _ => {}
        }
    }

    fn send_pending_state(&self, blocked_on_authorization: bool, pending: bool) {
        if blocked_on_authorization {
            let state = PendingUploadVerificationState::BlockedOnAuthorization;
            self.0.set_pending_upload_verification(state);
            return;
        }

        if pending {
            let state = PendingUploadVerificationState::Confirming;
            self.0.set_pending_upload_verification(state);
            return;
        }

        let state = match self.0.pending_verification_completion() {
            Some(_) => PendingUploadVerificationState::Confirming,
            None => PendingUploadVerificationState::Idle,
        };

        self.0.set_pending_upload_verification(state);
    }

    fn should_retry_wallet_upload(
        sync_state: &PersistedCloudBlobSyncState,
        next_attempt_count: u32,
    ) -> bool {
        sync_state.is_wallet_record()
            && next_attempt_count >= MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS
    }

    fn master_key_confirmation_expired(
        sync_state: &PersistedCloudBlobSyncState,
        current: &CloudBlobUploadedPendingConfirmationState,
        checked_at: u64,
    ) -> bool {
        sync_state.is_master_key_wrapper()
            && checked_at.saturating_sub(current.uploaded_at)
                >= MASTER_KEY_UPLOAD_CONFIRMATION_GRACE.as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::PersistedCloudBlobSyncState;

    fn wallet_pending_blob(attempt_count: u32) -> PersistedCloudBlobSyncState {
        PersistedCloudBlobSyncState::wallet(
            "ns-1".into(),
            "wallet-a".into(),
            "wallet-a".into(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count,
                    last_checked_at: None,
                },
            ),
        )
    }

    #[test]
    fn apply_blob_result_confirms_blob() {
        let blob = wallet_pending_blob(0);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
        };

        let blob =
            PendingUploadVerifier::apply_blob_result(&blob, &current, &BlobCheckResult::Confirmed);

        assert!(matches!(blob.state, PersistedCloudBlobState::Confirmed(_)));
    }

    #[test]
    fn apply_blob_result_tracks_pending_blob() {
        let blob = wallet_pending_blob(0);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
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
    fn apply_blob_result_fails_expired_master_key_confirmation() {
        let checked_at = u64::try_from(jiff::Timestamp::now().as_second()).unwrap_or(0);
        let uploaded_at =
            checked_at.saturating_sub(super::super::MASTER_KEY_UPLOAD_CONFIRMATION_GRACE.as_secs());
        let blob = PersistedCloudBlobSyncState::master_key_wrapper(
            "ns-1".into(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "master-key-wrapper".into(),
                    uploaded_at,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        );

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::NotYetUploaded,
        );

        assert!(matches!(
            blob.state,
            PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. })
        ));
    }

    #[test]
    fn apply_blob_result_keeps_pending_blob_when_remote_revision_is_stale() {
        let blob = wallet_pending_blob(0);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
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
        let blob = wallet_pending_blob(MAX_PENDING_WALLET_UPLOAD_CONFIRMATION_ATTEMPTS - 1);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
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
        let blob = wallet_pending_blob(0);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::Failed { error: "offline".into(), retryable: true, issue: None },
        );

        assert!(matches!(blob.state, PersistedCloudBlobState::UploadedPendingConfirmation(_)));
    }

    #[test]
    fn apply_blob_result_marks_terminal_failures_failed() {
        let blob = wallet_pending_blob(0);

        let current = match &blob.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => state.clone(),
            _ => panic!("expected uploaded pending confirmation state"),
        };

        let blob = PendingUploadVerifier::apply_blob_result(
            &blob,
            &current,
            &BlobCheckResult::Failed { error: "bad data".into(), retryable: false, issue: None },
        );

        assert!(matches!(blob.state, PersistedCloudBlobState::Failed(_)));
    }

    #[test]
    fn cloud_storage_failure_result_retries_offline_errors() {
        let result =
            BlobCheckResult::cloud_storage_failure(CloudStorageError::Offline("offline".into()));

        assert!(matches!(result, BlobCheckResult::Failed { retryable: true, .. }));
    }

    #[test]
    fn run_outcome_treats_failures_as_distinct_from_confirmed() {
        assert_eq!(
            PendingUploadVerifier::run_outcome(false, true, false, true),
            PendingUploadRunOutcome::Failed
        );
        assert_eq!(
            PendingUploadVerifier::run_outcome(false, true, false, false),
            PendingUploadRunOutcome::Confirmed
        );
        assert_eq!(
            PendingUploadVerifier::run_outcome(false, false, false, false),
            PendingUploadRunOutcome::Idle
        );
        assert_eq!(
            PendingUploadVerifier::run_outcome(true, true, true, true),
            PendingUploadRunOutcome::BlockedOnAuthorization
        );
    }
}
