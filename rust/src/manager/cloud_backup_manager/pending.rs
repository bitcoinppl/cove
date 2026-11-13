mod detail;
mod queue_processor;
mod remote_evidence;

use std::time::Duration;

use act_zero::send;
use backon::{BackoffBuilder as _, FibonacciBuilder};
use tokio::sync::Mutex;

use self::queue_processor::PendingUploadVerifier;
use super::{CloudBackupError, PendingUploadVerificationState, RustCloudBackupManager};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobFailedState,
    CloudBlobUploadedPendingConfirmationState, CloudStorageIssue, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};

pub(crate) use detail::remote_wallet_revision_matches;
pub(crate) use remote_evidence::RemoteRevisionEvidence;

pub(crate) const MASTER_KEY_UPLOAD_CONFIRMATION_GRACE: Duration = Duration::from_secs(60);
pub(crate) const MAX_PENDING_UPLOAD_VERIFICATION_DELAY: Duration = Duration::from_secs(10);

static PENDING_UPLOAD_VERIFIER_LOCK: Mutex<()> = Mutex::const_new(());

pub(crate) fn build_pending_upload_backoff() -> backon::FibonacciBackoff {
    FibonacciBuilder::default()
        .with_max_delay(MAX_PENDING_UPLOAD_VERIFICATION_DELAY)
        .without_max_times()
        .build()
}

impl RustCloudBackupManager {
    /// Hands remote revision evidence to the supervisor, the one owner that confirms uploads
    /// from it
    ///
    /// Sent rather than awaited: an Enable recovery refreshes the detail inside a supervisor
    /// handler, so waiting for the supervisor here would deadlock it
    pub(crate) fn confirm_remote_revisions(&self, evidence: RemoteRevisionEvidence) {
        send!(self.supervisor.confirm_remote_revisions(evidence));
    }

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
            .map_err(|source| CloudBackupError::internal_context(error_context, source))
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

        Database::global().cloud_blob_sync_states.set(&sync_state).map_err(|source| {
            CloudBackupError::internal_context("persist uploaded cloud blob state", source)
        })?;

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
        issue: Option<CloudStorageIssue>,
        error: String,
    ) -> Result<bool, CloudBackupError> {
        let failed_at = cove_util::time::unix_timestamp_secs_or_zero();

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

    /// Puts an upload's row back to Dirty, only while the row is still the one the upload wrote
    ///
    /// A reset or a newer upload may have replaced or removed the row while the upload ran, and
    /// the newer row wins
    pub(crate) fn mark_blob_dirty_if_current(
        &self,
        current_state: &PersistedCloudBlobSyncState,
    ) -> Result<bool, CloudBackupError> {
        let changed_at = cove_util::time::unix_timestamp_secs_or_zero();

        self.replace_blob_state_if_current(
            current_state,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            "persist dirty cloud blob state",
        )
    }

    pub(crate) fn remove_blob_sync_states<I>(&self, record_ids: I) -> Result<(), CloudBackupError>
    where
        I: IntoIterator<Item = String>,
    {
        let table = &Database::global().cloud_blob_sync_states;

        for record_id in record_ids {
            table.delete(&record_id).map_err(|source| {
                CloudBackupError::internal_context("remove cloud blob sync state", source)
            })?;
        }

        self.refresh_pending_upload_verification_state();
        self.wake_pending_upload_verifier();

        Ok(())
    }

    pub(crate) fn start_pending_upload_verification_loop(&self) {
        send!(self.supervisor.ensure_pending_upload_verification_loop());
    }

    pub(crate) async fn verify_pending_uploads_once(&self) -> PendingUploadVerificationState {
        // the persisted queue is process-global, so every manager shares one verifier pass
        let _guard = PENDING_UPLOAD_VERIFIER_LOCK.lock().await;

        PendingUploadVerifier(self.clone()).run_once().await
    }

    pub(crate) fn wake_pending_upload_verifier(&self) {
        send!(self.supervisor.wake_pending_upload_verifier());
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use tokio::sync::MutexGuard;

    use super::PENDING_UPLOAD_VERIFIER_LOCK;

    /// Takes the one verifier pass, for a test that reads and settles a confirmation itself
    ///
    /// A manager's own verifier loop waits for the guard before its next pass, so it cannot settle
    /// the marker under the test while the guard is held
    pub(crate) async fn hold_verifier_pass() -> MutexGuard<'static, ()> {
        PENDING_UPLOAD_VERIFIER_LOCK.lock().await
    }
}
