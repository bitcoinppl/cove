use std::sync::atomic::Ordering;

use ahash::HashMap;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

use super::{
    CloudBackupDetailResult, CloudBackupError, CloudBackupKeychain, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, PendingVerificationCompletion,
    RustCloudBackupManager,
};
use crate::database::Database;
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::actors::supervisor::EvidenceBasis;
use crate::manager::cloud_backup_manager::verify::coordinator::CloudBackupVerificationCoordinator;
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, PendingEnableJournalPhase, PendingVerificationUpload,
    master_key_wrapper_revision_hash,
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

enum PendingVerificationUploadsReadiness {
    Pending,
    Confirmed,
    TerminalFailure(String),
}

const PENDING_VERIFICATION_COMPLETION_TTL_SECONDS: u64 = 24 * 60 * 60;

/// An upload confirmation that finished, for the lifecycle owner to settle
///
/// The owner accepts it only while `completion` is still the stored marker and the configuration
/// and local wallet set are still `basis`, the ones this evidence was read under. A confirmation that a manual
/// verification, a newer marker or a wallet change superseded is dropped without effects
#[derive(Debug)]
pub(crate) struct PendingVerificationSettlement {
    pub(crate) basis: EvidenceBasis,
    pub(crate) completion: PendingVerificationCompletion,
    pub(crate) result: DeepVerificationResult,
}

impl RustCloudBackupManager {
    /// Checks the pending upload confirmation and returns its settlement once it finished
    ///
    /// `basis` is the owner's configuration and wallet set, read before any evidence here
    pub(crate) async fn ready_pending_verification_settlement(
        &self,
        basis: EvidenceBasis,
    ) -> Option<PendingVerificationSettlement> {
        if Self::pending_enable_finalization_owns_completion() {
            return None;
        }

        let completion = self.pending_verification_completion()?;

        if completion.is_expired(
            cove_util::time::unix_timestamp_secs_or_zero(),
            PENDING_VERIFICATION_COMPLETION_TTL_SECONDS,
        ) {
            return Some(Self::expired_pending_verification_settlement(basis, completion));
        }

        match self.pending_verification_uploads_readiness(&completion) {
            PendingVerificationUploadsReadiness::Pending => return None,
            PendingVerificationUploadsReadiness::Confirmed => {}
            PendingVerificationUploadsReadiness::TerminalFailure(error) => {
                let failure =
                    DeepVerificationFailure::retry(error, completion.report().detail, None);
                let result = DeepVerificationResult::Failed(failure);
                return Some(PendingVerificationSettlement { basis, completion, result });
            }
        }

        let result = match self.finalize_pending_verification(completion.clone()).await {
            Ok(FinalizePendingVerificationResult::Pending) => return None,
            Ok(FinalizePendingVerificationResult::Completed(report)) => {
                if report.wallet_issues.is_empty() {
                    DeepVerificationResult::Verified(report)
                } else {
                    DeepVerificationResult::NeedsAttention(report)
                }
            }
            Err(failure) => DeepVerificationResult::Failed(*failure),
        };

        Some(PendingVerificationSettlement { basis, completion, result })
    }

    fn pending_enable_finalization_owns_completion() -> bool {
        match CloudBackupKeychain::global().load_pending_enable_journal() {
            Ok(Some(journal)) => {
                matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_))
            }
            Ok(None) => false,
            Err(error) => {
                warn!(
                    "Pending upload verification: could not inspect pending enable state: {error}"
                );
                true
            }
        }
    }

    fn expired_pending_verification_settlement(
        basis: EvidenceBasis,
        completion: PendingVerificationCompletion,
    ) -> PendingVerificationSettlement {
        warn!(
            "Pending verification expired after {PENDING_VERIFICATION_COMPLETION_TTL_SECONDS}s without confirmed uploads"
        );
        let failure = DeepVerificationFailure::retry(
            "cloud backup upload confirmation expired; start verification again",
            completion.report().detail,
            None,
        );
        let result = DeepVerificationResult::Failed(failure);

        PendingVerificationSettlement { basis, completion, result }
    }

    fn pending_verification_uploads_readiness(
        &self,
        completion: &PendingVerificationCompletion,
    ) -> PendingVerificationUploadsReadiness {
        let Some(sync_states_by_record_id) = self.pending_verification_sync_states(completion)
        else {
            return PendingVerificationUploadsReadiness::Pending;
        };

        for upload in completion.uploads() {
            let sync_state = sync_states_by_record_id.get(upload.record_id());
            if let Some(PersistedCloudBlobState::Failed(failure)) = sync_state
                && !failure.retryable
                && matches!(upload, PendingVerificationUpload::MasterKeyWrapper)
            {
                return PendingVerificationUploadsReadiness::TerminalFailure(format!(
                    "cloud backup upload could not be confirmed: {}",
                    failure.error
                ));
            }
            if !Self::is_pending_upload_confirmed(upload, sync_state) {
                return PendingVerificationUploadsReadiness::Pending;
            }
        }

        PendingVerificationUploadsReadiness::Confirmed
    }

    /// Blob sync states for the completion's namespace, or `None` while the database is unreadable
    ///
    /// An unreadable table proves nothing about the uploads, so callers keep the completion
    /// pending; the queue retries every pass, so only a change in readability is logged
    fn pending_verification_sync_states(
        &self,
        completion: &PendingVerificationCompletion,
    ) -> Option<HashMap<String, PersistedCloudBlobState>> {
        match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => {
                if self.pending_sync_states_unreadable.swap(false, Ordering::AcqRel) {
                    info!("Pending verification: cloud blob sync states are readable again");
                }

                let states = states
                    .into_iter()
                    .filter(|state| state.namespace_id == completion.namespace_id())
                    .map(|state| (state.record_id().to_string(), state.state))
                    .collect();

                Some(states)
            }
            Err(error) => {
                if !self.pending_sync_states_unreadable.swap(true, Ordering::AcqRel) {
                    error!("Pending verification: failed to read cloud blob sync states: {error}");
                }

                None
            }
        }
    }

    fn is_pending_upload_confirmed(
        upload: &PendingVerificationUpload,
        sync_state: Option<&PersistedCloudBlobState>,
    ) -> bool {
        match upload {
            PendingVerificationUpload::MasterKeyWrapper => {
                matches!(sync_state, Some(PersistedCloudBlobState::Confirmed(_)))
            }
            PendingVerificationUpload::Wallet { .. } => match sync_state {
                Some(PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash,
                    ..
                })) => revision_hash.as_str() == upload.target_revision(sync_state),
                Some(PersistedCloudBlobState::Failed(failure)) if !failure.retryable => true,

                _ => false,
            },
        }
    }

    async fn finalize_pending_verification(
        &self,
        completion: PendingVerificationCompletion,
    ) -> Result<FinalizePendingVerificationResult, Box<DeepVerificationFailure>> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = match cspp.load_master_key_from_store() {
            Ok(master_key) => master_key,

            // an unreadable keychain, for example while the device is locked, proves nothing
            // about the backup, so keep the completion pending for the next check
            Err(error @ cove_cspp::CsppError::Read(_)) => {
                warn!("Pending verification: local master key is unreadable: {error}");
                return Ok(FinalizePendingVerificationResult::Pending);
            }

            Err(error) => {
                let error = CloudBackupError::internal_context("load local master key", error);
                return Err(Box::new(self.pending_verification_failure(
                    &completion,
                    error.logged_reader_message("Pending upload verification failed"),
                )));
            }
        };

        let Some(master_key) = master_key else {
            warn!("Pending upload verification failed: no local master key available");
            return Err(Box::new(
                self.pending_verification_failure(&completion, "no local master key available"),
            ));
        };

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let mut report = completion.report();
        let Some(sync_states_by_record_id) = self.pending_verification_sync_states(&completion)
        else {
            return Ok(FinalizePendingVerificationResult::Pending);
        };

        for upload in completion.uploads() {
            match upload {
                PendingVerificationUpload::MasterKeyWrapper => {
                    if !self
                        .verify_pending_master_key_wrapper(
                            &completion,
                            upload,
                            sync_states_by_record_id.get(upload.record_id()),
                        )
                        .await?
                    {
                        return Ok(FinalizePendingVerificationResult::Pending);
                    }
                }
                PendingVerificationUpload::Wallet { .. } => {
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
                        PendingWalletVerificationOutcome::Failed => {
                            report.wallet_issues.unreadable += 1;
                        }
                        PendingWalletVerificationOutcome::Unsupported => {
                            report.wallet_issues.unsupported += 1;
                        }
                    }
                }
            }
        }

        report.detail = self.pending_verification_detail(&completion).await;
        Ok(FinalizePendingVerificationResult::Completed(report))
    }

    async fn verify_pending_master_key_wrapper(
        &self,
        completion: &PendingVerificationCompletion,
        upload: &PendingVerificationUpload,
        sync_state: Option<&PersistedCloudBlobState>,
    ) -> Result<bool, Box<DeepVerificationFailure>> {
        let expected_revision = sync_state
            .and_then(PersistedCloudBlobState::revision_hash)
            .unwrap_or_else(|| upload.expected_revision());

        let bytes = CloudStorage::global_silent_client()
            .download_master_key_backup(completion.namespace_id().to_string())
            .await
            .map_err(|error| {
                let message = CloudBackupError::from(error)
                    .logged_reader_message("Pending upload verification failed");
                Box::new(self.pending_verification_failure(completion, message))
            })?;
        let actual_revision = master_key_wrapper_revision_hash(&bytes);

        if actual_revision == expected_revision {
            return Ok(true);
        }

        warn!(
            "Pending verification: master key wrapper is still stale expected_revision={expected_revision} actual_revision={actual_revision}"
        );
        Ok(false)
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
            CloudStorage::global_silent_client(),
            completion.namespace_id().to_string(),
            Zeroizing::new(*critical_key),
        );

        match reader.summary(record_id).await {
            Ok(WalletBackupLookup::Found(summary))
                if summary.revision_hash != expected_revision =>
            {
                if let Some(failure) = terminal_blob_failure(sync_state) {
                    return Err(Box::new(self.pending_verification_failure(
                        completion,
                        format!("cloud backup upload could not be confirmed: {}", failure.error),
                    )));
                }

                warn!(
                    "Pending verification: wallet backup is still stale expected_revision={} actual_revision={}",
                    expected_revision, summary.revision_hash
                );
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Ok(WalletBackupLookup::Found(_)) => Ok(PendingWalletVerificationOutcome::Verified),
            Ok(WalletBackupLookup::NotFound) => {
                if let Some(failure) = terminal_blob_failure(sync_state) {
                    return Err(Box::new(self.pending_verification_failure(
                        completion,
                        format!("cloud backup upload could not be confirmed: {}", failure.error),
                    )));
                }

                warn!("Pending verification: wallet backup is not ready yet: not found");
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Ok(WalletBackupLookup::UnsupportedVersion(_)) => {
                Ok(PendingWalletVerificationOutcome::Unsupported)
            }
            Err(error) if error.is_cloud_error() => {
                if let Some(failure) = terminal_blob_failure(sync_state) {
                    return Err(Box::new(self.pending_verification_failure(
                        completion,
                        format!("cloud backup upload could not be confirmed: {}", failure.error),
                    )));
                }

                warn!("Pending verification: wallet backup is not ready yet: {error}");
                Ok(PendingWalletVerificationOutcome::Pending)
            }
            Err(error) => {
                warn!("Pending verification: failed to decrypt wallet backup: {error}");
                Ok(PendingWalletVerificationOutcome::Failed)
            }
        }
    }

    async fn pending_verification_detail(
        &self,
        completion: &PendingVerificationCompletion,
    ) -> Option<CloudBackupDetail> {
        match self.refresh_cloud_backup_detail().await {
            Some(CloudBackupDetailResult::SuccessWithAuthority { listed, .. }) => {
                Some(listed.detail)
            }
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Pending verification: failed to refresh detail: {error}");
                completion.report().detail
            }
            None => completion.report().detail,
        }
    }

    fn pending_verification_failure(
        &self,
        completion: &PendingVerificationCompletion,
        message: impl Into<String>,
    ) -> DeepVerificationFailure {
        DeepVerificationFailure::retry(message, completion.report().detail, None)
    }

    pub(crate) fn apply_verified_report(&self, report: DeepVerificationReport) {
        let source = self.current_verification_source();
        self.apply_verification_effect(CloudBackupVerificationCoordinator::complete(
            source, report,
        ));
    }

    pub(crate) fn apply_needs_attention_report(&self, report: DeepVerificationReport) {
        let source = self.current_verification_source();
        self.apply_verification_effect(CloudBackupVerificationCoordinator::needs_attention(
            source, report,
        ));
    }

    /// Applies the in-memory effect of an upload confirmation the owner already settled
    pub(crate) fn apply_settled_verification_result(&self, result: DeepVerificationResult) {
        self.apply_verification_result_effect(result);
        self.refresh_pending_upload_verification_state();
    }

    fn apply_verification_result_effect(&self, result: DeepVerificationResult) {
        let source = self.current_verification_source();
        let effect = match result {
            DeepVerificationResult::Verified(report) => {
                CloudBackupVerificationCoordinator::complete(source, report)
            }
            DeepVerificationResult::NeedsAttention(report) => {
                CloudBackupVerificationCoordinator::needs_attention(source, report)
            }
            DeepVerificationResult::Failed(failure) => {
                CloudBackupVerificationCoordinator::fail(source, failure)
            }
            other => {
                debug!("Pending upload confirmation settled with {other:?}");
                return;
            }
        };

        self.apply_verification_effect(effect);
    }

    pub(crate) fn apply_failed_verification(&self, failure: DeepVerificationFailure) {
        let source = self.current_verification_source();
        self.apply_verification_effect(CloudBackupVerificationCoordinator::fail(source, failure));
    }
}

fn terminal_blob_failure(
    state: Option<&PersistedCloudBlobState>,
) -> Option<&crate::database::cloud_backup::CloudBlobFailedState> {
    match state {
        Some(PersistedCloudBlobState::Failed(failure)) if !failure.retryable => Some(failure),
        _ => None,
    }
}
