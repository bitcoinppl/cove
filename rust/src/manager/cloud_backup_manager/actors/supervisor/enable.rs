use super::*;

mod discard;
mod passkey_confirmation;
mod recovery;
mod restart;
mod start;
mod upload_finalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingEnableUploadSelection {
    RetryOnly,
    RetryOrForceNewConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SavedPasskeyConfirmationRetry {
    Manual,
    Automatic,
}

impl SavedPasskeyConfirmationRetry {
    fn for_mode(mode: SavedPasskeyConfirmationMode) -> Self {
        match mode {
            SavedPasskeyConfirmationMode::Manual => Self::Manual,
            SavedPasskeyConfirmationMode::Automatic => Self::Automatic,
        }
    }
}

pub(crate) struct EnableRecoveryFinalization {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) active_critical_key: zeroize::Zeroizing<[u8; 32]>,
    pub(crate) pending_completion: PendingVerificationCompletion,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for EnableRecoveryFinalization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnableRecoveryFinalization")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("pending_uploads_count", &self.pending_completion.uploads().len())
            .field("cleanup_sources_count", &self.cleanup_sources.len())
            .finish()
    }
}

pub(crate) struct EnableUploadFinalization {
    pub(crate) master_key: zeroize::Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: zeroize::Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) pending_completion: PendingVerificationCompletion,
}

fn enable_pending_verification_completion(
    namespace_id: String,
    pending_uploads: Vec<PendingVerificationUpload>,
) -> PendingVerificationCompletion {
    PendingVerificationCompletion::new(
        DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        },
        namespace_id,
        pending_uploads,
    )
}

impl CloudBackupSupervisor {
    fn fail_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            self.fail_reinitialize_enable_operation(manager, claim, error);
            return;
        }

        warn!("Enable failed: {error}");
        manager.clear_enable_progress_report();
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_enable_state(CloudBackupEnableState::Idle);
        manager
            .reconcile_runtime_status(RustCloudBackupManager::status_for_operation_error(&error));
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn fail_reinitialize_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        warn!("Reinitialize backup enable failed: {error}");
        match error {
            CloudBackupError::UnsupportedPasskeyProvider => {
                manager.apply_recovery_state(RecoveryState::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
            }
            error => {
                let runtime_status = RustCloudBackupManager::runtime_status_for(
                    &RustCloudBackupManager::load_persisted_state(),
                );
                manager.reconcile_runtime_status(runtime_status);
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: error.reader_message(),
                });
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn finish_enable_operation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            manager.apply_recovery_state(RecoveryState::Idle);
            let runtime_status = RustCloudBackupManager::runtime_status_for(
                &RustCloudBackupManager::load_persisted_state(),
            );
            if matches!(runtime_status, CloudBackupStatus::Enabled) {
                self.start_verification_with_context(
                    manager,
                    Some(claim),
                    DeepVerificationContinuation::ReinitializeBackup {
                        attempt: VerificationAttempt::Initial,
                    },
                );
                return;
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }
}
