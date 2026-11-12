use super::*;

mod continuation;
mod discard;
mod passkey_confirmation;
mod recovery;
mod restart;
mod start;
mod upload_finalization;

pub(crate) use continuation::{DiscardTarget, EnableContinuation};

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
    pub(crate) active_critical_key: zeroize::Zeroizing<[u8; 32]>,
    pub(crate) pending_completion: PendingVerificationCompletion,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for EnableRecoveryFinalization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnableRecoveryFinalization")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("pending_uploads_count", &self.pending_completion.uploads().len())
            .field("cleanup_sources_count", &self.cleanup_sources.len())
            .finish()
    }
}

pub(crate) struct EnableUploadFinalization {
    pub(crate) context: CloudBackupEnableContext,
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
            wallet_issues: Default::default(),
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
        self.screen_outcome.operation_failed(claim, &error);
        manager.clear_enable_progress_report();
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_enable_state(CloudBackupEnableState::Idle);
        manager
            .reconcile_runtime_status(RustCloudBackupManager::status_for_operation_error(&error));
        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }

    fn fail_reinitialize_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        warn!("Reinitialize backup enable failed: {error}");
        self.screen_outcome.operation_failed(claim, &error);
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
                let runtime_status = self.runtime.status();
                manager.reconcile_runtime_status(runtime_status);
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: error.reader_message(),
                });
            }
        }

        match self.drive_account_switch_reinitialization_finished(
            manager,
            claim,
            DriveAccountSwitchReinitializationEnd::Failed,
        ) {
            DriveAccountSwitchReinitializationCompletion::NotDriveAccountSwitch => {}
            DriveAccountSwitchReinitializationCompletion::Stale
            | DriveAccountSwitchReinitializationCompletion::Handled => return,
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }

    pub(crate) fn finish_enable_operation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            match self.drive_account_switch_reinitialization_finished(
                &manager,
                claim,
                DriveAccountSwitchReinitializationEnd::Succeeded,
            ) {
                DriveAccountSwitchReinitializationCompletion::NotDriveAccountSwitch => {}
                DriveAccountSwitchReinitializationCompletion::Stale
                | DriveAccountSwitchReinitializationCompletion::Handled => return,
            }

            self.finish_reinitialize_backup(manager, claim);
            return;
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }

    /// Ends a successful reinitialize that has nothing left to wait for
    ///
    /// A plain reinitialize gets here as soon as it succeeds, and a Google Drive account switch
    /// gets here once the platform has committed the new account
    pub(crate) fn finish_reinitialize_backup(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        manager.apply_recovery_state(RecoveryState::Idle);
        let runtime_status = self.runtime.status();

        if matches!(runtime_status, CloudBackupStatus::Enabled) {
            // the reinitialize claim reports the backup as still enabling, and verification only
            // reads the cloud for an enabled backup, so the verification gets its own claim
            let Some(verification_claim) = self.hand_off_exclusive_operation(
                &manager,
                claim,
                CloudBackupExclusiveOperation::VerifyReinitializedBackup,
            ) else {
                return;
            };

            self.start_verification_with_context(
                manager,
                verification_claim,
                DeepVerificationContinuation::ReinitializeBackup {
                    attempt: VerificationAttempt::Initial,
                },
            );
            return;
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }
}
