use ahash::HashMap;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_util::ResultExt as _;
use tracing::warn;
use zeroize::Zeroizing;

use super::{
    CloudBackupDetailResult, CloudBackupError, DeepVerificationFailure, DeepVerificationReport,
    DeepVerificationResult, PendingVerificationCompletion, RecoveryState, RustCloudBackupManager,
    VerificationFailureKind, VerificationState,
};
use crate::database::Database;
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, PendingVerificationUpload,
    wallets::{WalletBackupLookup, WalletBackupReader},
};

enum PendingWalletVerificationOutcome {
    Pending,
    Verified,
    Failed,
    Unsupported,
}

enum FinalizePendingVerificationResult {
    Pending,
    Completed(DeepVerificationReport),
}

impl RustCloudBackupManager {
    pub(crate) async fn finalize_pending_verification_if_ready(&self) {
        let Some(completion) = self.pending_verification_completion() else { return };

        if !self.pending_verification_uploads_confirmed(&completion).await {
            return;
        }

        match self.finalize_pending_verification(completion.clone()).await {
            Ok(FinalizePendingVerificationResult::Pending) => return,
            Ok(FinalizePendingVerificationResult::Completed(report)) => {
                self.apply_verified_report(report)
            }
            Err(failure) => self.apply_failed_verification(*failure),
        }

        self.clear_pending_verification_completion();
    }

    async fn pending_verification_uploads_confirmed(
        &self,
        completion: &PendingVerificationCompletion,
    ) -> bool {
        let sync_states_by_record_id: HashMap<_, _> = Database::global()
            .cloud_blob_sync_states
            .list()
            .ok()
            .unwrap_or_default()
            .into_iter()
            .filter(|state| state.namespace_id == completion.namespace_id())
            .map(|state| (state.record_id.clone(), state.state))
            .collect();

        for upload in completion.uploads() {
            let sync_state = sync_states_by_record_id.get(upload.record_id());
            if !self.is_pending_upload_confirmed(completion, upload, sync_state).await {
                return false;
            }
        }

        true
    }

    async fn is_pending_upload_confirmed(
        &self,
        completion: &PendingVerificationCompletion,
        upload: &PendingVerificationUpload,
        sync_state: Option<&PersistedCloudBlobState>,
    ) -> bool {
        match sync_state {
            Some(PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                revision_hash,
                ..
            })) => revision_hash.as_str() == upload.target_revision(sync_state),
            Some(PersistedCloudBlobState::Failed(_)) => true,
            Some(PersistedCloudBlobState::UploadedPendingConfirmation(_)) => CloudStorage::global()
                .download_wallet_backup(
                    completion.namespace_id().to_string(),
                    upload.record_id().to_string(),
                )
                .await
                .map(|_| true)
                .or_else(|error| match error {
                    CloudStorageError::NotFound(_) => Ok(false),
                    other => Err(other),
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    async fn finalize_pending_verification(
        &self,
        completion: PendingVerificationCompletion,
    ) -> Result<FinalizePendingVerificationResult, Box<DeepVerificationFailure>> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)
            .map_err(|error| {
                Box::new(self.pending_verification_failure(&completion, error.to_string()))
            })?
            .ok_or_else(|| {
                Box::new(
                    self.pending_verification_failure(&completion, "no local master key available"),
                )
            })?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let mut report = completion.report().clone();
        let sync_states_by_record_id: HashMap<_, _> = Database::global()
            .cloud_blob_sync_states
            .list()
            .ok()
            .unwrap_or_default()
            .into_iter()
            .filter(|state| state.namespace_id == completion.namespace_id())
            .map(|state| (state.record_id.clone(), state.state))
            .collect();

        for upload in completion.uploads() {
            match self
                .verify_pending_wallet_backup(
                    &completion,
                    upload,
                    sync_states_by_record_id.get(upload.record_id()),
                    &critical_key,
                )
                .await?
            {
                PendingWalletVerificationOutcome::Pending => {
                    return Ok(FinalizePendingVerificationResult::Pending);
                }
                PendingWalletVerificationOutcome::Verified => report.wallets_verified += 1,
                PendingWalletVerificationOutcome::Failed => report.wallets_failed += 1,
                PendingWalletVerificationOutcome::Unsupported => report.wallets_unsupported += 1,
            }
        }

        report.detail = self.pending_verification_detail(&completion).await;
        Ok(FinalizePendingVerificationResult::Completed(report))
    }

    async fn verify_pending_wallet_backup(
        &self,
        completion: &PendingVerificationCompletion,
        upload: &PendingVerificationUpload,
        sync_state: Option<&PersistedCloudBlobState>,
        critical_key: &[u8; 32],
    ) -> Result<PendingWalletVerificationOutcome, Box<DeepVerificationFailure>> {
        let record_id = upload.record_id();
        let expected_revision = upload.target_revision(sync_state);
        let reader = WalletBackupReader::new(
            CloudStorage::global().clone(),
            completion.namespace_id().to_string(),
            Zeroizing::new(*critical_key),
        );

        match reader.summary(record_id).await {
            Ok(WalletBackupLookup::Found(summary))
                if summary.revision_hash != expected_revision =>
            {
                warn!(
                    "Pending verification: wallet {record_id} is still stale expected_revision={} actual_revision={}",
                    expected_revision, summary.revision_hash
                );
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Ok(WalletBackupLookup::Found(_)) => Ok(PendingWalletVerificationOutcome::Verified),
            Ok(WalletBackupLookup::NotFound) => {
                warn!("Pending verification: wallet {record_id} is not ready yet: not found");
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Ok(WalletBackupLookup::UnsupportedVersion(_)) => {
                Ok(PendingWalletVerificationOutcome::Unsupported)
            }
            Err(CloudBackupError::Cloud(error)) => {
                warn!("Pending verification: wallet {record_id} is not ready yet: {error}");
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Err(error) => {
                warn!("Pending verification: failed to decrypt wallet {record_id}: {error}");
                Ok(PendingWalletVerificationOutcome::Failed)
            }
        }
    }

    async fn pending_verification_detail(
        &self,
        completion: &PendingVerificationCompletion,
    ) -> Option<CloudBackupDetail> {
        match self.refresh_cloud_backup_detail().await {
            Some(CloudBackupDetailResult::Success(detail)) => Some(detail),
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Pending verification: failed to refresh detail: {error}");
                completion.report().detail.clone()
            }
            None => completion.report().detail.clone(),
        }
    }

    fn pending_verification_failure(
        &self,
        completion: &PendingVerificationCompletion,
        message: impl Into<String>,
    ) -> DeepVerificationFailure {
        DeepVerificationFailure {
            kind: VerificationFailureKind::Retry,
            message: message.into(),
            detail: completion.report().detail.clone(),
        }
    }

    pub(crate) fn apply_verified_report(&self, report: DeepVerificationReport) {
        self.persist_verification_result(&DeepVerificationResult::Verified(report.clone()));
        if let Some(detail) = &report.detail {
            self.set_detail(Some(detail.clone()));
        }
        self.set_verification(VerificationState::Verified(report));
        self.set_recovery(RecoveryState::Idle);
    }

    pub(crate) fn apply_failed_verification(&self, failure: DeepVerificationFailure) {
        self.persist_verification_result(&DeepVerificationResult::Failed(failure.clone()));
        if let Some(detail) = failure.detail.clone() {
            self.set_detail(Some(detail));
        }
        self.set_verification(VerificationState::Failed(failure));
    }
}
