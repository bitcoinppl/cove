mod detail;
mod queue_processor;

use std::time::Duration;

use act_zero::send;
use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_util::ResultExt as _;

use self::queue_processor::PendingUploadVerifier;
use super::{CloudBackupError, RustCloudBackupManager};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, CloudBlobFailedState, CloudBlobUploadedPendingConfirmationState,
    CloudUploadKind, PersistedCloudBlobState, PersistedCloudBlobSyncState,
};
use crate::wallet::metadata::WalletId;

pub(crate) use detail::remote_wallet_revision_matches;

pub(super) const MAX_PENDING_UPLOAD_VERIFICATION_DELAY: Duration = Duration::from_secs(10);

pub(super) fn build_pending_upload_backoff() -> backon::FibonacciBackoff {
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
        let next_sync_state =
            PersistedCloudBlobSyncState { state: next_state, ..current_state.clone() };

        Database::global()
            .cloud_blob_sync_states
            .set_if_current(current_state, &next_sync_state)
            .map_err_prefix(error_context, CloudBackupError::Internal)
    }

    pub(crate) fn mark_blob_uploaded_pending_confirmation(
        &self,
        namespace_id: &str,
        wallet_id: Option<WalletId>,
        record_id: String,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Result<(), CloudBackupError> {
        let sync_state = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: namespace_id.to_string(),
            wallet_id,
            record_id,
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash,
                    uploaded_at,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        };

        Database::global()
            .cloud_blob_sync_states
            .set(&sync_state)
            .map_err_prefix("persist uploaded cloud blob state", CloudBackupError::Internal)?;

        self.set_pending_upload_verification(true);
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

        self.set_pending_upload_verification(true);
        self.wake_pending_upload_verifier();
        self.start_pending_upload_verification_loop();

        Ok(true)
    }

    pub(crate) fn mark_blob_failed_if_current(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: Option<String>,
        retryable: bool,
        error: String,
    ) -> Result<bool, CloudBackupError> {
        let failed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

        self.replace_blob_state_if_current(
            current_state,
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash,
                retryable,
                error,
                failed_at,
            }),
            "persist failed cloud blob state",
        )
    }

    pub(crate) fn mark_blob_dirty_state(
        &self,
        current_state: &PersistedCloudBlobSyncState,
    ) -> Result<(), CloudBackupError> {
        let changed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let dirty_state = PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            ..current_state.clone()
        };

        Database::global()
            .cloud_blob_sync_states
            .set(&dirty_state)
            .map_err_prefix("persist dirty cloud blob state", CloudBackupError::Internal)
    }

    pub(super) fn remove_blob_sync_states<I>(&self, record_ids: I) -> Result<(), CloudBackupError>
    where
        I: IntoIterator<Item = String>,
    {
        let table = &Database::global().cloud_blob_sync_states;

        for record_id in record_ids {
            table
                .delete(&record_id)
                .map_err_prefix("remove cloud blob sync state", CloudBackupError::Internal)?;
        }

        self.set_pending_upload_verification(self.has_pending_cloud_upload_verification());
        self.wake_pending_upload_verifier();

        Ok(())
    }

    pub(super) fn start_pending_upload_verification_loop(&self) {
        send!(self.runtime.ensure_pending_upload_verification_loop());
    }

    pub(crate) fn verify_pending_uploads_once(&self) -> bool {
        PendingUploadVerifier(self.clone()).run_once()
    }

    pub(crate) fn wake_pending_upload_verifier(&self) {
        send!(self.runtime.wake_pending_upload_verifier());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_upload_backoff_resets_to_short_delay() {
        let mut backoff = build_pending_upload_backoff();
        let initial_delay = backoff.next().expect("expected initial delay");

        let _ = backoff.next();
        let _ = backoff.next();

        let mut backoff = build_pending_upload_backoff();

        assert_eq!(backoff.next().expect("expected reset delay"), initial_delay);
    }

    #[test]
    fn pending_upload_backoff_produces_delays() {
        let mut backoff = build_pending_upload_backoff();

        for _ in 0..10 {
            assert!(backoff.next().is_some(), "expected delay");
        }
    }
}
