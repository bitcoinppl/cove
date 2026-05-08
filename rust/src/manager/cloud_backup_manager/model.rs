use super::{
    CloudBackupDetail, CloudBackupEnableState, CloudBackupRestoreProgress,
    CloudBackupRestoreReport, CloudBackupState, CloudBackupStatus, CloudOnlyOperation,
    CloudOnlyState, OtherBackupsOperation, PendingUploadVerificationState, RecoveryAction,
    RecoveryState, SyncState, VerificationState,
};

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupModel {
    compatibility: CloudBackupState,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyState {
    Available,
    Missing,
    UnsupportedProvider,
    Repairing,
    RepairFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupConfiguredState {
    pub passkey: CloudBackupPasskeyState,
    pub verification: VerificationState,
    pub pending_upload_verification: PendingUploadVerificationState,
    pub sync: SyncState,
    pub detail: Option<CloudBackupDetail>,
    pub cloud_only: CloudOnlyState,
    pub cloud_only_operation: CloudOnlyOperation,
    pub other_backups_operation: OtherBackupsOperation,
    pub last_restore_report: Option<CloudBackupRestoreReport>,
}

#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupLifecycle {
    Disabled,
    Enabling { enable_state: CloudBackupEnableState, progress: Option<super::CloudBackupProgress> },
    Restoring { progress: Option<CloudBackupRestoreProgress> },
    Configured { state: CloudBackupConfiguredState },
    Failed { message: String },
}

impl Default for CloudBackupModel {
    fn default() -> Self {
        Self::from_compatibility(CloudBackupState::default())
    }
}

impl std::ops::Deref for CloudBackupModel {
    type Target = CloudBackupState;

    fn deref(&self) -> &Self::Target {
        &self.compatibility
    }
}

impl std::ops::DerefMut for CloudBackupModel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.compatibility
    }
}

impl CloudBackupModel {
    pub(crate) fn from_compatibility(mut state: CloudBackupState) -> Self {
        state.refresh_domain_projection();
        Self { compatibility: state }
    }

    pub(crate) fn snapshot(&self) -> CloudBackupState {
        self.compatibility.clone().projected_with_fresh_domain()
    }

    pub(crate) fn refresh_domain_projection(&mut self) {
        self.compatibility.refresh_domain_projection();
    }
}

impl CloudBackupState {
    pub(crate) fn refresh_domain_projection(&mut self) {
        self.lifecycle = self.project_lifecycle();
    }

    pub(crate) fn projected_with_fresh_domain(mut self) -> Self {
        self.refresh_domain_projection();
        self
    }

    fn project_lifecycle(&self) -> CloudBackupLifecycle {
        match &self.status {
            CloudBackupStatus::Disabled => CloudBackupLifecycle::Disabled,
            CloudBackupStatus::Enabling => CloudBackupLifecycle::Enabling {
                enable_state: self.enable_state.clone(),
                progress: self.progress,
            },
            CloudBackupStatus::Restoring => {
                CloudBackupLifecycle::Restoring { progress: self.restore_progress.clone() }
            }
            CloudBackupStatus::Enabled
            | CloudBackupStatus::PasskeyMissing
            | CloudBackupStatus::UnsupportedPasskeyProvider => {
                CloudBackupLifecycle::Configured { state: self.configured_projection() }
            }
            CloudBackupStatus::Error(message) => {
                CloudBackupLifecycle::Failed { message: message.clone() }
            }
        }
    }

    fn configured_projection(&self) -> CloudBackupConfiguredState {
        CloudBackupConfiguredState {
            passkey: self.passkey_projection(),
            verification: self.verification.clone(),
            pending_upload_verification: self.pending_upload_verification,
            sync: self.sync.clone(),
            detail: self.detail.clone(),
            cloud_only: self.cloud_only.clone(),
            cloud_only_operation: self.cloud_only_operation.clone(),
            other_backups_operation: self.other_backups_operation.clone(),
            last_restore_report: self.restore_report.clone(),
        }
    }

    fn passkey_projection(&self) -> CloudBackupPasskeyState {
        match &self.status {
            CloudBackupStatus::PasskeyMissing => match &self.recovery {
                RecoveryState::Recovering(RecoveryAction::RepairPasskey) => {
                    CloudBackupPasskeyState::Repairing
                }
                RecoveryState::Failed { action: RecoveryAction::RepairPasskey, error } => {
                    CloudBackupPasskeyState::RepairFailed(error.clone())
                }
                RecoveryState::Idle
                | RecoveryState::Recovering(_)
                | RecoveryState::Failed { .. } => CloudBackupPasskeyState::Missing,
            },
            CloudBackupStatus::UnsupportedPasskeyProvider => {
                CloudBackupPasskeyState::UnsupportedProvider
            }
            CloudBackupStatus::Enabled
            | CloudBackupStatus::Enabling
            | CloudBackupStatus::Restoring => CloudBackupPasskeyState::Available,
            CloudBackupStatus::Disabled | CloudBackupStatus::Error(_) => {
                CloudBackupPasskeyState::Missing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::{
        CloudBackupEnableContext, CloudBackupPasskeyHint, CloudBackupProgress,
        CloudBackupRestoreStage, CloudBackupRootPrompt, DeepVerificationReport,
    };

    #[test]
    fn disabled_projects_disabled_lifecycle() {
        let state = CloudBackupState::default().projected_with_fresh_domain();

        assert_eq!(state.lifecycle, CloudBackupLifecycle::Disabled);
    }

    #[test]
    fn enabling_carries_enable_step_and_progress() {
        let state = CloudBackupState {
            status: CloudBackupStatus::Enabling,
            enable_state: CloudBackupEnableState::UploadingBackup,
            progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(
            state.lifecycle,
            CloudBackupLifecycle::Enabling {
                enable_state: CloudBackupEnableState::UploadingBackup,
                progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            },
        );
    }

    #[test]
    fn restoring_carries_restore_progress() {
        let progress = CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Downloading,
            completed: 1,
            total: Some(3),
        };
        let state = CloudBackupState {
            status: CloudBackupStatus::Restoring,
            restore_progress: Some(progress.clone()),
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(state.lifecycle, CloudBackupLifecycle::Restoring { progress: Some(progress) },);
    }

    #[test]
    fn configured_projects_passkey_and_verification_state() {
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 1,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        let state = CloudBackupState {
            status: CloudBackupStatus::Enabled,
            verification: VerificationState::Verified(report.clone()),
            sync: SyncState::Syncing,
            pending_upload_verification: PendingUploadVerificationState::Confirming,
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured { state } = state.lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };

        assert_eq!(state.passkey, CloudBackupPasskeyState::Available);
        assert_eq!(state.verification, VerificationState::Verified(report));
        assert_eq!(state.sync, SyncState::Syncing);
        assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming,);
    }

    #[test]
    fn passkey_missing_projects_missing_or_repairing() {
        let missing = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured { state } = missing.lifecycle else {
            panic!("passkey-missing backup should still be configured");
        };
        assert_eq!(state.passkey, CloudBackupPasskeyState::Missing);

        let repairing = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            recovery: RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured { state } = repairing.lifecycle else {
            panic!("repairing backup should still be configured");
        };
        assert_eq!(state.passkey, CloudBackupPasskeyState::Repairing);
    }

    #[test]
    fn root_prompt_projects_root_prompt() {
        let hint = CloudBackupPasskeyHint {
            provider_name: Some("iCloud Keychain".into()),
            name_suffix: "abc123".into(),
            registered_at: 1,
        };
        let state = CloudBackupState {
            root_prompt: CloudBackupRootPrompt::ExistingBackupFound(
                CloudBackupEnableContext::settings_manual(),
                Some(hint.clone()),
            ),
            ..CloudBackupState::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(
            state.root_prompt,
            CloudBackupRootPrompt::ExistingBackupFound(
                CloudBackupEnableContext::settings_manual(),
                Some(hint),
            ),
        );
    }
}
