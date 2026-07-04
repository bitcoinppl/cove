use super::*;

mod discard;
mod passkey_confirmation;
mod recovery;
mod start;
mod upload_finalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingEnableUploadSelection {
    RetryOnly,
    RetryOrForceNewConfirmation,
}

const AUTOMATIC_SAVED_PASSKEY_CONFIRMATION_RETRIES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SavedPasskeyConfirmationRetry {
    Manual,
    Automatic { retries_remaining: u8 },
}

impl SavedPasskeyConfirmationRetry {
    fn for_mode(mode: SavedPasskeyConfirmationMode) -> Self {
        match mode {
            SavedPasskeyConfirmationMode::Manual => Self::Manual,
            SavedPasskeyConfirmationMode::Automatic => {
                Self::Automatic { retries_remaining: AUTOMATIC_SAVED_PASSKEY_CONFIRMATION_RETRIES }
            }
        }
    }

    fn should_retry(self, error: &CloudBackupError) -> bool {
        matches!(
            self,
            Self::Automatic { retries_remaining } if retries_remaining > 0
        ) && matches!(error, CloudBackupError::Passkey(_))
    }

    fn after_retry(self) -> Self {
        match self {
            Self::Manual => Self::Manual,
            Self::Automatic { retries_remaining } => {
                Self::Automatic { retries_remaining: retries_remaining.saturating_sub(1) }
            }
        }
    }
}

pub(crate) struct EnableRecoveryFinalization {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) active_critical_key: zeroize::Zeroizing<[u8; 32]>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for EnableRecoveryFinalization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnableRecoveryFinalization")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("pending_uploads_count", &self.pending_uploads.len())
            .field("cleanup_sources_count", &self.cleanup_sources.len())
            .finish()
    }
}

pub(crate) struct EnableUploadFinalization {
    pub(crate) master_key: zeroize::Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: zeroize::Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
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
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
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
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
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
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: error.to_string(),
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
            manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
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
