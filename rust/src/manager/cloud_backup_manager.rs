pub(crate) mod actors;
mod admission;
mod blob_state;
mod catastrophic_recovery;
mod cloud_inventory;
mod cspp_exports;
mod delete_consent;
mod detail;
mod dto;
mod error;
#[cfg(test)]
mod harness;
mod inventory;
mod keychain;
mod model;
mod ops;
mod other_backups;
mod owner;
mod pending;
mod pending_enable;
mod pending_verification;
mod reconcile;
mod recovery_coverage;
mod remote_inventory;
mod screen;
mod store;
mod sync_health;
mod timing;
mod turn_off;
mod verify;
mod wallet_changes;
mod wallets;

use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use act_zero::{Addr, call, send};
use cove_cspp::MasterKeyPromotionStatus;
use cove_device::cloud_storage::CloudSyncHealth;
use cove_device::keychain::Keychain;
use cove_tokio::task::spawn_actor;
use flume::Receiver;
use parking_lot::{Condvar, Mutex, RwLock};
use tracing::{error, info, warn};

use crate::database::Database;
pub(crate) use crate::database::cloud_backup::CloudStorageIssue;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::reconcile_channel::ReconcileChannel;
use crate::wallet::metadata::{WalletId, WalletMode as LocalWalletMode};

pub(crate) use self::actors::CloudBackupRestoreEvent;
use self::actors::supervisor::{AbandonedReset, LocalResetOutcome, ResetEnd};
use self::actors::{
    CloudBackupSupervisor, CloudBackupWriteClient, CloudBackupWriteSupervisor, WriteTicket,
};
use self::admission::Rejection;
pub use self::delete_consent::{
    CloudBackupBatchDeleteConsent, CloudBackupDeleteCategory, CloudBackupRowDeleteConsent,
};
pub(crate) use self::detail::{
    CloudBackupCloudOnlyFetchOutcome, CloudBackupCloudOnlyOperationWarning,
    CloudBackupCloudOnlyWalletOutcome, CloudBackupDetailOutcome, CloudBackupOtherBackupsOutcome,
    CloudBackupRestoreOutcome, CloudOnlyFetchApplication,
};
pub use self::detail::{
    CloudBackupVerificationPresentation, CloudBackupVerificationReason,
    CloudBackupVerificationSource, CloudOnlyOperation,
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
    CloudBackupWalletRestoreFailure, CloudBackupWalletStatus, CloudBackupWalletVerificationIssues,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult, OtherBackupsOperation,
    RecordId, SavedPasskeyConfirmationMode,
};

pub type CloudBackupManagerAction = self::dto::CloudBackupManagerAction;
pub type CloudBackupState = self::dto::CloudBackupState;
pub(crate) use self::error::CloudBackupDriveAccountSwitchError;
pub(crate) use self::error::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_LABELS_WARNING_MESSAGE,
    CLOUD_BACKUP_RECREATE_MESSAGE, CLOUD_BACKUP_REINITIALIZE_MESSAGE, CloudBackupError,
    GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, PasskeyRequestFailure, blocking_cloud_error,
    is_connectivity_related_issue, is_provider_wide_interruption, offline_error_for_step,
};
pub(crate) use self::keychain::CloudBackupKeychain;
pub(crate) use self::keychain::LocalStateOwner;
#[cfg(test)]
pub(crate) use self::model::test_support;
pub(crate) use self::model::{
    CloudBackupAcceptedEnablePrompt, CloudBackupDetailInventorySnapshot,
    CloudBackupDetailInventorySnapshotResult, CloudBackupDetailProviderConfirmation,
    CloudBackupDetailResult, CloudBackupDetailSnapshotCompletion, CloudBackupEnableState,
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim, CloudBackupListedDetail,
    CloudBackupStateReducer, CloudBackupStateReducerEvent, CloudBackupStatus, CloudCopySecret,
    OwnSecretFacts,
};
pub use self::model::{
    CloudBackupConfiguredState, CloudBackupEnableFlow, CloudBackupInventoryIncompleteReason,
    CloudBackupLifecycle, CloudBackupPasskeyState, CloudBackupPendingEnableCleanupState,
    CloudBackupPendingEnableRecovery, CloudBackupRestoreAllState, CloudBackupRestoreFlow,
    CloudBackupSyncState, CloudBackupUndecryptableWalletDeletionState,
    CloudBackupVerificationState,
};
#[cfg(test)]
pub(crate) use self::ops::test_support::rebase_owners_after_test_write;
pub(crate) use self::ops::{
    CloudBackupEnablePasskeyPreparation, CloudBackupEnablePasskeyRegistration,
    CloudBackupEnablePreparation, CloudBackupEnableRecoveryCompletion,
    CloudBackupEnableRecoveryPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupPreparedCloudWalletDelete, CloudBackupPreparedRestoreAll,
    CloudBackupPreparedUndecryptableWalletDeletion, CloudBackupReadyEnableUpload,
    CloudBackupRegisteredEnablePasskey, CloudBackupReuploadedWallets,
    CloudBackupSavedPasskeyConfirmation, CloudBackupUploadedEnableBackup,
    EnablePasskeyRegistrationFlow,
};
pub(crate) use self::owner::inventory::{
    ClassificationRequestId, CloudBackupDetailState, CloudBackupInventoryAuthority, CloudOnlyFetch,
    CloudOnlyListingUpdate, CloudOnlyRow, CloudOnlyState, InventoryInput, InventoryOwner,
    InventoryReadAt, InventoryResult, InventoryScope, LoadedCloudBackupDetail, OwnSecretUpdate,
};
pub(crate) use self::owner::lifecycle::LifecycleCapability;
use self::owner::lifecycle::{DisablingRecord, LifecycleLoad, LifecycleWriter};
pub(crate) use self::pending_enable::{
    PENDING_ENABLE_JOURNAL_VERSION, PendingEnableCoordinator, PendingEnableJournal,
    PendingEnableJournalPhase, PendingEnableLocalMetadataSnapshot, PendingEnableNamespaceOwnership,
    PendingEnablePasskeyMetadata, PendingEnableSession, PendingWork, PromotionStartError,
};
pub(crate) use self::pending_verification::{
    PendingVerificationCompletion, PendingVerificationUpload,
};
use self::reconcile::CloudBackupReconcileMessage;
pub use self::reconcile::{DriveAccountSwitchPlatformResult, DriveAccountSwitchPlatformState};
pub(crate) use self::recovery_coverage::CloudBackupRecoveryCoverage;
pub(crate) use self::remote_inventory::current_namespace_wallet_record_ids;
use self::screen::PublishedScreen;
pub use self::screen::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupNoticeKind, CloudBackupPublishedScreen,
    CloudBackupScreenDelta,
};
pub(crate) use self::store::CloudBackupStore;
pub(crate) use self::sync_health::SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE;
use self::turn_off::AutomaticResume;
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

#[derive(Debug, Default)]
pub(crate) struct CloudBackupRuntimeOwnership {
    manager_initialized: bool,
    recovery_cleanup_active: bool,
}

static CLOUD_BACKUP_RUNTIME_OWNERSHIP: Mutex<CloudBackupRuntimeOwnership> =
    Mutex::new(CloudBackupRuntimeOwnership {
        manager_initialized: false,
        recovery_cleanup_active: false,
    });
static CLOUD_BACKUP_RUNTIME_CHANGED: Condvar = Condvar::new();

pub(crate) fn initialize_cloud_backup_manager() -> Arc<RustCloudBackupManager> {
    initialize_cloud_backup_manager_with(LifecycleWriter::bootstrap)
}

/// Starts a manager whose lifecycle writer comes from `bootstrap`
///
/// Production always passes [`LifecycleWriter::bootstrap`]; the parameter lets tests make the
/// load heal fail without a test seam inside the bootstrap itself
pub(crate) fn initialize_cloud_backup_manager_with(
    bootstrap: fn(&CloudBackupRuntimeOwnership) -> (LifecycleWriter, LifecycleLoad),
) -> Arc<RustCloudBackupManager> {
    // the exclusive bootstrap phase: recovery cleanup is excluded from here on, and load
    // normalization creates the lifecycle writer before any Cloud Backup actor exists
    let (lifecycle, load) = {
        let mut ownership = CLOUD_BACKUP_RUNTIME_OWNERSHIP.lock();
        while ownership.recovery_cleanup_active {
            CLOUD_BACKUP_RUNTIME_CHANGED.wait(&mut ownership);
        }
        ownership.manager_initialized = true;
        bootstrap(&ownership)
    };

    RustCloudBackupManager::init(lifecycle, load)
}

pub static CLOUD_BACKUP_MANAGER: LazyLock<Arc<RustCloudBackupManager>> =
    LazyLock::new(initialize_cloud_backup_manager);

/// Proof that database-unavailable cleanup cannot race a Cloud Backup runtime
#[derive(Debug)]
pub(crate) struct CloudBackupRecoveryCleanupPermit;

impl Drop for CloudBackupRecoveryCleanupPermit {
    fn drop(&mut self) {
        let mut ownership = CLOUD_BACKUP_RUNTIME_OWNERSHIP.lock();
        ownership.recovery_cleanup_active = false;
        CLOUD_BACKUP_RUNTIME_CHANGED.notify_all();
    }
}

/// Reserve catastrophic cleanup before the Cloud Backup manager exists
pub(crate) fn begin_recovery_cleanup() -> Result<CloudBackupRecoveryCleanupPermit, CloudBackupError>
{
    let mut ownership = CLOUD_BACKUP_RUNTIME_OWNERSHIP.lock();
    if ownership.manager_initialized || ownership.recovery_cleanup_active {
        // a precondition of catastrophic cleanup, not a wait: no manager may exist beside it
        return Err(CloudBackupError::Internal("cloud backup local writers are not absent".into()));
    }

    ownership.recovery_cleanup_active = true;
    Ok(CloudBackupRecoveryCleanupPermit)
}

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
    RestoreCloudWallet(RecordId),
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    /// Delete on a listed row, with the consent the confirmation was opened for; a stale one is
    /// rejected
    DeleteCloudWallet(CloudBackupRowDeleteConsent),
    /// Delete the backups the verified key couldn't decrypt, with the consent the confirmation was
    /// opened for; a stale one is rejected
    DeleteUndecryptableWalletBackups(CloudBackupBatchDeleteConsent),
    RecoverOtherBackups,
    DeleteOtherBackups,
    DisableCloudBackup,
    /// Confirms the review the user saw; a stale id is rejected
    ConfirmDisable {
        review_id: u64,
    },
    /// Every way out of the Turn Off review or drain: Cancel, backdrop, back, screen pop
    CancelDisable,
    /// Restore Them First on the review the user saw: ends it and restores the wallets it found
    /// only in the cloud; a stale id is rejected
    RestoreBeforeTurnOff {
        review_id: u64,
    },
    /// Verify First on the review the user saw: ends it and verifies the backups it couldn't
    /// check; a stale id is rejected
    VerifyBeforeTurnOff {
        review_id: u64,
    },
    /// Try Again on a committed Turn Off
    ResumeDisable,
    /// Finishes a committed Turn Off locally and leaves the remote data
    StopUsingOnThisDevice,
    DismissNotice(CloudBackupNoticeKind),
    /// Google Drive authorization, separate from verification
    ReconnectDrive,
    /// Moves Cloud Backup to another Google Drive account; Android only
    SwitchDriveAccount,
    /// Try Again on a Google Drive account switch that needs attention
    RetryDriveAccountSwitchRecovery,
    /// Restores the previous Google Drive account of a switch that needs attention
    CancelDriveAccountSwitch,
    RefreshDetail,
    EnterDetail,
    /// The detail screen went away; a Turn Off review still open is cancelled
    LeaveDetail,
    /// Runs the failed load again
    RetryLoad,
    /// Resets a corrupted Cloud Backup on this device
    ResetOnThisDevice,
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
    /// The inventory's only writer, changed through `apply_inventory`
    ///
    /// Taken after `state` when both are held, never before, so a reader holding the reducer
    /// can't deadlock against one holding the inventory
    pub(crate) inventory: Arc<RwLock<InventoryOwner>>,
    pub(crate) reconciler: ReconcileChannel<Message>,
    published_screen: Arc<PublishedScreen>,
    pub(crate) pending_enable: PendingEnableCoordinator,
    cloud_writes: Addr<CloudBackupWriteSupervisor>,
    pub(crate) supervisor: Addr<CloudBackupSupervisor>,
    local_reset_in_progress: Arc<AtomicBool>,
    /// Whether the last pending verification read of blob sync states failed, so a lasting
    /// database error logs once per change instead of on every queue pass
    pending_sync_states_unreadable: Arc<AtomicBool>,
}

/// Why background work may be able to restart, as reported to the supervisor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudReachable {
    /// The device reconnected
    Connected,
    /// The platform reported a cloud storage change
    StorageChanged,
    /// The app asked to resume pending upload verification
    PendingVerification,
}

/// Proof that Cloud Backup local writers are fenced and drained
#[derive(Debug)]
pub(crate) struct CloudBackupResetPermit {
    manager: Arc<RustCloudBackupManager>,
    previous_status: CloudBackupStatus,
    completed: bool,
}

/// Safe runtime state restored after a failed local reset
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum CloudBackupResetRecovery {
    PreviousStatusResumed,
    SafelyDisabled,
}

impl CloudBackupResetPermit {
    /// Clears Cloud Backup's local keychain state through its owner, the supervisor
    pub(crate) async fn clear_local_state(&self) -> Result<(), CloudBackupError> {
        call!(self.manager.supervisor.clear_local_state_for_reset())
            .await
            .map_err(CloudBackupError::internal)?
    }

    pub(crate) async fn complete_after_database_reset(mut self) -> Result<(), CloudBackupError> {
        call!(self.manager.supervisor.finish_local_reset(LocalResetOutcome::Wiped))
            .await
            .map_err(CloudBackupError::internal)?;
        call!(self.manager.supervisor.clear_upload_runtime_state())
            .await
            .map_err(CloudBackupError::internal)?;
        self.manager.apply_local_reset_projection();
        self.manager.local_reset_in_progress.store(false, Ordering::Release);
        self.completed = true;
        Ok(())
    }

    /// Ends a reset whose wipe failed before its database reset
    ///
    /// The owner decides what the reset left: the backup as it was when the local keys were
    /// kept, or off once their clear began, since that can't be taken back
    pub(crate) async fn resume_after_failed_reset(
        mut self,
    ) -> Result<CloudBackupResetRecovery, CloudBackupError> {
        // ending the reset publishes the policy of the body it left, and the write lane wakes
        // uploads if it runs them
        let end = call!(self.manager.supervisor.finish_local_reset(LocalResetOutcome::Abandoned))
            .await
            .map_err(CloudBackupError::internal)?;

        self.completed = true;
        Ok(self.manager.project_abandoned_reset(end, self.previous_status.clone()))
    }
}

impl Drop for CloudBackupResetPermit {
    fn drop(&mut self) {
        if self.completed || !cove_tokio::is_tokio_initialized() {
            return;
        }

        let manager = self.manager.clone();
        let status = self.previous_status.clone();
        cove_tokio::task::spawn(async move {
            manager.abandon_local_reset(status).await;
        });
    }
}

impl RustCloudBackupManager {
    /// The reducer's presentation state, which the screen is projected from
    pub(crate) fn model_state(&self) -> CloudBackupState {
        self.state.read().public_state()
    }

    async fn recover_failed_local_reset_preparation(&self, previous_status: CloudBackupStatus) {
        self.abandon_local_reset(previous_status).await;
    }

    /// Ends a local reset that won't reach its database reset, showing what the owner left
    async fn abandon_local_reset(&self, previous_status: CloudBackupStatus) {
        match call!(self.supervisor.finish_local_reset(LocalResetOutcome::Abandoned)).await {
            Ok(end) => {
                self.project_abandoned_reset(end, previous_status);
            }
            Err(error) => {
                error!("Cloud Backup could not end an abandoned reset: {error}");
                self.reconcile_runtime_status(CloudBackupStatus::Disabled);
                self.local_reset_in_progress.store(false, Ordering::Release);
            }
        }
    }

    /// Shows what an abandoned reset left, and lets the manager's callers in again
    fn project_abandoned_reset(
        &self,
        end: Option<ResetEnd>,
        previous_status: CloudBackupStatus,
    ) -> CloudBackupResetRecovery {
        let recovery = match end {
            Some(ResetEnd::Abandoned(AbandonedReset::Disabled)) => {
                self.reconcile_runtime_status(CloudBackupStatus::Disabled);
                CloudBackupResetRecovery::SafelyDisabled
            }
            // the backup is as it was; an abandon never ends in a wipe, and `None` means no reset
            // was left to end, so nothing changed underneath the previous status either
            Some(ResetEnd::Abandoned(AbandonedReset::Resumed) | ResetEnd::Wiped) | None => {
                self.reconcile_runtime_status(previous_status);
                CloudBackupResetRecovery::PreviousStatusResumed
            }
        };

        self.local_reset_in_progress.store(false, Ordering::Release);
        recovery
    }

    pub(crate) fn load_persisted_state() -> PersistedCloudBackupState {
        Database::global().cloud_backup_state.get().unwrap_or_else(|error| {
            error!("Failed to load cloud backup state: {error}");
            PersistedCloudBackupState::default()
        })
    }

    /// The status a lifecycle record shows, or `None` for a record that can't give one
    ///
    /// Only a failed load leaves an unnormalized Disabling record, and the lifecycle owner's
    /// `LoadFailed` body projects the status for it, so the record projection leaves it alone
    pub(crate) fn runtime_status_for(
        state: &PersistedCloudBackupState,
    ) -> Option<CloudBackupStatus> {
        let status = match state.status() {
            PersistedCloudBackupStatus::Disabled => CloudBackupStatus::Disabled,
            PersistedCloudBackupStatus::Disabling => match DisablingRecord::of(state) {
                Some(DisablingRecord::Committed(_)) | None => CloudBackupStatus::Disabling,
                Some(DisablingRecord::Unnormalized) => return None,
            },
            PersistedCloudBackupStatus::Enabled | PersistedCloudBackupStatus::Unverified => {
                CloudBackupStatus::Enabled
            }
            PersistedCloudBackupStatus::PasskeyMissing => CloudBackupStatus::PasskeyMissing,
            PersistedCloudBackupStatus::Corrupted => {
                CloudBackupStatus::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into())
            }
        };

        Some(status)
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
        self.state.read().status()
    }

    fn apply_local_reset_projection(&self) {
        self.clear_prompt_state();
        self.clear_enable_progress_report();
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.observe_sync_health(CloudSyncHealth::Unknown);
        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);

        // clearing is right in whatever configuration is current
        let scope = self.inventory_scope();
        self.apply_detail_outcome(scope, CloudBackupDetailOutcome::Cleared);
        self.apply_verification_state(VerificationState::Idle);
        self.apply_sync_state(SyncState::Idle);
        self.apply_recovery_state(RecoveryState::Idle);
        self.apply_cloud_only_fetch_outcome(scope, CloudBackupCloudOnlyFetchOutcome::Reset);
        self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
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

    pub(crate) fn project_exclusive_operation_handed_off(
        &self,
        from: CloudBackupExclusiveOperationClaim,
        to: CloudBackupExclusiveOperationClaim,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::ExclusiveOperationHandedOff {
            from,
            to,
        });
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

    /// Ends the recovery projection once the supervisor's runtime left it
    pub(crate) fn clear_pending_enable_recovery(&self, status: CloudBackupStatus) {
        self.apply_model_event(CloudBackupStateReducerEvent::PendingEnableRecoveryCleared(status));
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

    /// Whether the keychain holds an enable journal or staged master key material, counting an
    /// unreadable keychain as holding them, since unknown state must not look like no setup
    pub(crate) fn pending_enable_evidence_remains() -> bool {
        match CloudBackupKeychain::global().load_pending_enable_journal() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(error) => {
                error!("Failed to inspect pending Cloud Backup enable journal: {error}");
                return true;
            }
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        match cspp.master_key_promotion_status() {
            Ok(MasterKeyPromotionStatus::None) => false,
            Ok(_) => true,
            Err(error) => {
                error!("Failed to inspect staged Cloud Backup enable material: {error}");
                true
            }
        }
    }

    /// A write client for background work under the configuration the write lane runs now
    ///
    /// Call it before reading the namespace or anything else the configuration decides: a
    /// change after this read leaves the commands with an older generation, which the lane drops
    pub(crate) async fn background_writes(
        &self,
    ) -> Result<CloudBackupWriteClient, CloudBackupError> {
        let ticket = self.write_ticket().await?;
        Ok(CloudBackupWriteClient::background(self.cloud_writes.clone(), ticket.generation))
    }

    /// The write lane's current generation and its decision for an ordinary write
    pub(crate) async fn write_ticket(&self) -> Result<WriteTicket, CloudBackupError> {
        call!(self.cloud_writes.ticket()).await.map_err(CloudBackupError::internal)
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

    fn init(lifecycle: LifecycleWriter, load: LifecycleLoad) -> Arc<Self> {
        let load_error = match load {
            LifecycleLoad::Failed(error) => {
                // the legacy record stays as it was and the runtime is `LoadFailed`; relaunching
                // runs normalization again
                error!("Cloud Backup failed to load its lifecycle record: {error}");
                Some(error)
            }
            LifecycleLoad::Loaded | LifecycleLoad::Healed => None,
        };

        let manager = Arc::new_cyclic(|manager| {
            let cloud_writes = spawn_actor(CloudBackupWriteSupervisor::new(manager.clone()));
            Self {
                state: Arc::new(RwLock::new(CloudBackupStateReducer::default())),
                inventory: Arc::new(RwLock::new(InventoryOwner::default())),
                reconciler: ReconcileChannel::new(1000),
                published_screen: Arc::new(PublishedScreen::default()),
                pending_enable: PendingEnableCoordinator::new(Keychain::global().clone()),
                cloud_writes: cloud_writes.clone(),
                // the bootstrap phase hands its writer to the supervisor, the lifecycle owner
                supervisor: spawn_actor(CloudBackupSupervisor::new(
                    manager.clone(),
                    cloud_writes,
                    lifecycle,
                    load_error,
                )),
                local_reset_in_progress: Arc::new(AtomicBool::new(false)),
                pending_sync_states_unreadable: Arc::new(AtomicBool::new(false)),
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

        send!(self.supervisor.cloud_reachable(CloudReachable::Connected));
    }

    /// Restarts the background work `cause` calls for; the supervisor calls this only while
    /// its published policy runs ordinary writes
    pub(crate) fn restart_background_work(&self, cause: CloudReachable) {
        if cause == CloudReachable::PendingVerification {
            self.sync_persisted_state();
            send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
            self.start_pending_upload_verification_loop();
            return;
        }

        send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
        send!(self.supervisor.wake_pending_upload_verifier());
        send!(self.supervisor.provider_inventory_did_change());
        self.start_pending_upload_verification_loop();
        match cause {
            CloudReachable::Connected => self.resume_failed_connectivity_verification(),
            CloudReachable::StorageChanged => self.refresh_sync_health(),
            CloudReachable::PendingVerification => {}
        }
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
                send!(self.supervisor.retry_verification_after_reconnect(false))
            }
            Some(CloudBackupRetryAction::VerifyDiscoverable) => {
                send!(self.supervisor.retry_verification_after_reconnect(true));
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

    /// Ends the prompt an Enable waits on, back to off
    pub(crate) fn clear_enable_prompt(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::EnablePromptCleared);
    }

    pub(crate) fn present_passkey_choice_prompt(&self, intent: CloudBackupPasskeyChoiceIntent) {
        self.apply_model_event(CloudBackupStateReducerEvent::PasskeyChoicePromptSet(intent));
    }

    pub(crate) fn clear_passkey_choice_prompt(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::PasskeyChoicePromptCleared);
    }

    /// Consumes the Enable prompt with `choice` and returns the Enable it starts
    ///
    /// The supervisor starts that Enable in the same turn, so the prompt is never consumed by an
    /// Enable that does not start
    pub(crate) fn accept_enable_prompt(
        &self,
        choice: CloudBackupEnablePromptChoice,
    ) -> Option<CloudBackupAcceptedEnablePrompt> {
        let (accepted, effects) = {
            let mut state = self.state.write();
            state.accept_enable_prompt(choice)
        };

        self.send_model_effects(effects);
        accepted
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
        self.apply_model_event(CloudBackupStateReducerEvent::VerificationStateResolved(
            verification,
        ));
    }

    pub(crate) fn apply_sync_state(&self, sync: SyncState) {
        self.apply_model_event(CloudBackupStateReducerEvent::SyncStateResolved(sync));
    }

    pub(crate) fn apply_recovery_state(&self, recovery: RecoveryState) {
        self.apply_model_event(CloudBackupStateReducerEvent::RecoveryStateResolved(recovery));
    }

    /// Resets Cloud Backup on this device after its record was corrupted
    ///
    /// The same fence as a full wipe: admission closes and the write lane drains, then the
    /// supervisor clears this device's Cloud Backup state and leaves it off. A failure before the
    /// record is replaced resumes the corrupted body, which offers the reset again
    pub(crate) async fn reset_corrupted_on_this_device(
        self: &Arc<Self>,
    ) -> Result<(), CloudBackupError> {
        let permit = self.prepare_local_reset().await?;
        let reset = call!(self.supervisor.reset_corrupted_on_this_device())
            .await
            .map_err(CloudBackupError::internal)
            .and_then(|reset| reset);

        if let Err(error) = reset {
            if let Err(resume) = permit.resume_after_failed_reset().await {
                error!("Cloud Backup could not resume after a failed reset: {resume}");
            }

            return Err(error);
        }

        permit.complete_after_database_reset().await
    }

    pub(crate) async fn prepare_local_reset(
        self: &Arc<Self>,
    ) -> Result<CloudBackupResetPermit, CloudBackupError> {
        if self.local_reset_in_progress.swap(true, Ordering::AcqRel) {
            return Err(CloudBackupError::NotAdmitted(Rejection::Resetting));
        }

        let previous_status = self.current_status();
        let effects_settled = match call!(self.supervisor.begin_local_reset()).await {
            Ok(Ok(effects_settled)) => effects_settled,
            Ok(Err(rejection)) => {
                self.local_reset_in_progress.store(false, Ordering::Release);
                return Err(CloudBackupError::NotAdmitted(rejection));
            }
            Err(error) => {
                self.recover_failed_local_reset_preparation(previous_status).await;
                return Err(CloudBackupError::internal(error));
            }
        };

        // the supervisor answers once an effect that was in flight reports; waiting here, not in
        // its handler, keeps its mailbox free to receive that completion
        if effects_settled.await.is_err() {
            self.recover_failed_local_reset_preparation(previous_status).await;
            return Err(CloudBackupError::Internal(
                "cloud backup reset stopped before its effects reported".into(),
            ));
        }

        // the supervisor published the resetting policy before it answered, so the lane already
        // pauses every queued command; this waits out the write that was in flight
        let idle = match call!(self.cloud_writes.when_idle()).await {
            Ok(idle) => idle,
            Err(error) => {
                self.recover_failed_local_reset_preparation(previous_status).await;
                return Err(CloudBackupError::internal(error));
            }
        };
        if idle.await.is_err() {
            self.recover_failed_local_reset_preparation(previous_status).await;
            return Err(CloudBackupError::Internal(
                "cloud backup write lane stopped before going idle".into(),
            ));
        }

        Ok(CloudBackupResetPermit { manager: self.clone(), previous_status, completed: false })
    }

    /// Hands keychain writes a worker prepared to the owner, which applies them in its handler
    /// while their authority still owns the keychain
    /// Projects a committed lifecycle write into runtime status and flags
    pub(crate) fn reconcile_committed<T>(
        &self,
        context: &str,
        committed: Result<owner::lifecycle::Committed<T>, crate::database::Error>,
    ) -> Result<T, CloudBackupError> {
        let committed =
            committed.map_err(|source| CloudBackupError::internal_context(context, source))?;
        self.reconcile_persisted_state(&committed.state);
        Ok(committed.outcome)
    }

    pub(crate) fn dismiss_verification_prompt_impl(
        &self,
        lifecycle: &LifecycleWriter,
    ) -> Result<(), CloudBackupError> {
        let dismissed_at = cove_util::time::unix_timestamp_secs_or_zero();
        self.reconcile_committed(
            "persist cloud backup prompt dismissal",
            lifecycle.dismiss_verification_request(dismissed_at),
        )?;

        Ok(())
    }

    fn current_namespace_id(&self) -> Result<String, CloudBackupError> {
        CloudBackupKeychain::global()
            .namespace_id()
            .map_err(|error| CloudBackupError::internal_context("read namespace_id", error))?
            .ok_or_else(|| CloudBackupError::Internal("namespace_id not found in keychain".into()))
    }

    #[cfg(test)]
    pub(crate) fn clear_pending_enable_session(&self) {
        send!(self.supervisor.clear_pending_enable_session());
    }

    pub(crate) fn replace_pending_verification_completion(
        &self,
        lifecycle: &LifecycleWriter,
        completion: PendingVerificationCompletion,
    ) -> Result<(), CloudBackupError> {
        self.replace_pending_verification_completion_for_source(
            lifecycle,
            completion,
            self.current_verification_source(),
        )
    }

    pub(crate) fn replace_pending_verification_completion_for_source(
        &self,
        lifecycle: &LifecycleWriter,
        completion: PendingVerificationCompletion,
        source: CloudBackupVerificationSource,
    ) -> Result<(), CloudBackupError> {
        let replaced = self.reconcile_committed(
            "persist pending verification completion",
            lifecycle.replace_pending_verification_completion(completion.clone()),
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

        self.reconcile_pending_upload_verification_for_source(
            PendingUploadVerificationState::Confirming,
            source,
        );

        Ok(())
    }

    pub(crate) fn pending_verification_completion(&self) -> Option<PendingVerificationCompletion> {
        Self::load_persisted_state().pending_verification_completion().cloned()
    }

    /// A manual verification is starting, so the lifecycle owner drops any pending upload
    /// confirmation
    pub(crate) fn clear_pending_verification_completion(&self) {
        send!(self.supervisor.discard_pending_verification_completion());
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
pub(crate) use manager_test_support::{
    RequestedDriveAccountSwitchStep, set_aside_reconcile_messages, take_settled_off_count,
};
#[cfg(test)]
mod manager_test_support {
    /// What the inventory owner holds, read in one place for test assertions
    #[derive(Debug, Clone)]
    pub(crate) struct TestInventory {
        pub(crate) detail: CloudBackupDetailState,
        pub(crate) restore_all: CloudBackupRestoreAllState,
    }

    /// A Google Drive account switch step Rust asked the platform to take, as tests assert it
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum RequestedDriveAccountSwitchStep {
        SelectAccount(u64),
        ReportState,
        Commit(u64),
        Finalize(u64),
        Rollback(u64),
    }

    use super::*;
    use crate::manager::deferred_sender::SingleOrMany;

    /// Messages a test read took off the reconcile queue for its own kind, kept for the read
    /// that takes the rest
    ///
    /// The harness's screen read drains the queue for screen deltas; the platform requests it
    /// passes over wait here for the test that plays the platform. Each harness case and each
    /// nextest test runs in its own process, so no other test shares them
    static SET_ASIDE_RECONCILE_MESSAGES: Mutex<Vec<CloudBackupReconcileMessage>> =
        Mutex::new(Vec::new());

    /// Keeps `messages` another test read drained but doesn't handle
    pub(crate) fn set_aside_reconcile_messages(
        messages: impl IntoIterator<Item = CloudBackupReconcileMessage>,
    ) {
        SET_ASIDE_RECONCILE_MESSAGES.lock().extend(messages);
    }

    /// Takes the times Cloud Backup settled off from the messages a harness boundary set aside,
    /// keeping the rest for the reads that handle them
    ///
    /// The boundary's screen read drains the queue, so this reads what the last boundary saw
    pub(crate) fn take_settled_off_count() -> usize {
        let mut set_aside = SET_ASIDE_RECONCILE_MESSAGES.lock();
        let before = set_aside.len();
        set_aside.retain(|message| !matches!(message, CloudBackupReconcileMessage::SettledOff));

        before - set_aside.len()
    }

    impl RustCloudBackupManager {
        /// Drains the reconcile queue and returns the switch steps it asked for, in order
        pub(crate) fn drained_drive_account_switch_steps(
            &self,
        ) -> Vec<RequestedDriveAccountSwitchStep> {
            use CloudBackupReconcileMessage as M;
            use RequestedDriveAccountSwitchStep as Step;

            let set_aside = std::mem::take(&mut *SET_ASIDE_RECONCILE_MESSAGES.lock());
            let receiver = self.reconciler.receiver();
            let queued = receiver.try_iter().flat_map(|field| match field {
                SingleOrMany::Single(message) => vec![message],
                SingleOrMany::Many(messages) => messages,
            });

            set_aside
                .into_iter()
                .chain(queued)
                .filter_map(|message| match message {
                    M::DriveAccountSwitchSelectionRequired(id) => Some(Step::SelectAccount(id)),
                    M::DriveAccountSwitchStateRequired => Some(Step::ReportState),
                    M::DriveAccountSwitchCommitRequired(id) => Some(Step::Commit(id)),
                    M::DriveAccountSwitchFinalizeRequired(id) => Some(Step::Finalize(id)),
                    M::DriveAccountSwitchRollbackRequired(id) => Some(Step::Rollback(id)),
                    _ => None,
                })
                .collect()
        }

        /// Hands `result` to the switch's owner and waits until it has handled it
        pub(crate) async fn drive_account_switch_platform_result_handled(
            &self,
            result: DriveAccountSwitchPlatformResult,
        ) {
            call!(self.supervisor.drive_account_switch_platform_result(result))
                .await
                .expect("the supervisor handles the platform result");
        }

        /// Seeds `state` directly and projects it, bypassing the lifecycle owner
        pub(crate) fn persist_cloud_backup_state(
            &self,
            state: &PersistedCloudBackupState,
            context: impl std::fmt::Display,
        ) -> Result<(), CloudBackupError> {
            Database::global()
                .cloud_backup_state
                .set(state)
                .map_err(|source| CloudBackupError::internal_context(context, source))?;
            self.reconcile_persisted_state(state);
            Ok(())
        }

        /// The inventory as the screen reads it, for tests that assert on detail or Restore All
        pub(crate) fn test_inventory(&self) -> TestInventory {
            // the detail's read guard ends here, before the projection takes the lock again: a
            // writer waiting between two reads of one lock deadlocks them
            let detail = self.inventory.read().detail_state().clone();
            TestInventory { detail, restore_all: self.projected_restore_all_state() }
        }

        pub(crate) fn model_snapshot(&self) -> test_support::CloudBackupModelSnapshot {
            let mut snapshot = self.state.read().snapshot();
            snapshot.detail =
                self.inventory.read().loaded_detail().map(|loaded| loaded.detail.clone());

            snapshot
        }

        pub(crate) fn debug_reset_cloud_backup_state(&self) {
            if let Err(error) = CloudBackupKeychain::global().clear_local_state_for_test() {
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
            self.apply_detail_outcome(self.inventory_scope(), CloudBackupDetailOutcome::Cleared);
            self.apply_verification_state(VerificationState::Idle);
            self.apply_sync_state(SyncState::Idle);
            self.apply_recovery_state(RecoveryState::Idle);
            self.apply_cloud_only_fetch_outcome(
                self.inventory_scope(),
                CloudBackupCloudOnlyFetchOutcome::Reset,
            );
            self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
            self.reconcile_runtime_status(CloudBackupStatus::Disabled);
            self.refresh_sync_health();
            send!(self.supervisor.clear_upload_runtime_state());
            send!(self.supervisor.reset_runtime_for_test());
        }
    }
}

#[uniffi::export]
impl RustCloudBackupManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        CLOUD_BACKUP_MANAGER.clone()
    }

    /// The screen the platform starts from, with its revision
    ///
    /// Read it before `listen_for_updates`, then advance it only with
    /// [`CloudBackupPublishedScreen::applying`]: every later change arrives as a `Screen` delta,
    /// and the deltas published before this read, still queued for the listener, are at or
    /// before its revision, so they are skipped instead of replayed over it
    pub fn state(&self) -> CloudBackupPublishedScreen {
        self.published_snapshot()
    }

    /// Number of wallets in the cloud backup
    pub fn backup_wallet_count(&self) -> Option<u32> {
        let db = Database::global();
        let current = Self::load_persisted_state();

        match current.wallet_count() {
            Some(count) => Some(count),
            None if current.is_configured() => match CloudBackupStore::new(&db).wallet_count() {
                // the lifecycle owner records the derived count; the caller gets it now
                Ok(count) => {
                    send!(self.supervisor.record_cloud_wallet_count(count));
                    Some(count)
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
        if !self.has_in_flight_operation() {
            if let Some(status) = Self::runtime_status_for(&db_state) {
                self.reconcile_runtime_status(status);
            }
            self.reconcile_persisted_restore_all(&db_state);
        }

        self.refresh_persisted_flags();
        if !self.has_in_flight_operation() {
            self.refresh_pending_upload_verification_state();
        }

        // the runtime owner has the last word for bodies the record alone can't show
        send!(self.supervisor.project_runtime());
    }

    /// Start silent supplemental inventory checks for this app process
    pub fn start_background_inventory_discovery(&self) {
        send!(self.supervisor.start_background_inventory_discovery());
    }

    pub fn cloud_storage_did_change(&self) {
        send!(self.supervisor.cloud_reachable(CloudReachable::StorageChanged));
    }

    /// Hands Rust's switch owner what Android's half of a Google Drive account switch did
    ///
    /// Android sends one result for each step a `DriveAccountSwitch*Required` message asked for,
    /// and its durable state at launch. The switch owner decides the next step, and any recovery
    /// the user sees, from it
    pub fn report_drive_account_switch_platform_result(
        &self,
        result: DriveAccountSwitchPlatformResult,
    ) {
        send!(self.supervisor.drive_account_switch_platform_result(result));
    }

    /// Check if cloud backup is enabled, used as nav guard
    pub fn is_cloud_backup_enabled(&self) -> bool {
        Self::load_persisted_state().is_configured()
    }

    /// Reports whether onboarding may recover a lost enable-completion event from durable state
    pub fn onboarding_enable_completion_readiness(
        &self,
    ) -> CloudBackupOnboardingCompletionReadiness {
        if Self::pending_enable_evidence_remains() {
            return CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery;
        }

        match self.model_state().lifecycle {
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

    pub fn has_pending_cloud_upload_verification(&self) -> bool {
        if Self::load_persisted_state().pending_verification_completion().is_some() {
            return true;
        }

        Database::global().cloud_blob_sync_states.list().ok().is_some_and(|states| {
            states.into_iter().any(|state| state.is_uploaded_pending_confirmation())
        })
    }

    pub fn resume_pending_cloud_upload_verification(&self) {
        send!(self.supervisor.cloud_reachable(CloudReachable::PendingVerification));
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

        let current_namespace = match CloudBackupKeychain::global().namespace_id() {
            Ok(current_namespace) => current_namespace,

            // an unreadable namespace proves nothing about the marker, so keep it for the next sync
            Err(error) => {
                warn!("Keeping Restore All marker, namespace_id is unreadable: {error}");
                return;
            }
        };

        if current_namespace.as_deref() == Some(marker.namespace_id.as_str()) {
            self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllRetryRequired);
            return;
        }

        send!(self.supervisor.clear_stale_restore_all_marker(marker.namespace_id.clone()));
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllReset);
    }

    /// Only a committed Turn Off resumes on its own; it goes forward until Disabled
    fn resume_persisted_disable_if_needed(&self) {
        send!(self.supervisor.resume_disable_automatically(AutomaticResume::Launch));
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
        async_test_lock, complete_loaded_outcome, configure_enabled_cloud_backup,
        ensure_cloud_backup_test_tokio_runtime, listed_detail,
        persisted_enabled_cloud_backup_state, supervisor_checkpoint, test_globals, test_lock,
    };
    use super::owner::operation::test_support::operation_id;
    use super::*;
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
        PersistedPasskeyState, PersistedVerificationRequirement,
    };
    use act_zero::call;
    use cove_device::cloud_storage::CloudStorageError;
    use cove_device::passkey::{PasskeyError, PasskeyFailureReason, PasskeyOperation};

    fn init_manager() -> Arc<RustCloudBackupManager> {
        ensure_cloud_backup_test_tokio_runtime();
        test_globals().reset();
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        crate::manager::cloud_backup_manager::initialize_cloud_backup_manager()
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
            heal_notice: None,
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
            Some(CloudBackupStatus::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into()))
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
                wallet_issues: Default::default(),
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
                wallet_issues: Default::default(),
                detail: None,
            },
            "namespace".into(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        );

        let error = manager
            .replace_pending_verification_completion(
                &crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer(),
                completion,
            )
            .unwrap_err();

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
            reason: PersistedVerificationRequirement::Unknown,
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

        // restore starts only while backup is off; the stale write is refused before it writes,
        // which its Cancelled error shows
        let persisted_state = PersistedCloudBackupState::Disabled;
        db.cloud_backup_state.set(&persisted_state).unwrap();
        manager.reconcile_runtime_status(CloudBackupStatus::Disabled);

        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);

        let error = run_on_cloud_backup_runtime(async move {
            stale_operation.record_restore_without_backup().await.unwrap_err()
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(db.cloud_backup_state.get().unwrap(), persisted_state);
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Restoring);

        run_on_cloud_backup_runtime(async move {
            current_operation.record_restore_without_backup().await.unwrap()
        });

        assert_eq!(db.cloud_backup_state.get().unwrap(), PersistedCloudBackupState::Disabled);
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
            operation_id(1),
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
            operation_id(1),
        ));

        assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Enabling(_)));
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
            operation_id(1),
        ));

        assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Restoring(_)));
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
            operation_id(1),
        ));
        manager.sync_persisted_state();

        assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Enabling(_)));
        assert_eq!(manager.state.read().status(), CloudBackupStatus::Enabling);
    }

    #[test]
    fn sync_persisted_restore_all_projects_retry_without_starting_work() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        let namespace_id = manager.current_namespace_id().unwrap();
        crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer()
            .persist_restore_all_marker(namespace_id)
            .unwrap();
        let authenticate_count = globals.passkey.authenticate_count();

        manager.sync_persisted_state();
        manager.apply_detail_outcome(
            manager.inventory_scope(),
            CloudBackupDetailOutcome::refreshed(listed_detail(&["remaining-record"])),
        );
        manager.apply_cloud_only_fetch_outcome(
            manager.inventory_scope(),
            complete_loaded_outcome(
                &manager,
                vec![CloudBackupWalletItem {
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
                }],
            ),
        );

        let configured = manager.test_inventory();

        assert_eq!(
            configured.restore_all,
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );
        assert_eq!(manager.projected_exclusive_operation(), None);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    }

    #[test]
    fn sync_persisted_restore_all_keeps_marker_when_namespace_is_unreadable() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer()
            .persist_restore_all_marker("inactive-namespace".into())
            .unwrap();

        globals.keychain.fail_reads_for(keychain::CSPP_NAMESPACE_ID_KEY);
        manager.sync_persisted_state();
        globals.keychain.clear_read_failures();

        assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_persisted_restore_all_clears_marker_for_inactive_namespace() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer()
            .persist_restore_all_marker("inactive-namespace".into())
            .unwrap();

        manager.sync_persisted_state();
        supervisor_checkpoint(&manager).await;

        assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
        assert_eq!(manager.projected_exclusive_operation(), None);
        assert_eq!(globals.passkey.authenticate_count(), 0);
    }
}
