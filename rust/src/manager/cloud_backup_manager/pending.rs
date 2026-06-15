mod detail;
mod queue_processor;

use std::time::Duration;

use act_zero::send;
use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_util::ResultExt as _;

use self::queue_processor::PendingUploadVerifier;
use super::{CloudBackupError, PendingUploadVerificationState, RustCloudBackupManager};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobFailedState, CloudBlobFailureIssue,
    CloudBlobUploadedPendingConfirmationState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};

pub(crate) use detail::remote_wallet_revision_matches;

pub(crate) const MASTER_KEY_UPLOAD_CONFIRMATION_GRACE: Duration = Duration::from_secs(60);
pub(crate) const MAX_PENDING_UPLOAD_VERIFICATION_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingUploadVerificationStatus {
    Idle,
    Pending,
    BlockedOnAuthorization,
}

pub(crate) fn build_pending_upload_backoff() -> backon::FibonacciBackoff {
    FibonacciBuilder::default()
        .with_max_delay(MAX_PENDING_UPLOAD_VERIFICATION_DELAY)
        .without_max_times()
        .build()
}

impl RustCloudBackupManager {
    pub(crate) fn replace_blob_state_if_current(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        next_state: PersistedCloudBlobState,
        error_context: &'static str,
    ) -> Result<bool, CloudBackupError> {
        let next_sync_state = current_state.with_state(next_state);

        Database::global()
            .cloud_blob_sync_states
            .set_if_current(current_state, &next_sync_state)
            .map_err_prefix(error_context, CloudBackupError::Internal)
    }

    pub(crate) fn mark_blob_uploaded_pending_confirmation(
        &self,
        namespace_id: &str,
        record_key: CloudBackupRecordKey,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Result<(), CloudBackupError> {
        let starts_master_key_grace = record_key.is_master_key_wrapper();
        let state = PersistedCloudBlobState::UploadedPendingConfirmation(
            CloudBlobUploadedPendingConfirmationState {
                revision_hash,
                uploaded_at,
                attempt_count: 0,
                last_checked_at: None,
            },
        );
        let sync_state = PersistedCloudBlobSyncState::from_record_key(
            namespace_id.to_string(),
            record_key,
            state,
        );

        Database::global()
            .cloud_blob_sync_states
            .set(&sync_state)
            .map_err_prefix("persist uploaded cloud blob state", CloudBackupError::Internal)?;

        if starts_master_key_grace {
            send!(
                self.supervisor
                    .start_master_key_upload_confirmation_grace(namespace_id.to_string())
            );
        }

        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Confirming);
        self.wake_pending_upload_verifier();
        self.start_pending_upload_verification_loop();

        Ok(())
    }

    pub(crate) fn mark_blob_uploaded_pending_confirmation_if_current(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Result<bool, CloudBackupError> {
        let updated = self.replace_blob_state_if_current(
            current_state,
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash,
                    uploaded_at,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
            "persist uploaded cloud blob state",
        )?;

        if !updated {
            return Ok(false);
        }

        if current_state.is_master_key_wrapper() {
            send!(
                self.supervisor
                    .start_master_key_upload_confirmation_grace(current_state.namespace_id.clone())
            );
        }

        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Confirming);
        self.wake_pending_upload_verifier();
        self.start_pending_upload_verification_loop();

        Ok(true)
    }

    pub(crate) fn mark_blob_failed_if_current(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: Option<String>,
        retryable: bool,
        issue: Option<CloudBlobFailureIssue>,
        error: String,
    ) -> Result<bool, CloudBackupError> {
        let failed_at = crate::manager::cloud_backup_manager::current_timestamp();

        self.replace_blob_state_if_current(
            current_state,
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash,
                retryable,
                error,
                issue,
                failed_at,
            }),
            "persist failed cloud blob state",
        )
    }

    pub(crate) fn mark_blob_dirty_state(
        &self,
        current_state: &PersistedCloudBlobSyncState,
    ) -> Result<(), CloudBackupError> {
        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();
        let dirty_state = current_state
            .with_state(PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }));

        Database::global()
            .cloud_blob_sync_states
            .set(&dirty_state)
            .map_err_prefix("persist dirty cloud blob state", CloudBackupError::Internal)
    }

    pub(crate) fn remove_blob_sync_states<I>(&self, record_ids: I) -> Result<(), CloudBackupError>
    where
        I: IntoIterator<Item = String>,
    {
        let table = &Database::global().cloud_blob_sync_states;

        for record_id in record_ids {
            table
                .delete(&record_id)
                .map_err_prefix("remove cloud blob sync state", CloudBackupError::Internal)?;
        }

        self.refresh_pending_upload_verification_state();
        self.wake_pending_upload_verifier();

        Ok(())
    }

    pub(crate) fn start_pending_upload_verification_loop(&self) {
        send!(self.supervisor.ensure_pending_upload_verification_loop());
    }

    pub(crate) async fn verify_pending_uploads_once(&self) -> PendingUploadVerificationStatus {
        PendingUploadVerifier(self.clone()).run_once().await
    }

    pub(crate) fn wake_pending_upload_verifier(&self) {
        send!(self.supervisor.wake_pending_upload_verifier());
    }
}
