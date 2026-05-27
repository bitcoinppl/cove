use act_zero::send;
use tracing::error;

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use super::{
    CLOUD_BACKUP_MANAGER, CloudBackupDetail, CloudBackupManagerAction, CloudBackupRestoreFlow,
    CloudBackupWalletItem, DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult,
    OtherBackupsOperation, RustCloudBackupManager, SavedPasskeyConfirmationMode,
    actors::CloudBackupOperation,
};

type Action = CloudBackupManagerAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    Hidden {
        source: Option<CloudBackupVerificationSource>,
    },
    /// The verification sheet is only for an unanswered user decision
    NeedsDecision {
        reason: CloudBackupVerificationReason,
        source: CloudBackupVerificationSource,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingUploadVerificationState {
    Idle,
    Confirming,
    BlockedOnAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Idle,
    Syncing,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupSyncOutcome {
    Started,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupCloudOnlyFetchOutcome {
    Reset,
    Started,
    Loaded(Vec<CloudBackupWalletItem>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudBackupCloudOnlyOperationWarning {
    pub(crate) message: String,
    pub(crate) error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupCloudOnlyWalletOutcome {
    Started { record_id: String },
    Restored { record_id: String, warning: Option<CloudBackupCloudOnlyOperationWarning> },
    SkippedDuplicate { record_id: String },
    Deleted { record_id: String },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupOtherBackupsOutcome {
    Idle,
    Recovering,
    Recovered { wallets_restored: u32, wallets_failed: u32, failed_wallet_errors: Vec<String> },
    Deleting,
    Deleted,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupEnableOutcome {
    ProgressCleared,
    ReturnedToIdle,
    CreatingPasskey,
    UploadingBackup,
    ConfirmingSavedPasskey,
    AwaitingSavedPasskeyConfirmation(SavedPasskeyConfirmationMode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupRestoreOutcome {
    ProgressCleared,
    ProgressReported(CloudBackupRestoreFlow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupDetailOutcome {
    Cleared,
    Refreshed(CloudBackupDetail),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupVerificationOutcome {
    Idle,
    Started,
    Verified(DeepVerificationReport),
    PasskeyConfirmed,
    Failed(DeepVerificationFailure),
    Cancelled,
}

impl CloudBackupVerificationOutcome {
    pub(crate) fn from_state(state: VerificationState) -> Self {
        match state {
            VerificationState::Idle => Self::Idle,
            VerificationState::Verifying => Self::Started,
            VerificationState::Verified(report) => Self::Verified(report),
            VerificationState::PasskeyConfirmed => Self::PasskeyConfirmed,
            VerificationState::Failed(failure) => Self::Failed(failure),
            VerificationState::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupRecoveryOutcome {
    Idle,
    Started(RecoveryAction),
    Failed { action: RecoveryAction, error: String },
}

impl CloudBackupRecoveryOutcome {
    pub(crate) fn from_state(state: RecoveryState) -> Self {
        match state {
            RecoveryState::Idle => Self::Idle,
            RecoveryState::Recovering(action) => Self::Started(action),
            RecoveryState::Failed { action, error } => Self::Failed { action, error },
        }
    }
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
            A::DisableCloudBackup => CLOUD_BACKUP_MANAGER.disable_cloud_backup(),
            A::KeepCloudBackupEnabled => CLOUD_BACKUP_MANAGER.keep_cloud_backup_enabled(),
            A::RefreshDetail => CLOUD_BACKUP_MANAGER.clone().spawn_refresh_detail(),
            A::EnterDetail => CLOUD_BACKUP_MANAGER.clone().spawn_enter_detail(),
        }
    }
}

impl RustCloudBackupManager {
    fn start_verification(&self, source: CloudBackupVerificationSource) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt before verification: {error}");
        }

        if self.has_pending_cloud_upload_verification() {
            self.apply_verification_effect(
                CloudBackupVerificationCoordinator::begin_background_confirmation(source),
            );
            self.resume_pending_cloud_upload_verification();
            return;
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
        self.apply_verification_effect(CloudBackupVerificationCoordinator::dismiss_decision(
            self.current_verification_source(),
        ));
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
            self.state.read().other_backups_operation(),
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
            self.state.read().other_backups_operation(),
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
                    self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
                }
                self.apply_verification_effect(
                    CloudBackupVerificationCoordinator::begin_background_confirmation(
                        self.current_verification_source(),
                    ),
                );
            }
            DeepVerificationResult::PasskeyConfirmed(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
                }
                self.apply_verification_outcome(CloudBackupVerificationOutcome::PasskeyConfirmed);
            }
            DeepVerificationResult::PasskeyMissing(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
                }
                self.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
                self.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
            }
            DeepVerificationResult::UserCancelled(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
                }
                self.apply_verification_outcome(CloudBackupVerificationOutcome::Cancelled);
            }
            DeepVerificationResult::NotEnabled => {
                self.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
                self.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
            }
            DeepVerificationResult::Failed(failure) => {
                self.apply_failed_verification(failure);
            }
        }
    }
}
