use act_zero::send;
use tracing::error;

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use super::{
    CLOUD_BACKUP_MANAGER, CloudBackupError, CloudBackupManagerAction,
    CloudBackupPasskeyChoiceIntent, CloudBackupWalletItem, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, OtherBackupsOperation, RustCloudBackupManager,
    workers::CloudBackupOperation,
};

type Action = CloudBackupManagerAction;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RecoveryAction {
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum VerificationState {
    Idle,
    Verifying,
    Verified(DeepVerificationReport),
    PasskeyConfirmed,
    Failed(DeepVerificationFailure),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationReason {
    BackupChanged,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationSource {
    RootPrompt,
    Settings,
    CloudBackupDetail,
    Onboarding,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationPresentation {
    Hidden,
    /// The verification sheet is only for an unanswered user decision
    NeedsDecision {
        reason: CloudBackupVerificationReason,
    },
    /// Native passkey UI may appear while this state is active
    ManualVerifying {
        source: CloudBackupVerificationSource,
    },
    BackgroundConfirming(CloudBackupVerificationSource),
    BackgroundBlockedOnAuthorization(CloudBackupVerificationSource),
    /// Completion feedback should match the source instead of reopening the sheet
    Completed {
        source: CloudBackupVerificationSource,
    },
    /// Failure is a result, not another request to show the decision sheet
    Failed {
        source: CloudBackupVerificationSource,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PendingUploadVerificationState {
    Idle,
    Confirming,
    BlockedOnAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncState {
    Idle,
    Syncing,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RecoveryState {
    Idle,
    Recovering(RecoveryAction),
    Failed { action: RecoveryAction, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudOnlyState {
    NotFetched,
    Loading,
    Loaded { wallets: Vec<CloudBackupWalletItem> },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudOnlyOperation {
    Idle,
    Operating { record_id: String },
    Warning { message: String, error: String },
    Failed { error: String },
}

#[uniffi::export]
impl RustCloudBackupManager {
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) {
        use Action as A;
        match action {
            A::EnableCloudBackup(context) => {
                self.clear_passkey_choice_prompt();
                self.enable_cloud_backup(context);
            }
            A::EnableCloudBackupForceNew(context) => {
                self.clear_existing_backup_found_prompt();
                self.enable_cloud_backup_force_new(context);
            }
            A::EnableCloudBackupNoDiscovery(context) => {
                self.clear_existing_backup_found_prompt();
                self.clear_passkey_choice_prompt();
                self.enable_cloud_backup_no_discovery(context);
            }
            A::ConfirmSavedPasskey => {
                self.confirm_saved_passkey();
            }
            A::DiscardPendingEnableCloudBackup => {
                self.discard_pending_enable_cloud_backup();
            }
            A::DismissPasskeyChoicePrompt => self.clear_passkey_choice_prompt(),
            A::DismissMissingPasskeyReminder => self.dismiss_missing_passkey_prompt(),
            A::RestoreFromCloudBackup => self.restore_from_cloud_backup(),
            A::CancelRestore => self.cancel_restore(),
            A::StartVerification(source) => self.start_verification(source),
            A::StartVerificationDiscoverable(source) => {
                self.start_verification_discoverable(source);
            }
            A::DismissVerificationPrompt => self.dismiss_verification_prompt(),
            A::RecreateManifest => {
                CLOUD_BACKUP_MANAGER.clone().spawn_recovery(RecoveryAction::RecreateManifest);
            }
            A::ReinitializeBackup => {
                CLOUD_BACKUP_MANAGER.clone().spawn_recovery(RecoveryAction::ReinitializeBackup);
            }
            A::RepairPasskey => {
                self.clear_passkey_choice_prompt();
                CLOUD_BACKUP_MANAGER.clone().spawn_repair_passkey(false);
            }
            A::RepairPasskeyNoDiscovery => {
                self.clear_passkey_choice_prompt();
                CLOUD_BACKUP_MANAGER.clone().spawn_repair_passkey(true);
            }
            A::SyncUnsynced => CLOUD_BACKUP_MANAGER.clone().spawn_sync(),
            A::FetchCloudOnly => CLOUD_BACKUP_MANAGER.clone().spawn_fetch_cloud_only(),
            A::RestoreCloudWallet(record_id) => {
                CLOUD_BACKUP_MANAGER.clone().spawn_restore_cloud_wallet(record_id);
            }
            A::DeleteCloudWallet(record_id) => {
                CLOUD_BACKUP_MANAGER.clone().spawn_delete_cloud_wallet(record_id);
            }
            A::RecoverOtherBackups => CLOUD_BACKUP_MANAGER.clone().spawn_recover_other_backups(),
            A::DeleteOtherBackups => CLOUD_BACKUP_MANAGER.clone().spawn_delete_other_backups(),
            A::RefreshDetail => CLOUD_BACKUP_MANAGER.clone().spawn_refresh_detail(),
            A::EnterDetail => CLOUD_BACKUP_MANAGER.clone().spawn_enter_detail(),
        }
    }
}

impl RustCloudBackupManager {
    fn start_verification(&self, source: CloudBackupVerificationSource) {
        if self.has_pending_cloud_upload_verification() {
            self.resume_pending_cloud_upload_verification();
            return;
        }

        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt before verification: {error}");
        }

        self.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(source),
        );
        send!(self.supervisor.start_verification(false));
    }

    fn start_verification_discoverable(&self, source: CloudBackupVerificationSource) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt before verification: {error}");
        }
        self.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(source),
        );
        send!(self.supervisor.start_verification(true));
    }

    fn dismiss_verification_prompt(&self) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt: {error}");
        }
        self.apply_verification_effect(CloudBackupVerificationCoordinator::dismiss_decision());
    }

    fn spawn_recovery(self: std::sync::Arc<Self>, action: RecoveryAction) {
        let operation = CloudBackupOperation::Recovery(action);
        send!(self.supervisor.start_operation(operation, None));
    }

    fn spawn_repair_passkey(self: std::sync::Arc<Self>, no_discovery: bool) {
        let operation = CloudBackupOperation::RepairPasskey { no_discovery };
        send!(self.supervisor.start_operation(operation, None));
    }

    fn spawn_sync(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_operation(CloudBackupOperation::Sync, None));
    }

    fn spawn_fetch_cloud_only(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_operation(CloudBackupOperation::FetchCloudOnly, None));
    }

    fn spawn_restore_cloud_wallet(self: std::sync::Arc<Self>, record_id: super::RecordId) {
        let operation = CloudBackupOperation::RestoreCloudWallet;
        send!(self.supervisor.start_operation(operation, Some(record_id.into())));
    }

    fn spawn_delete_cloud_wallet(self: std::sync::Arc<Self>, record_id: super::RecordId) {
        let operation = CloudBackupOperation::DeleteCloudWallet;
        send!(self.supervisor.start_operation(operation, Some(record_id.into())));
    }

    fn spawn_recover_other_backups(self: std::sync::Arc<Self>) {
        if !matches!(
            &self.state.read().other_backups_operation,
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return;
        }

        send!(self.supervisor.start_operation(CloudBackupOperation::RecoverOtherBackups, None));
    }

    fn spawn_delete_other_backups(self: std::sync::Arc<Self>) {
        if !matches!(
            &self.state.read().other_backups_operation,
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return;
        }

        send!(self.supervisor.start_operation(CloudBackupOperation::DeleteOtherBackups, None));
    }

    fn spawn_refresh_detail(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_refresh_detail());
    }

    fn spawn_enter_detail(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_enter_detail());
    }

    fn confirm_saved_passkey(&self) {
        send!(self.supervisor.confirm_saved_passkey());
    }

    pub(crate) async fn handle_start_verification(&self, force_discoverable: bool) {
        self.clear_pending_verification_completion();
        if !matches!(
            self.state.read().verification_presentation,
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            self.apply_verification_effect(
                CloudBackupVerificationCoordinator::begin_manual_presentation(
                    CloudBackupVerificationSource::Settings,
                ),
            );
        }
        self.set_verification(VerificationState::Verifying);

        let result = self.deep_verify_cloud_backup(force_discoverable).await;
        self.handle_deep_verification_result(result);
    }

    pub(crate) fn handle_deep_verification_result(&self, result: DeepVerificationResult) {
        self.apply_deep_verification_result(result);
    }

    pub(crate) fn apply_deep_verification_result(&self, result: DeepVerificationResult) {
        match result {
            DeepVerificationResult::Verified(report) => {
                self.apply_verified_report(report);
            }
            DeepVerificationResult::AwaitingUploadConfirmation(report) => {
                if let Some(detail) = report.detail.clone() {
                    self.set_detail(Some(detail));
                }
                self.apply_verification_effect(
                    CloudBackupVerificationCoordinator::begin_background_confirmation(
                        self.current_verification_source(),
                    ),
                );
            }
            DeepVerificationResult::PasskeyConfirmed(detail) => {
                if let Some(detail) = detail {
                    self.set_detail(Some(detail));
                }
                self.set_verification(VerificationState::PasskeyConfirmed);
            }
            DeepVerificationResult::PasskeyMissing(detail) => {
                if let Some(detail) = detail {
                    self.set_detail(Some(detail));
                }
                self.set_verification(VerificationState::Idle);
                self.set_recovery(RecoveryState::Idle);
            }
            DeepVerificationResult::UserCancelled(detail) => {
                if let Some(detail) = detail {
                    self.set_detail(Some(detail));
                }
                self.set_verification(VerificationState::Cancelled);
            }
            DeepVerificationResult::NotEnabled => {
                self.set_verification(VerificationState::Idle);
                self.set_recovery(RecoveryState::Idle);
            }
            DeepVerificationResult::Failed(failure) => {
                self.apply_failed_verification(failure);
            }
        }
    }

    pub(crate) async fn handle_recovery(&self, action: RecoveryAction) {
        self.set_recovery(RecoveryState::Recovering(action.clone()));

        let result = match &action {
            RecoveryAction::RecreateManifest => self.do_reupload_all_wallets().await,
            RecoveryAction::ReinitializeBackup => self.run_reinitialize_backup().await,
            RecoveryAction::RepairPasskey => self.do_repair_passkey_wrapper().await,
        };
        let should_auto_verify = match action {
            RecoveryAction::ReinitializeBackup => {
                matches!(self.current_status(), super::CloudBackupStatus::Enabled)
            }
            RecoveryAction::RecreateManifest | RecoveryAction::RepairPasskey => true,
        };

        match result {
            Ok(()) => {
                self.set_recovery(RecoveryState::Idle);
                if should_auto_verify {
                    self.handle_start_verification(false).await;
                }
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                self.set_recovery(RecoveryState::Idle);
                self.set_status(RustCloudBackupManager::status_for_operation_error(
                    &CloudBackupError::UnsupportedPasskeyProvider,
                ));
            }
            Err(error) => {
                self.set_recovery(RecoveryState::Failed { action, error: error.to_string() });
            }
        }
    }

    pub(crate) async fn handle_repair_passkey(&self, no_discovery: bool) {
        self.set_recovery(RecoveryState::Recovering(RecoveryAction::RepairPasskey));

        let result = if no_discovery {
            self.do_repair_passkey_wrapper_no_discovery().await
        } else {
            self.do_repair_passkey_wrapper().await
        };

        match result {
            Ok(()) => {
                if let Err(error) = self.finalize_passkey_repair().await {
                    self.set_recovery(RecoveryState::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.to_string(),
                    });
                    return;
                }

                self.set_recovery(RecoveryState::Idle);
                self.set_verification(VerificationState::Idle);
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.set_recovery(RecoveryState::Idle);
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::RepairPasskey);
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                self.set_recovery(RecoveryState::Idle);
                self.set_status(RustCloudBackupManager::status_for_operation_error(
                    &CloudBackupError::UnsupportedPasskeyProvider,
                ));
            }
            Err(error) => {
                self.set_recovery(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
            }
        }
    }

    async fn run_reinitialize_backup(&self) -> Result<(), CloudBackupError> {
        if !self.begin_background_operation(
            "reinitialize_cloud_backup",
            Some(super::CloudBackupStatus::Enabling),
        ) {
            return Err(CloudBackupError::RecoveryRequired(
                "cloud backup operation already running".into(),
            ));
        }

        let result = self.do_enable_cloud_backup().await;
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.set_status(RustCloudBackupManager::runtime_status_for(
                    &RustCloudBackupManager::load_persisted_state(),
                ));
                Err(error)
            }
        }
    }

    pub(crate) async fn handle_sync(&self) {
        self.set_sync(SyncState::Syncing);

        match self.do_sync_unsynced_wallets().await {
            Ok(()) => {
                self.handle_refresh_detail().await;
                self.set_sync(SyncState::Idle);
            }
            Err(error) => {
                self.set_sync(SyncState::Failed(error.to_string()));
            }
        }
    }

    pub(crate) async fn handle_fetch_cloud_only(&self) {
        self.set_cloud_only(CloudOnlyState::Loading);
        self.set_cloud_only_operation(CloudOnlyOperation::Idle);

        match self.do_fetch_cloud_only_wallets().await {
            Ok(items) => {
                self.set_loaded_cloud_only(items);
            }
            Err(error) => {
                error!("Failed to fetch cloud-only wallets: {error}");
                self.set_cloud_only(CloudOnlyState::Failed { error: error.to_string() });
            }
        }
    }

    pub(crate) async fn handle_restore_cloud_wallet(&self, record_id: &str) {
        self.set_cloud_only_operation(CloudOnlyOperation::Operating {
            record_id: record_id.to_string(),
        });

        match self.do_restore_cloud_wallet(record_id).await {
            Ok(outcome) => {
                if let Some(warning) = outcome.labels_warning {
                    self.set_cloud_only_operation(CloudOnlyOperation::Warning {
                        message: format!(
                            "{} was restored, but its labels could not be imported",
                            warning.wallet_name
                        ),
                        error: warning.error,
                    });
                } else {
                    self.set_cloud_only_operation(CloudOnlyOperation::Idle);
                }

                let mut cloud_only = self.state.read().cloud_only.clone();
                if let CloudOnlyState::Loaded { wallets } = &mut cloud_only {
                    wallets.retain(|wallet| wallet.record_id != record_id);
                }
                self.set_cloud_only(cloud_only);
                self.handle_refresh_detail().await;
            }
            Err(error) => {
                self.set_cloud_only_operation(CloudOnlyOperation::Failed {
                    error: error.to_string(),
                });
            }
        }
    }

    pub(crate) async fn handle_delete_cloud_wallet(&self, record_id: &str) {
        self.set_cloud_only_operation(CloudOnlyOperation::Operating {
            record_id: record_id.to_string(),
        });

        match self.do_delete_cloud_wallet(record_id).await {
            Ok(()) => {
                self.set_cloud_only_operation(CloudOnlyOperation::Idle);

                let mut cloud_only = self.state.read().cloud_only.clone();
                if let CloudOnlyState::Loaded { wallets } = &mut cloud_only {
                    wallets.retain(|wallet| wallet.record_id != record_id);
                }
                self.set_cloud_only(cloud_only);
                self.handle_refresh_detail().await;
            }
            Err(error) => {
                self.set_cloud_only_operation(CloudOnlyOperation::Failed {
                    error: error.to_string(),
                });
            }
        }
    }

    pub(crate) async fn handle_recover_other_backups(&self) {
        match self.do_recover_other_backups().await {
            Ok(report) => {
                self.set_other_backups_operation(OtherBackupsOperation::Recovered {
                    wallets_restored: report.wallets_restored,
                    wallets_failed: report.wallets_failed,
                    failed_wallet_errors: report.failed_wallet_errors,
                });
                self.handle_sync().await;
            }
            Err(error) => {
                self.set_other_backups_operation(OtherBackupsOperation::Failed {
                    error: error.to_string(),
                });
            }
        }
    }

    pub(crate) async fn handle_delete_other_backups(&self) {
        match self.do_delete_other_backups().await {
            Ok(()) => {
                self.set_other_backups_operation(OtherBackupsOperation::Deleted);
                self.handle_refresh_detail().await;
            }
            Err(error) => {
                self.set_other_backups_operation(OtherBackupsOperation::Failed {
                    error: error.to_string(),
                });
            }
        }
    }

    pub(crate) async fn handle_refresh_detail(&self) {
        self.refresh_sync_health();
        if let Some(result) = self.refresh_cloud_backup_detail().await {
            match result {
                super::CloudBackupDetailResult::Success(detail) => {
                    self.set_detail(Some(detail));
                }
                super::CloudBackupDetailResult::AccessError(error) => {
                    error!("Failed to refresh detail: {error}");
                }
            }
        }
    }
}
