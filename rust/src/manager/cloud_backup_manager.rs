pub(crate) mod actors;
mod blob_state;
mod catastrophic_recovery;
mod cloud_inventory;
mod cspp_exports;
mod detail;
mod dto;
mod error;
mod keychain;
mod model;
mod ops;
mod other_backups;
mod pending;
mod pending_enable;
mod pending_verification;
mod reconcile;
mod remote_inventory;
mod store;
mod sync_health;
mod verify;
mod wallet_changes;
mod wallets;

use std::sync::{Arc, LazyLock};

use act_zero::{Addr, call, send};
use cove_cspp::MasterKeyPromotionStatus;
use cove_device::cloud_storage::{CloudStorageClient, CloudSyncHealth};
use cove_device::keychain::Keychain;
use cove_tokio::task::spawn_actor;
use cove_util::ResultExt as _;
use flume::Receiver;
use parking_lot::RwLock;
use tracing::{error, info, warn};

use crate::database::Database;
pub(crate) use crate::database::cloud_backup::CloudStorageIssue;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::reconcile_channel::ReconcileChannel;
use crate::wallet::metadata::{WalletId, WalletMode as LocalWalletMode};

pub(crate) use self::actors::CloudBackupRestoreEvent;
use self::actors::{
    CloudBackupSupervisor, CloudBackupUploadedWallet, CloudBackupWalletCountRefresh,
    CloudBackupWriteBlocker, CloudBackupWriteClient, CloudBackupWriteCompletion,
    CloudBackupWriteSupervisor,
};
pub(crate) use self::detail::{
    CloudBackupCloudOnlyFetchOutcome, CloudBackupCloudOnlyOperationWarning,
    CloudBackupCloudOnlyWalletOutcome, CloudBackupDetailOutcome, CloudBackupOtherBackupsOutcome,
    CloudBackupRestoreOutcome,
};
pub use self::detail::{
    CloudBackupVerificationPresentation, CloudBackupVerificationReason,
    CloudBackupVerificationSource, CloudOnlyOperation, CloudOnlyState,
};
pub(crate) use self::detail::{
    PendingUploadVerificationState, RecoveryAction, RecoveryState, SyncState, VerificationState,
};
pub use self::dto::{
    CloudBackupDetail, CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    CloudBackupOnboardingCompletionReadiness, CloudBackupOtherBackupsState,
    CloudBackupOtherBackupsSummary, CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint,
    CloudBackupProgress, CloudBackupRestoreReport, CloudBackupRetryAction, CloudBackupRootPrompt,
    CloudBackupSettingsRowStatus, CloudBackupVerificationMetadata, CloudBackupWalletItem,
    CloudBackupWalletRestoreFailure, CloudBackupWalletStatus, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, OtherBackupsOperation, RecordId,
    SavedPasskeyConfirmationMode,
};
pub type CloudBackupManagerAction = self::dto::CloudBackupManagerAction;
pub type CloudBackupState = self::dto::CloudBackupState;
pub use self::error::CloudBackupDriveAccountSwitchError;
pub(crate) use self::error::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_DISABLE_ERROR_MESSAGE,
    CLOUD_BACKUP_LABELS_WARNING_MESSAGE, CLOUD_BACKUP_RECREATE_MESSAGE,
    CLOUD_BACKUP_REINITIALIZE_MESSAGE, CloudBackupError, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE,
    blocking_cloud_error, is_connectivity_related_issue, is_provider_wide_interruption,
    offline_error_for_step,
};
pub(crate) use self::keychain::CloudBackupKeychain;
#[cfg(test)]
pub(crate) use self::model::test_support;
pub(crate) use self::model::{
    CloudBackupAcceptedEnablePrompt, CloudBackupDetailInventorySnapshot,
    CloudBackupDetailInventorySnapshotResult, CloudBackupDetailResult, CloudBackupDisableOutcome,
    CloudBackupEnableState, CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
    CloudBackupStateReducer, CloudBackupStateReducerEvent, CloudBackupStatus,
};
pub use self::model::{
    CloudBackupInventoryIncompleteReason, CloudBackupLifecycle,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
    CloudBackupRestoreAllState, CloudBackupRestoreFlow,
};
pub(crate) use self::ops::{
    CloudBackupDisablePreparation, CloudBackupEnablePasskeyPreparation,
    CloudBackupEnablePasskeyRegistration, CloudBackupEnablePreparation,
    CloudBackupEnableRecoveryCompletion, CloudBackupEnableRecoveryPreparation,
    CloudBackupKeepEnabledPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupPreparedCloudWalletDelete, CloudBackupPreparedRestoreAll,
    CloudBackupReadyEnableUpload, CloudBackupRegisteredEnablePasskey, CloudBackupReuploadedWallets,
    CloudBackupSavedPasskeyConfirmation, CloudBackupUploadedEnableBackup,
    EnablePasskeyRegistrationFlow,
};
#[cfg(test)]
pub(crate) use self::pending_enable::PendingEnableSessionMaterial;
pub(crate) use self::pending_enable::{
    PENDING_ENABLE_JOURNAL_VERSION, PendingEnableCoordinator, PendingEnableJournal,
    PendingEnableJournalPhase, PendingEnableLocalMetadataSnapshot, PendingEnableNamespaceOwnership,
    PendingEnablePasskeyMetadata, PendingEnableSession,
};
pub(crate) use self::pending_verification::{
    PendingVerificationCompletion, PendingVerificationUpload,
};
use self::reconcile::CloudBackupReconcileMessage;
pub use self::reconcile::{DriveAccountSwitchPlatformState, DriveAccountSwitchReconcileAction};
pub(crate) use self::remote_inventory::current_namespace_wallet_record_ids;
pub(crate) use self::store::CloudBackupStore;
pub(crate) use self::sync_health::SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE;
pub(crate) use self::wallet_changes::{LIVE_UPLOAD_DEBOUNCE, live_upload_retry_delay_for_attempt};
#[cfg(test)]
pub(crate) use self::wallets::UnpersistedPrfKey;
use super::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};
pub(crate) use cspp_exports::master_key_wrapper_revision_hash;

type LocalWalletSecret = crate::backup::model::WalletSecret;

const PASSKEY_RP_ID: &str = "covebitcoinwallet.com";
pub(crate) const CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE: &str = concat!(
    "Cloud Backup local state could not be read. ",
    "Contact support before changing Cloud Backup settings."
);
pub(crate) const CLOUD_BACKUP_IO_CONCURRENCY: usize = 4;
type Message = CloudBackupReconcileMessage;

pub(crate) fn current_timestamp() -> u64 {
    jiff::Timestamp::now().as_second().try_into().unwrap_or(0)
}

pub static CLOUD_BACKUP_MANAGER: LazyLock<Arc<RustCloudBackupManager>> =
    LazyLock::new(RustCloudBackupManager::init);

/// User intent routed from Swift or Kotlin into the Rust cloud backup manager
#[uniffi::remote(Enum)]
pub enum CloudBackupManagerAction {
    EnableCloudBackup(CloudBackupEnableContext),
    EnableCloudBackupForceNew(CloudBackupEnableContext),
    EnableCloudBackupNoDiscovery(CloudBackupEnableContext),
    ConfirmSavedPasskey,
    DiscardPendingEnableCloudBackup,
    ConfirmPendingEnableCleanup,
    DismissPasskeyChoicePrompt,
    DismissMissingPasskeyReminder,
    RestoreFromCloudBackup,
    CancelRestore,
    StartVerification(CloudBackupVerificationSource),
    StartVerificationDiscoverable(CloudBackupVerificationSource),
    DismissVerificationPrompt,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
    RepairPasskeyNoDiscovery,
    SyncUnsynced,
    FetchCloudOnly,
    RestoreCloudWallet(RecordId),
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    DeleteCloudWallet(RecordId),
    RecoverOtherBackups,
    DeleteOtherBackups,
    DisableCloudBackup,
    KeepCloudBackupEnabled,
    RefreshDetail,
    EnterDetail,
    CloseDetail,
    PromptEnablePasskeyChoice(CloudBackupEnableContext),
    AcceptEnablePrompt(CloudBackupEnablePromptChoice),
}

/// Trust failure that tells the UI which recovery path is valid
#[uniffi::remote(Enum)]
pub enum DeepVerificationFailure {
    /// Transient iCloud/network/passkey error — safe to retry
    Retry {
        message: String,
        detail: Option<CloudBackupDetail>,
        retry_action: Option<CloudBackupRetryAction>,
    },
    /// Manifest missing, master key verified intact — recreate from local wallets
    RecreateManifest { message: String, warning: String, detail: Option<CloudBackupDetail> },
    /// No verified cloud or local master key available — full re-enable needed
    ReinitializeBackup { message: String, warning: String, detail: Option<CloudBackupDetail> },
    /// Backup uses a newer format — do not overwrite
    UnsupportedVersion { message: String, detail: Option<CloudBackupDetail> },
}

/// Top-level state snapshot exposed to platform managers
#[uniffi::remote(Record)]
pub struct CloudBackupState {
    pub lifecycle: CloudBackupLifecycle,
    pub settings_row_status: CloudBackupSettingsRowStatus,
}

#[uniffi::export]
impl DeepVerificationFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Retry { message, .. }
            | Self::RecreateManifest { message, .. }
            | Self::ReinitializeBackup { message, .. }
            | Self::UnsupportedVersion { message, .. } => message.clone(),
        }
    }
}

#[cfg_attr(
    doc,
    doc = "Single entry point for Cloud Backup orchestration, UniFFI calls, and reconcile wiring",
    doc = "",
    doc = "The intentional non-UI reach-ins are wallet-set notifications from",
    doc = "`pending_wallet_manager.rs` and `import_wallet_manager.rs`, plus the import",
    doc = "wallet backup-change notification that triggers re-verification."
)]
#[derive(Clone, Debug, uniffi::Object)]
pub struct RustCloudBackupManager {
    pub(crate) state: Arc<RwLock<CloudBackupStateReducer>>,
    pub(crate) reconciler: ReconcileChannel<Message>,
    cloud_only_detail_snapshot: Arc<RwLock<Option<CloudBackupDetail>>>,
    pub(crate) pending_enable: PendingEnableCoordinator,
    cloud_writes: Addr<CloudBackupWriteSupervisor>,
    pub(crate) supervisor: Addr<CloudBackupSupervisor>,
}

impl RustCloudBackupManager {
    pub(crate) fn load_persisted_state() -> PersistedCloudBackupState {
        Database::global().cloud_backup_state.get().unwrap_or_else(|error| {
            error!("Failed to load cloud backup state: {error}");
            PersistedCloudBackupState::default()
        })
    }

    pub(crate) fn runtime_status_for(state: &PersistedCloudBackupState) -> CloudBackupStatus {
        match state.status() {
            PersistedCloudBackupStatus::Disabled => CloudBackupStatus::Disabled,
            PersistedCloudBackupStatus::Disabling => CloudBackupStatus::Disabling,
            PersistedCloudBackupStatus::Enabled | PersistedCloudBackupStatus::Unverified => {
                CloudBackupStatus::Enabled
            }
            PersistedCloudBackupStatus::PasskeyMissing => CloudBackupStatus::PasskeyMissing,
            PersistedCloudBackupStatus::Corrupted => {
                CloudBackupStatus::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into())
            }
        }
    }

    pub(crate) fn status_for_operation_error(error: &CloudBackupError) -> CloudBackupStatus {
        match error {
            CloudBackupError::UnsupportedPasskeyProvider => {
                CloudBackupStatus::UnsupportedPasskeyProvider
            }
            other => CloudBackupStatus::Error(other.reader_message()),
        }
    }

    pub(crate) fn current_status(&self) -> CloudBackupStatus {
        self.state.read().status().clone()
    }

    pub(crate) fn projected_exclusive_operation(
        &self,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        self.state.read().active_operation()
    }

    pub(crate) fn project_exclusive_operation_started(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(claim));
    }

    pub(crate) fn project_exclusive_operation_finished(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(claim));
    }

    pub(crate) fn project_enable_context_started(&self, context: CloudBackupEnableContext) {
        self.apply_model_event(CloudBackupStateReducerEvent::EnableContextStarted(context));
    }

    pub(crate) fn project_pending_enable_recovery(
        &self,
        recovery: CloudBackupPendingEnableRecovery,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::PendingEnableRecoveryProjected(
            recovery,
        ));
    }

    fn has_in_flight_lifecycle(status: &CloudBackupStatus) -> bool {
        matches!(
            status,
            CloudBackupStatus::Disabling
                | CloudBackupStatus::Enabling
                | CloudBackupStatus::Restoring
        )
    }

    fn has_in_flight_operation(&self) -> bool {
        self.projected_exclusive_operation().is_some()
            || Self::has_in_flight_lifecycle(&self.current_status())
    }

    pub(crate) fn cloud_backup_writes_blocked(&self) -> bool {
        let disable_active = self
            .projected_exclusive_operation()
            .is_some_and(|claim| claim.operation() == CloudBackupExclusiveOperation::Disable);

        if disable_active {
            return true;
        }

        // keep this DB read so restarts preserve destructive-operation write fences
        let persisted = Self::load_persisted_state();
        persisted.is_disabling() || persisted.drive_account_switch().is_some()
    }

    pub(crate) fn ensure_cloud_backup_writes_allowed(&self) -> Result<(), CloudBackupError> {
        if self.cloud_backup_writes_blocked() {
            return Err(CloudBackupError::Deferred(
                "cloud backup writes are paused during an exclusive operation".into(),
            ));
        }

        Ok(())
    }

    pub(crate) async fn upload_cloud_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::new(self.cloud_writes.clone())
            .upload_wallet_backup(cloud, namespace, record_id, data)
            .await
    }

    pub(crate) async fn upload_cloud_wallet_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::new(self.cloud_writes.clone())
            .upload_wallet_backup_with_completion(cloud, namespace, record_id, data, completion)
            .await
    }

    pub(crate) async fn complete_cloud_wallet_upload_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::new(self.cloud_writes.clone())
            .complete_uploaded_wallet_batch(cloud, namespace_id, uploaded_wallets, count_refresh)
            .await
    }

    pub(crate) fn persistable_cloud_storage_issue(
        issue: CloudStorageIssue,
    ) -> Option<CloudStorageIssue> {
        issue.persistable()
    }

    pub(crate) fn connection_status(&self) -> ConnectivityStatus {
        CONNECTIVITY_MANAGER.connection_status()
    }

    pub(crate) fn is_known_offline(&self) -> bool {
        CONNECTIVITY_MANAGER.known_disconnected()
    }

    pub(crate) fn offline_error_for_step(&self, step: BlockingCloudStep) -> CloudBackupError {
        offline_error_for_step(step)
    }

    pub(crate) fn ensure_cloud_connectivity(
        &self,
        step: BlockingCloudStep,
    ) -> Result<(), CloudBackupError> {
        if self.is_known_offline() {
            return Err(offline_error_for_step(step));
        }

        Ok(())
    }

    fn init() -> Arc<Self> {
        let manager = Arc::new_cyclic(|manager| {
            let cloud_writes = spawn_actor(CloudBackupWriteSupervisor::new(manager.clone()));
            Self {
                state: Arc::new(RwLock::new(CloudBackupStateReducer::default())),
                reconciler: ReconcileChannel::new(1000),
                cloud_only_detail_snapshot: Arc::new(RwLock::new(None)),
                pending_enable: PendingEnableCoordinator::new(Keychain::global().clone()),
                cloud_writes: cloud_writes.clone(),
                supervisor: spawn_actor(CloudBackupSupervisor::new(manager.clone(), cloud_writes)),
            }
        });

        manager.sync_persisted_state();
        send!(manager.supervisor.resume_pending_enable_after_restart());
        manager.start_connectivity_listener();
        manager.resume_persisted_disable_if_needed();
        manager
    }

    fn start_connectivity_listener(self: &Arc<Self>) {
        // use a weak reference so the listener thread exits when the manager is dropped
        let manager = Arc::downgrade(self);
        let receiver = CONNECTIVITY_MANAGER.subscribe();

        std::thread::spawn(move || {
            while receiver.recv().is_ok() {
                let Some(manager) = manager.upgrade() else {
                    break;
                };

                let status = CONNECTIVITY_MANAGER.connection_status();
                manager.handle_connectivity_change(status);
            }
        });
    }

    pub(crate) fn handle_connectivity_change(&self, status: ConnectivityStatus) {
        if status != ConnectivityStatus::Connected {
            return;
        }

        if self.cloud_backup_writes_blocked() {
            self.resume_persisted_disable_if_needed();
            return;
        }

        send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
        send!(self.supervisor.wake_pending_upload_verifier());
        send!(self.supervisor.provider_inventory_did_change());
        self.start_pending_upload_verification_loop();
        self.resume_failed_connectivity_verification();
    }

    fn resume_failed_connectivity_verification(&self) {
        let retry_action = {
            let state = self.state.read();
            match state.verification() {
                VerificationState::Failed(failure) => failure.connectivity_retry_action(),
                _ => None,
            }
        };

        match retry_action {
            Some(CloudBackupRetryAction::Verify) => {
                send!(self.supervisor.start_verification(false))
            }
            Some(CloudBackupRetryAction::VerifyDiscoverable) => {
                send!(self.supervisor.start_verification(true));
            }
            None => {}
        }
    }

    pub(crate) fn present_existing_backup_found_prompt(
        &self,
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::ExistingBackupFoundPromptSet {
            context,
            passkey_hint,
        });
    }

    pub(crate) fn clear_existing_backup_found_prompt(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::ExistingBackupFoundPromptCleared);
    }

    pub(crate) fn present_passkey_choice_prompt(&self, intent: CloudBackupPasskeyChoiceIntent) {
        self.apply_model_event(CloudBackupStateReducerEvent::PasskeyChoicePromptSet(intent));
    }

    pub(crate) fn clear_passkey_choice_prompt(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::PasskeyChoicePromptCleared);
    }

    pub(crate) fn accept_enable_prompt(&self, choice: CloudBackupEnablePromptChoice) {
        let (accepted, effects) = {
            let mut state = self.state.write();
            state.accept_enable_prompt(choice)
        };
        self.send_model_effects(effects);

        match accepted {
            Some(CloudBackupAcceptedEnablePrompt::Enable(context)) => {
                self.enable_cloud_backup(context);
            }
            Some(CloudBackupAcceptedEnablePrompt::ForceNew(context)) => {
                self.enable_cloud_backup_force_new(context);
            }
            Some(CloudBackupAcceptedEnablePrompt::NoDiscovery(context)) => {
                self.enable_cloud_backup_no_discovery(context);
            }
            None => {}
        }
    }

    pub(crate) fn dismiss_missing_passkey_prompt(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::MissingPasskeyPromptDismissed);
    }

    pub(crate) fn clear_enable_progress_report(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::EnableProgressReported(None));
    }

    pub(crate) fn report_enable_progress(&self, progress: CloudBackupProgress) {
        self.apply_model_event(CloudBackupStateReducerEvent::EnableProgressReported(Some(
            progress,
        )));
    }

    pub(crate) fn apply_enable_state(&self, enable_state: CloudBackupEnableState) {
        self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(enable_state));
    }

    pub(crate) fn apply_restore_outcome(&self, outcome: CloudBackupRestoreOutcome) {
        match outcome {
            CloudBackupRestoreOutcome::ProgressCleared => {}
            CloudBackupRestoreOutcome::ProgressReported(progress) => {
                self.apply_model_event(CloudBackupStateReducerEvent::RestoreProgressReported(
                    progress,
                ));
            }
        }
    }

    pub(crate) fn refresh_sync_health(&self) {
        send!(self.supervisor.request_sync_health_refresh());
    }

    pub(crate) fn apply_verification_state(&self, verification: VerificationState) {
        if matches!(
            verification,
            VerificationState::Idle | VerificationState::Failed(_) | VerificationState::Cancelled
        ) {
            self.clear_runtime_passkey_authorization();
        }

        self.apply_model_event(CloudBackupStateReducerEvent::VerificationStateResolved(
            verification,
        ));
    }

    pub(crate) fn apply_sync_state(&self, sync: SyncState) {
        self.apply_model_event(CloudBackupStateReducerEvent::SyncStateResolved(sync));
    }

    pub(crate) fn apply_recovery_state(&self, recovery: RecoveryState) {
        if !matches!(recovery, RecoveryState::Idle) {
            self.clear_runtime_passkey_authorization();
        }

        self.apply_model_event(CloudBackupStateReducerEvent::RecoveryStateResolved(recovery));
    }

    pub(crate) fn apply_disable_outcome(&self, outcome: CloudBackupDisableOutcome) {
        self.apply_model_event(CloudBackupStateReducerEvent::DisableStateResolved(outcome));
    }

    pub(crate) fn clear_runtime_passkey_authorization(&self) {
        send!(self.supervisor.clear_runtime_passkey_authorization());
    }

    pub(crate) fn clear_in_process_state_for_local_reset(&self) {
        let supervisor = self.supervisor.clone();
        if let Err(error) = cove_tokio::task::block_on(async move {
            call!(supervisor.clear_upload_runtime_state()).await
        }) {
            error!("Failed to clear cloud backup runtime state during local reset: {error}");
        }

        self.clear_prompt_state();
        self.clear_enable_progress_report();
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.observe_sync_health(CloudSyncHealth::Unknown);
        self.apply_enable_state(CloudBackupEnableState::Idle);
        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        self.apply_detail_outcome(CloudBackupDetailOutcome::Cleared);
        self.apply_verification_state(VerificationState::Idle);
        self.apply_sync_state(SyncState::Idle);
        self.apply_recovery_state(RecoveryState::Idle);
        self.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Reset);
        self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
    }

    pub(crate) fn mutate_persisted_cloud_backup_state<T>(
        &self,
        context: &str,
        mutation: impl FnOnce(&mut PersistedCloudBackupState) -> T,
    ) -> Result<T, CloudBackupError> {
        let committed = Database::global()
            .cloud_backup_state
            .mutate(mutation)
            .map_err(|source| CloudBackupError::internal_context(context, source))?;

        self.reconcile_runtime_status(Self::runtime_status_for(&committed.state));
        self.refresh_persisted_flags();

        Ok(committed.outcome)
    }

    pub(crate) fn dismiss_verification_prompt_impl(&self) -> Result<(), CloudBackupError> {
        let dismissed_at = crate::manager::cloud_backup_manager::current_timestamp();
        self.mutate_persisted_cloud_backup_state(
            "persist cloud backup prompt dismissal",
            |state| state.dismiss_verification_request(dismissed_at),
        )?;

        Ok(())
    }

    fn current_namespace_id(&self) -> Result<String, CloudBackupError> {
        CloudBackupKeychain::global()
            .namespace_id()
            .ok_or_else(|| CloudBackupError::Internal("namespace_id not found in keychain".into()))
    }

    #[cfg(test)]
    pub(crate) fn clear_pending_enable_session(&self) {
        send!(self.supervisor.clear_pending_enable_session());
    }

    pub(crate) fn replace_pending_verification_completion(
        &self,
        completion: PendingVerificationCompletion,
    ) -> Result<(), CloudBackupError> {
        self.replace_pending_verification_completion_for_source(
            completion,
            self.current_verification_source(),
        )
    }

    pub(crate) fn replace_pending_verification_completion_for_source(
        &self,
        completion: PendingVerificationCompletion,
        source: CloudBackupVerificationSource,
    ) -> Result<(), CloudBackupError> {
        let replaced = self.mutate_persisted_cloud_backup_state(
            "persist pending verification completion",
            |state| state.replace_pending_verification_completion(completion.clone()),
        )?;
        if !replaced {
            return Err(CloudBackupError::Internal(
                "pending verification completion requires configured cloud backup state".into(),
            ));
        }

        self.activate_persisted_pending_verification_completion_for_source(completion, source)
    }

    pub(crate) fn activate_persisted_pending_verification_completion_for_source(
        &self,
        completion: PendingVerificationCompletion,
        source: CloudBackupVerificationSource,
    ) -> Result<(), CloudBackupError> {
        let state = Database::global().cloud_backup_state.get().map_err(|error| {
            CloudBackupError::internal_context("read pending verification completion", error)
        })?;
        if state.pending_verification_completion() != Some(&completion) {
            return Err(CloudBackupError::Internal(
                "pending verification completion was not durably persisted".into(),
            ));
        }

        send!(self.supervisor.cache_pending_verification_completion(completion));
        self.reconcile_pending_upload_verification_for_source(
            PendingUploadVerificationState::Confirming,
            source,
        );

        Ok(())
    }

    pub(crate) fn pending_verification_completion(&self) -> Option<PendingVerificationCompletion> {
        Self::load_persisted_state().pending_verification_completion().cloned()
    }

    pub(crate) fn clear_pending_verification_completion(&self) {
        let cleared = self.mutate_persisted_cloud_backup_state(
            "clear pending verification completion",
            PersistedCloudBackupState::clear_pending_verification_completion,
        );
        let cleared = match cleared {
            Ok(cleared) => cleared,
            Err(error) => {
                error!("Failed to clear pending verification completion: {error}");
                return;
            }
        };
        if !cleared {
            send!(self.supervisor.clear_pending_verification_completion());
            return;
        }

        send!(self.supervisor.clear_pending_verification_completion());
        self.refresh_pending_upload_verification_state();
    }

    pub(crate) fn project_exclusive_operation_failed(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        error: &CloudBackupError,
    ) {
        self.project_exclusive_operation_finished(claim);
        self.clear_enable_progress_report();
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.apply_enable_state(CloudBackupEnableState::Idle);
        self.reconcile_runtime_status(Self::status_for_operation_error(error));
    }
}

#[cfg(test)]
mod manager_test_support {
    use super::*;

    impl RustCloudBackupManager {
        pub(crate) fn persist_cloud_backup_state(
            &self,
            state: &PersistedCloudBackupState,
            context: &str,
        ) -> Result<(), CloudBackupError> {
            Database::global()
                .cloud_backup_state
                .set(state)
                .map_err(|source| CloudBackupError::internal_context(context, source))?;

            self.reconcile_runtime_status(Self::runtime_status_for(state));
            self.refresh_persisted_flags();

            Ok(())
        }

        pub(crate) fn model_snapshot(&self) -> test_support::CloudBackupModelSnapshot {
            self.state.read().snapshot()
        }

        pub(crate) fn debug_reset_cloud_backup_state(&self) {
            if let Err(error) = CloudBackupKeychain::global().clear_local_state() {
                error!("Failed to clear cloud backup keychain state: {error}");
                return;
            }
            self.clear_pending_enable_session();

            let db = Database::global();
            let _ = db.cloud_backup_state.delete();
            let _ = db.cloud_blob_sync_states.delete_all();

            self.clear_pending_verification_completion();
            self.clear_prompt_state();
            self.clear_enable_progress_report();
            self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
            self.refresh_persisted_flags();
            self.apply_enable_state(CloudBackupEnableState::Idle);
            self.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
            self.apply_detail_outcome(CloudBackupDetailOutcome::Cleared);
            self.apply_verification_state(VerificationState::Idle);
            self.apply_sync_state(SyncState::Idle);
            self.apply_recovery_state(RecoveryState::Idle);
            self.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Reset);
            self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
            self.reconcile_runtime_status(CloudBackupStatus::Disabled);
            self.refresh_sync_health();
            send!(self.supervisor.clear_upload_runtime_state());
        }
    }
}

#[uniffi::export]
impl RustCloudBackupManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        CLOUD_BACKUP_MANAGER.clone()
    }

    pub fn state(&self) -> CloudBackupState {
        self.state.read().public_state()
    }

    /// Number of wallets in the cloud backup
    pub fn backup_wallet_count(&self) -> Option<u32> {
        let db = Database::global();
        let current = Self::load_persisted_state();

        match current.wallet_count() {
            Some(count) => Some(count),
            None if current.is_configured() => match CloudBackupStore::new(&db).wallet_count() {
                Ok(count) => {
                    let persisted = db
                        .cloud_backup_state
                        .mutate(|state| {
                            if !state.is_configured() {
                                return false;
                            }

                            state.set_wallet_count(Some(count));
                            true
                        })
                        .map(|mutation| mutation.outcome)
                        .unwrap_or(false);
                    persisted.then_some(count)
                }
                Err(error) => {
                    warn!("Failed to derive cloud backup wallet count: {error}");
                    None
                }
            },
            None => None,
        }
    }

    /// Read persisted cloud backup state from DB and update in-memory state
    ///
    /// Called after bootstrap completes so the UI reflects the correct state
    /// even before the reconciler has delivered its first message
    pub fn sync_persisted_state(&self) {
        let db_state = Self::load_persisted_state();
        if let Some(disabling) = db_state.disabling() {
            send!(self.cloud_writes.block(CloudBackupWriteBlocker::Disabling {
                operation_id: disabling.disable_generation,
            }));
        }
        if let Some(account_switch) = db_state.drive_account_switch() {
            send!(self.cloud_writes.block(CloudBackupWriteBlocker::DriveAccountSwitch {
                transition_id: account_switch.transition_id,
            }));
        }
        if !self.has_in_flight_operation() {
            self.reconcile_runtime_status(Self::runtime_status_for(&db_state));
            self.reconcile_persisted_restore_all(&db_state);
        }

        self.refresh_persisted_flags();
        if !self.has_in_flight_operation() {
            self.refresh_pending_upload_verification_state();
        }
    }

    pub fn cloud_storage_did_change(&self) {
        if self.cloud_backup_writes_blocked() {
            self.resume_persisted_disable_if_needed();
            return;
        }

        send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
        send!(self.supervisor.wake_pending_upload_verifier());
        send!(self.supervisor.provider_inventory_did_change());
        self.start_pending_upload_verification_loop();
        self.refresh_sync_health();
    }

    /// Claim the exclusive operation and return after all prior cloud writes drain
    pub async fn begin_drive_account_switch(
        &self,
    ) -> Result<u64, CloudBackupDriveAccountSwitchError> {
        let (transition_id, ready) = call!(self.supervisor.begin_drive_account_switch())
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)??;
        ready.await.map_err_str(CloudBackupDriveAccountSwitchError::Internal)??;

        Ok(transition_id.value())
    }

    /// Continue the claimed transition after Android durably stages the selected account
    pub async fn continue_drive_account_switch(
        &self,
        transition_id: u64,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        call!(self.supervisor.continue_drive_account_switch(transition_id.into()))
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
    }

    /// Move an unstarted account transition to its rollback phase
    pub async fn cancel_drive_account_switch(
        &self,
        transition_id: u64,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        call!(self.supervisor.cancel_drive_account_switch(transition_id.into()))
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
    }

    /// Release the transition after Android commits its staged account
    pub async fn confirm_drive_account_switch_committed(
        &self,
        transition_id: u64,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        call!(self.supervisor.confirm_drive_account_switch_committed(transition_id.into()))
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
    }

    /// Release the transition after Android discards its staged account
    pub async fn confirm_drive_account_switch_rolled_back(
        &self,
        transition_id: u64,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        call!(self.supervisor.confirm_drive_account_switch_rolled_back(transition_id.into()))
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
    }

    /// Reconcile persisted Rust and Android transition state after process startup
    ///
    /// Android must complete the returned action before starting its initial cloud refresh
    pub async fn reconcile_drive_account_switch(
        &self,
        platform_state: DriveAccountSwitchPlatformState,
    ) -> Result<DriveAccountSwitchReconcileAction, CloudBackupDriveAccountSwitchError> {
        call!(self.supervisor.reconcile_drive_account_switch(platform_state))
            .await
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
    }

    /// Check if cloud backup is enabled, used as nav guard
    pub fn is_cloud_backup_enabled(&self) -> bool {
        Self::load_persisted_state().is_configured()
    }

    /// Reports whether onboarding may recover a lost enable-completion event from durable state
    pub fn onboarding_enable_completion_readiness(
        &self,
    ) -> CloudBackupOnboardingCompletionReadiness {
        match CloudBackupKeychain::global().load_pending_enable_journal() {
            Ok(Some(_)) => {
                return CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery;
            }
            Ok(None) => {}
            Err(error) => {
                error!("Failed to inspect pending Cloud Backup enable journal: {error}");
                return CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery;
            }
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        match cspp.master_key_promotion_status() {
            Ok(MasterKeyPromotionStatus::None) => {}
            Ok(_) => return CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery,
            Err(error) => {
                error!("Failed to inspect staged Cloud Backup enable material: {error}");
                return CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery;
            }
        }

        match self.state().lifecycle {
            CloudBackupLifecycle::Configured(configured)
                if configured.passkey == model::CloudBackupPasskeyState::Available =>
            {
                CloudBackupOnboardingCompletionReadiness::Ready
            }
            CloudBackupLifecycle::Disabled
            | CloudBackupLifecycle::Enabling(_)
            | CloudBackupLifecycle::Restoring(_)
            | CloudBackupLifecycle::Configured(_)
            | CloudBackupLifecycle::PendingEnableRecovery(_)
            | CloudBackupLifecycle::Failed(_) => CloudBackupOnboardingCompletionReadiness::NotReady,
        }
    }

    /// Whether the persisted cloud backup state is unverified
    pub fn is_cloud_backup_unverified(&self) -> bool {
        Self::load_persisted_state().is_unverified()
    }

    /// Whether the persisted cloud backup passkey is missing
    pub fn is_cloud_backup_passkey_missing(&self) -> bool {
        Self::load_persisted_state().is_passkey_missing()
    }

    pub fn has_pending_cloud_upload_verification(&self) -> bool {
        if Self::load_persisted_state().pending_verification_completion().is_some() {
            return true;
        }

        Database::global().cloud_blob_sync_states.list().ok().is_some_and(|states| {
            states.into_iter().any(|state| state.is_uploaded_pending_confirmation())
        })
    }

    pub fn resume_pending_cloud_upload_verification(&self) {
        if self.cloud_backup_writes_blocked() {
            return;
        }

        self.sync_persisted_state();
        send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
        self.start_pending_upload_verification_loop();
    }

    /// Background startup health check for cloud backup integrity
    pub async fn verify_backup_integrity(&self) -> Option<String> {
        self.verify_backup_integrity_impl().await
    }

    /// Back up a newly created wallet, fire-and-forget
    ///
    /// Returns immediately unless cloud backup is configured or disabling
    pub fn backup_new_wallet(&self, metadata: crate::wallet::metadata::WalletMetadata) {
        // disabling can be canceled, so new wallets still need queued uploads
        if !Self::load_persisted_state().has_configured_backup() {
            return;
        }

        self.handle_wallet_backup_change_and_reverify(metadata.id);
    }
}

impl RustCloudBackupManager {
    fn reconcile_persisted_restore_all(&self, db_state: &PersistedCloudBackupState) {
        let Some(marker) = db_state.pending_restore_all() else {
            self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllReset);
            return;
        };

        if self.current_namespace_id().ok().as_deref() == Some(marker.namespace_id.as_str()) {
            self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllRetryRequired);
            return;
        }

        if let Err(error) = CloudBackupStore::global().clear_restore_all_marker() {
            error!("Failed to clear stale Restore All marker: {error}");
        }
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllReset);
    }

    pub(crate) fn enable_cloud_backup(&self, context: CloudBackupEnableContext) {
        send!(self.supervisor.start_enable_operation(context));
    }

    pub(crate) fn enable_cloud_backup_force_new(&self, context: CloudBackupEnableContext) {
        send!(self.supervisor.start_enable_force_new_operation(context));
    }

    pub(crate) fn enable_cloud_backup_no_discovery(&self, context: CloudBackupEnableContext) {
        send!(self.supervisor.start_enable_no_discovery_operation(context));
    }

    pub(crate) fn disable_cloud_backup(&self) {
        send!(self.supervisor.start_disable_operation());
    }

    pub(crate) fn keep_cloud_backup_enabled(&self) {
        send!(self.supervisor.keep_cloud_backup_enabled());
    }

    fn resume_persisted_disable_if_needed(&self) {
        if Self::load_persisted_state().is_disabling() {
            self.disable_cloud_backup();
        }
    }

    /// Dismiss staged enable state for the existing-backup confirmation flow
    pub(crate) fn discard_pending_enable_cloud_backup(&self) {
        send!(self.supervisor.discard_pending_enable_cloud_backup());
        self.clear_existing_backup_found_prompt();
    }

    pub(crate) fn cancel_restore(&self) {
        send!(self.supervisor.cancel_restore());
    }

    pub(crate) async fn cancel_restore_and_wait(&self) {
        if let Err(error) = call!(self.supervisor.cancel_restore()).await {
            warn!("restore_from_cloud_backup: failed to await restore cancellation: {error}");
        }
    }

    pub(crate) fn restore_from_cloud_backup(&self) {
        info!("restore_from_cloud_backup: enqueueing restore task");
        send!(self.supervisor.start_restore_from_cloud_backup());
    }

    pub(crate) fn restore_from_cloud_backup_with_events(
        &self,
    ) -> Receiver<CloudBackupRestoreEvent> {
        let (sender, receiver) = flume::bounded(250);
        info!("restore_from_cloud_backup: enqueueing onboarding restore task");
        send!(self.supervisor.start_restore_from_cloud_backup_with_events(sender));
        receiver
    }

    fn clear_prompt_state(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::PromptStateCleared);
    }
}

#[cfg(test)]
mod tests {
    use super::actors::restore::RestoreOperation;
    use super::ops::test_support::{
        async_test_lock, configure_enabled_cloud_backup, ensure_cloud_backup_test_tokio_runtime,
        persisted_enabled_cloud_backup_state, test_globals, test_lock,
    };
    use super::*;
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
        PersistedPasskeyState,
    };
    use act_zero::call;
    use cove_device::cloud_storage::CloudStorageError;
    use cove_device::passkey::{PasskeyError, PasskeyFailureReason, PasskeyOperation};

    fn init_manager() -> Arc<RustCloudBackupManager> {
        ensure_cloud_backup_test_tokio_runtime();
        test_globals().reset();
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        RustCloudBackupManager::init()
    }

    fn persisted_configured_state(
        verification: PersistedBackupVerificationState,
    ) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification,
            sync: PersistedBackupSyncState { last_sync: None, wallet_count: None },
            pending_verification_completion: None,
            pending_restore_all: None,
        })
    }

    fn new_restore_operation(manager: &RustCloudBackupManager) -> RestoreOperation {
        let supervisor = manager.supervisor.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let _task = cove_tokio::task::spawn(async move {
            let result = call!(supervisor.new_restore_operation()).await;
            sender.send(result).expect("send restore operation result");
        });
        receiver
            .recv()
            .expect("receive restore operation result")
            .expect("create restore operation")
    }

    fn invalidate_restore_operation(manager: &RustCloudBackupManager) {
        let supervisor = manager.supervisor.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let _task = cove_tokio::task::spawn(async move {
            let result = call!(supervisor.invalidate_restore_operation()).await;
            sender.send(result).expect("send invalidate restore operation result");
        });
        receiver
            .recv()
            .expect("receive invalidate restore operation result")
            .expect("invalidate restore operation");
    }

    fn run_on_cloud_backup_runtime<T: Send + 'static>(
        future: impl Future<Output = T> + Send + 'static,
    ) -> T {
        ensure_cloud_backup_test_tokio_runtime();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let _task = cove_tokio::task::spawn(async move {
            sender.send(future.await).expect("send cloud backup runtime result");
        });
        receiver.recv().expect("receive cloud backup runtime result")
    }

    #[test]
    fn cloud_storage_issue_classifies_typed_errors() {
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::AuthorizationRequired(
                "authorization required".into(),
            )),
            CloudStorageIssue::AuthorizationRequired
        );
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::Offline("offline".into())),
            CloudStorageIssue::Offline
        );
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::NotAvailable("not available".into())),
            CloudStorageIssue::Unavailable
        );
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::QuotaExceeded),
            CloudStorageIssue::QuotaExceeded
        );
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::NotFound("wallet".into())),
            CloudStorageIssue::NotFound
        );
    }

    #[test]
    fn persistable_cloud_storage_issue_filters_other_for_persistence() {
        assert_eq!(
            RustCloudBackupManager::persistable_cloud_storage_issue(CloudStorageIssue::Offline),
            Some(CloudStorageIssue::Offline)
        );
        assert_eq!(
            RustCloudBackupManager::persistable_cloud_storage_issue(CloudStorageIssue::Other),
            None
        );
    }

    #[test]
    fn corrupted_persisted_state_projects_runtime_error() {
        assert_eq!(
            RustCloudBackupManager::runtime_status_for(&PersistedCloudBackupState::corrupted(
                "decode failed"
            )),
            CloudBackupStatus::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into())
        );
    }

    #[test]
    fn pending_verification_completion_expires_future_created_at() {
        let mut completion = PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            "namespace".into(),
            Vec::new(),
        );
        completion.created_at = Some(11);

        assert!(completion.is_expired(10, 60));
    }

    #[test]
    fn pending_verification_completion_rejects_disabled_state_without_projection() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let completion = PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            "namespace".into(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        );

        let error = manager.replace_pending_verification_completion(completion).unwrap_err();

        assert!(matches!(error, CloudBackupError::Internal(_)));
        assert!(manager.pending_verification_completion().is_none());
        assert_eq!(
            manager.model_snapshot().pending_upload_verification,
            PendingUploadVerificationState::Idle
        );
    }

    #[test]
    fn opaque_upload_messages_are_not_classified_by_text() {
        assert_eq!(
            CloudStorageIssue::from(&CloudStorageError::UploadFailed(
                "authorization required".into()
            )),
            CloudStorageIssue::Other
        );
    }

    #[test]
    fn convert_cloud_secret_mnemonic() {
        let secret = cove_cspp::backup_data::WalletSecret::Mnemonic("abandon".into());
        let result = wallets::tests::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::Mnemonic(ref m) if m == "abandon"));
    }

    #[test]
    fn convert_cloud_secret_tap_signer() {
        let secret = cove_cspp::backup_data::WalletSecret::TapSignerBackup(vec![1, 2, 3]);
        let result = wallets::tests::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::TapSignerBackup(ref b) if b == &[1, 2, 3]));
    }

    #[test]
    fn convert_cloud_secret_descriptor_to_none() {
        let secret = cove_cspp::backup_data::WalletSecret::Descriptor("wpkh(...)".into());
        let result = wallets::tests::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::None));
    }

    #[test]
    fn convert_cloud_secret_watch_only_to_none() {
        let result =
            wallets::tests::convert_cloud_secret(&cove_cspp::backup_data::WalletSecret::WatchOnly);
        assert!(matches!(result, LocalWalletSecret::None));
    }

    #[test]
    fn restore_progress_updates_state() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let progress = CloudBackupRestoreFlow::Downloading { completed: 1, total: 2 };

        manager.reconcile_runtime_status(CloudBackupStatus::Restoring);
        manager
            .apply_restore_outcome(CloudBackupRestoreOutcome::ProgressReported(progress.clone()));

        assert_eq!(manager.state.read().snapshot().restore_progress, Some(progress));
    }

    #[test]
    fn verification_metadata_is_not_configured_when_backup_is_disabled() {
        let db_state = PersistedCloudBackupState::default();

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::NotConfigured,
        );
    }

    #[test]
    fn verification_metadata_is_configured_never_verified_without_timestamp() {
        let db_state = persisted_enabled_cloud_backup_state(None);

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::ConfiguredNeverVerified,
        );
    }

    #[test]
    fn verification_metadata_is_verified_with_timestamp() {
        let db_state = persisted_configured_state(PersistedBackupVerificationState::Verified {
            last_verified_at: 21,
            requested_at: None,
            dismissed_at: None,
        });

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::Verified(21),
        );
    }

    #[test]
    fn verification_metadata_is_needs_verification_when_unverified() {
        let db_state = persisted_configured_state(PersistedBackupVerificationState::Required {
            last_verified_at: Some(21),
            requested_at: None,
            dismissed_at: None,
        });

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::NeedsVerification,
        );
    }

    #[test]
    fn restore_complete_configures_lifecycle_without_report() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.reconcile_runtime_status(CloudBackupStatus::Restoring);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressReported(
            CloudBackupRestoreFlow::Restoring { completed: 1, total: 2 },
        ));
        manager.reconcile_runtime_status(CloudBackupStatus::Enabled);

        assert!(manager.state.read().snapshot().restore_progress.is_none());
    }

    #[test]
    fn terminal_status_clears_restore_progress_without_report() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.reconcile_runtime_status(CloudBackupStatus::Restoring);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressReported(
            CloudBackupRestoreFlow::Restoring { completed: 1, total: 2 },
        ));
        manager.reconcile_runtime_status(CloudBackupStatus::Error("all wallets failed".into()));

        let state = manager.state.read();
        assert!(state.snapshot().restore_progress.is_none());
        assert!(matches!(state.public_state().lifecycle, CloudBackupLifecycle::Failed(_)));
    }

    #[test]
    fn unsupported_passkey_provider_maps_to_typed_status() {
        assert_eq!(
            RustCloudBackupManager::status_for_operation_error(
                &CloudBackupError::UnsupportedPasskeyProvider,
            ),
            CloudBackupStatus::UnsupportedPasskeyProvider,
        );
    }

    #[test]
    fn raw_passkey_diagnostic_cannot_reach_reader_facing_status() {
        let diagnostic =
            "Q8UP8C53Y8 org.bitcoinppl.cove webcredentials:covebitcoinwallet.com credential=secret";
        let error = CloudBackupError::from(PasskeyError::RequestFailed {
            operation: PasskeyOperation::DiscoverAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: diagnostic.into() },
        });
        let status = RustCloudBackupManager::status_for_operation_error(&error);

        assert_eq!(
            status,
            CloudBackupStatus::Error("Cove couldn't use this passkey. Please try again.".into())
        );
        for marker in [
            "Q8UP8C53Y8",
            "org.bitcoinppl.cove",
            "covebitcoinwallet.com",
            "webcredentials",
            "credential",
            "secret",
        ] {
            assert!(!format!("{status:?}").contains(marker));
        }
    }

    #[test]
    fn stale_restore_operation_cannot_update_restore_progress() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);
        let restore_progress_before_stale_outcome =
            manager.state.read().snapshot().restore_progress;
        assert_eq!(restore_progress_before_stale_outcome, Some(CloudBackupRestoreFlow::Finding));
        let progress = CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 };

        let error = run_on_cloud_backup_runtime({
            let progress = progress.clone();
            async move {
                stale_operation
                    .apply_outcome(CloudBackupRestoreOutcome::ProgressReported(progress))
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(
            manager.state.read().snapshot().restore_progress,
            restore_progress_before_stale_outcome
        );

        run_on_cloud_backup_runtime({
            let progress = progress.clone();
            async move {
                current_operation.apply_status(CloudBackupStatus::Restoring).await.unwrap();
                current_operation
                    .apply_outcome(CloudBackupRestoreOutcome::ProgressReported(progress))
                    .await
                    .unwrap()
            }
        });

        assert_eq!(manager.state.read().snapshot().restore_progress, Some(progress));
    }

    #[test]
    fn stale_restore_operation_cannot_update_status() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);

        let error = run_on_cloud_backup_runtime({
            async move { stale_operation.apply_status(CloudBackupStatus::Restoring).await.unwrap_err() }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Restoring);

        run_on_cloud_backup_runtime({
            async move { current_operation.apply_status(CloudBackupStatus::Restoring).await.unwrap() }
        });

        assert_eq!(manager.state.read().status(), CloudBackupStatus::Restoring);
    }

    #[test]
    fn stale_restore_operation_cannot_persist_cloud_backup_state() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let db = Database::global();
        db.cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        manager.reconcile_runtime_status(CloudBackupStatus::Disabled);

        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);
        let persisted_state = persisted_enabled_cloud_backup_state(None);

        let error = run_on_cloud_backup_runtime({
            let persisted_state = persisted_state.clone();
            async move {
                stale_operation
                    .persist_cloud_backup_state(
                        persisted_state,
                        "test stale restore persist".into(),
                    )
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(db.cloud_backup_state.get().unwrap(), PersistedCloudBackupState::default());
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Restoring);

        run_on_cloud_backup_runtime({
            let persisted_state = persisted_state.clone();
            async move {
                current_operation
                    .persist_cloud_backup_state(
                        persisted_state,
                        "test current restore persist".into(),
                    )
                    .await
                    .unwrap()
            }
        });

        assert_eq!(db.cloud_backup_state.get().unwrap(), persisted_state);
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Restoring);
    }

    #[test]
    fn invalidated_restore_operation_becomes_cancelled() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let operation = new_restore_operation(&manager);

        invalidate_restore_operation(&manager);

        let error = run_on_cloud_backup_runtime({
            async move { operation.ensure_current().await.unwrap_err() }
        });
        assert!(matches!(error, CloudBackupError::Cancelled));
    }

    #[test]
    fn invalidated_restore_operation_cannot_update_restore_progress() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let operation = new_restore_operation(&manager);

        invalidate_restore_operation(&manager);
        assert_eq!(manager.state.read().snapshot().restore_progress, None);

        let progress = CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 };
        let error = run_on_cloud_backup_runtime(async move {
            operation
                .apply_outcome(CloudBackupRestoreOutcome::ProgressReported(progress))
                .await
                .unwrap_err()
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(manager.state.read().snapshot().restore_progress, None);
    }

    #[test]
    fn stale_restore_operation_rejects_current_check() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let stale_operation = new_restore_operation(&manager);
        let _current_operation = new_restore_operation(&manager);
        let error = run_on_cloud_backup_runtime(async move {
            stale_operation.ensure_current().await.unwrap_err()
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exclusive_operation_claims_enabling_synchronously() {
        let _guard = async_test_lock().lock().await;
        let manager = init_manager();
        manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
        manager.clear_enable_progress_report();
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);

        manager.project_exclusive_operation_started(CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            1,
        ));

        assert_eq!(manager.state.read().status(), CloudBackupStatus::Enabling);
        manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
    }

    #[test]
    fn public_state_preserves_enabling_when_persisted_state_is_disabled() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();

        manager.project_exclusive_operation_started(CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            1,
        ));

        assert!(matches!(manager.state().lifecycle, CloudBackupLifecycle::Enabling(_)));
        assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabling);
    }

    #[test]
    fn public_state_preserves_restoring_when_persisted_state_is_configured() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::mark_enabled_reset_verification(42, 2))
            .unwrap();

        manager.project_exclusive_operation_started(CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Restore,
            1,
        ));

        assert!(matches!(manager.state().lifecycle, CloudBackupLifecycle::Restoring(_)));
        assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Restoring);
    }

    #[test]
    fn sync_persisted_state_preserves_in_flight_lifecycle() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::mark_enabled_reset_verification(42, 2))
            .unwrap();

        manager.project_exclusive_operation_started(CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            1,
        ));
        manager.sync_persisted_state();

        assert!(matches!(manager.state().lifecycle, CloudBackupLifecycle::Enabling(_)));
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Enabling);
    }

    #[test]
    fn sync_persisted_restore_all_projects_retry_without_starting_work() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        let namespace_id = manager.current_namespace_id().unwrap();
        CloudBackupStore::global().persist_restore_all_marker(namespace_id).unwrap();
        let authenticate_count = globals.passkey.authenticate_count();

        manager.sync_persisted_state();
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(CloudBackupDetail {
            last_sync: None,
            up_to_date: Vec::new(),
            needs_sync: Vec::new(),
            cloud_only_count: 1,
            other_backups: CloudBackupOtherBackupsState::Loaded { summary: Default::default() },
        }));
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(vec![
            CloudBackupWalletItem {
                name: "Remaining wallet".into(),
                network: None,
                wallet_mode: None,
                wallet_type: None,
                fingerprint: None,
                label_count: None,
                backup_updated_at: None,
                sync_status: CloudBackupWalletStatus::DeletedFromDevice,
                record_id: "remaining-record".into(),
                restore_failure: None,
            },
        ]));

        let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
            panic!("expected configured state");
        };
        assert_eq!(
            configured.restore_all,
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );
        assert_eq!(manager.projected_exclusive_operation(), None);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    }

    #[test]
    fn sync_persisted_restore_all_clears_marker_for_inactive_namespace() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        CloudBackupStore::global().persist_restore_all_marker("inactive-namespace".into()).unwrap();

        manager.sync_persisted_state();

        assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
        assert_eq!(manager.projected_exclusive_operation(), None);
        assert_eq!(globals.passkey.authenticate_count(), 0);
    }
}
