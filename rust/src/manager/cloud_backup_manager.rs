pub(crate) mod actors;
mod blob_state;
mod catastrophic_recovery;
mod cloud_inventory;
mod cspp_exports;
mod detail;
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
use cove_device::cloud_storage::{CloudStorageClient, CloudSyncHealth};
use cove_tokio::task::spawn_actor;
use cove_util::ResultExt as _;
use flume::{Receiver, Sender};
use parking_lot::RwLock;
use tracing::{error, info, warn};

use cove_types::network::Network;

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobFailureIssue, PersistedCloudBackupState, PersistedCloudBackupStatus,
};
use crate::wallet::metadata::{WalletId, WalletMode as LocalWalletMode, WalletType};

pub(crate) use self::actors::CloudBackupRestoreEvent;
use self::actors::{
    CloudBackupOperation, CloudBackupSupervisor, CloudBackupUploadedWallet,
    CloudBackupWalletCountRefresh, CloudBackupWriteBlocker, CloudBackupWriteClient,
    CloudBackupWriteCompletion, CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor,
};
pub(crate) use self::detail::{
    CloudBackupCloudOnlyFetchOutcome, CloudBackupCloudOnlyOperationWarning,
    CloudBackupCloudOnlyWalletOutcome, CloudBackupDetailOutcome, CloudBackupEnableOutcome,
    CloudBackupOtherBackupsOutcome, CloudBackupRecoveryOutcome, CloudBackupRestoreOutcome,
    CloudBackupSyncOutcome, CloudBackupVerificationOutcome,
};
pub use self::detail::{
    CloudBackupVerificationPresentation, CloudBackupVerificationReason,
    CloudBackupVerificationSource, CloudOnlyOperation, CloudOnlyState,
    PendingUploadVerificationState, RecoveryAction, RecoveryState, SyncState, VerificationState,
};
pub(crate) use self::error::{
    BlockingCloudStep, CloudBackupError, CloudStorageIssue, blocking_cloud_error,
    is_connectivity_related_issue, offline_error_for_step,
};
pub(crate) use self::keychain::CloudBackupKeychain;
use self::model::{
    CloudBackupAcceptedEnablePrompt, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim, CloudBackupStateReducer, CloudBackupStateReducerEvent,
};
pub use self::model::{CloudBackupLifecycle, CloudBackupRestoreFlow};
pub(crate) use self::ops::{
    CloudBackupDisablePreparation, CloudBackupEnablePasskeyPreparation,
    CloudBackupEnablePasskeyRegistration, CloudBackupEnablePreparation,
    CloudBackupEnableRecoveryCompletion, CloudBackupEnableRecoveryPreparation,
    CloudBackupKeepEnabledPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupPreparedCloudWalletDelete, CloudBackupReadyEnableUpload,
    CloudBackupRegisteredEnablePasskey, CloudBackupReuploadedWallets,
    CloudBackupSavedPasskeyConfirmation, CloudBackupUploadedEnableBackup,
    EnablePasskeyRegistrationFlow,
};
pub(crate) use self::pending_enable::PendingEnableSession;
#[cfg(test)]
pub(crate) use self::pending_enable::PendingEnableSessionMaterial;
pub(crate) use self::pending_verification::{
    PendingVerificationCompletion, PendingVerificationUpload,
};
use self::reconcile::CloudBackupReconcileMessage;
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
pub(super) const CLOUD_BACKUP_IO_CONCURRENCY: usize = 4;
type Message = CloudBackupReconcileMessage;

pub(crate) fn current_timestamp() -> u64 {
    jiff::Timestamp::now().as_second().try_into().unwrap_or(0)
}

pub static CLOUD_BACKUP_MANAGER: LazyLock<Arc<RustCloudBackupManager>> =
    LazyLock::new(RustCloudBackupManager::init);

/// Runtime cloud backup status persisted or projected for compatibility paths
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum CloudBackupStatus {
    Disabled,
    Disabling,
    Enabling,
    Restoring,
    Enabled,
    PasskeyMissing,
    UnsupportedPasskeyProvider,
    Error(String),
}

/// Shared settings row state projected for Swift and Kotlin presentation
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupSettingsRowStatus {
    Disabled,
    Disabling,
    SettingUp,
    Restoring,
    Active,
    PasskeyMissing,
    PasskeyProviderUnsupported,
    Unverified,
    Confirming,
    VerificationRecommended,
    CheckingSync,
    Syncing,
    NoFiles,
    DriveUnavailable,
    Error(String),
    AuthorizationRequired(String),
}

/// Whether saved passkey confirmation was user-triggered or flow-triggered
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SavedPasskeyConfirmationMode {
    Automatic,
    Manual,
}

/// Context carried through enable so prompts and verification attribution stay stable
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupEnableContext {
    pub saved_passkey_confirmation: SavedPasskeyConfirmationMode,
    pub verification_source: CloudBackupVerificationSource,
}

impl CloudBackupEnableContext {
    pub(crate) fn settings_manual() -> Self {
        Self {
            saved_passkey_confirmation: SavedPasskeyConfirmationMode::Manual,
            verification_source: CloudBackupVerificationSource::Settings,
        }
    }
}

/// Internal enable status before projection into the public lifecycle
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum CloudBackupEnableState {
    Idle,
    CreatingPasskey,
    WaitingForPasskeyAvailability,
    AwaitingSavedPasskeyConfirmation(SavedPasskeyConfirmationMode),
    ConfirmingSavedPasskey,
    UploadingBackup,
}

/// Prompt intent for choosing between an existing passkey and a new one
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupPasskeyChoiceIntent {
    Enable(CloudBackupEnableContext, Option<CloudBackupPasskeyHint>),
    RepairPasskey,
}

/// User selection for the currently visible enable prompt
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupEnablePromptChoice {
    UseExisting,
    CreateNew,
}

/// Root-level prompt the UI should show for the current cloud backup state
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRootPrompt {
    None,
    ExistingBackupFound(CloudBackupEnableContext, Option<CloudBackupPasskeyHint>),
    PasskeyChoice(CloudBackupPasskeyChoiceIntent),
    MissingPasskeyReminder,
    Verification,
}

/// User intent routed from Swift or Kotlin into the Rust cloud backup manager
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupManagerAction {
    EnableCloudBackup(CloudBackupEnableContext),
    EnableCloudBackupForceNew(CloudBackupEnableContext),
    EnableCloudBackupNoDiscovery(CloudBackupEnableContext),
    ConfirmSavedPasskey,
    DiscardPendingEnableCloudBackup,
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
    DeleteCloudWallet(RecordId),
    RecoverOtherBackups,
    DeleteOtherBackups,
    DisableCloudBackup,
    KeepCloudBackupEnabled,
    RefreshDetail,
    EnterDetail,
    PromptEnablePasskeyChoice(CloudBackupEnableContext),
    AcceptEnablePrompt(CloudBackupEnablePromptChoice),
}

/// Result of a disable attempt after the supervisor resolves remote and local work
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupDisableOutcome {
    Started,
    ReturnedToIdle,
    Failed { message: String, can_keep_enabled: bool },
}

/// Restore summary shown after cloud backup onboarding restore completes
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRestoreReport {
    pub wallets_restored: u32,
    pub wallets_failed: u32,
    pub failed_wallet_errors: Vec<String>,
    pub labels_failed_wallet_names: Vec<String>,
    pub labels_failed_errors: Vec<String>,
}

/// Completed and total counts for long-running cloud backup work
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupProgress {
    pub completed: u32,
    pub total: u32,
}

/// Cloud backup record identifier exposed through UniFFI as an opaque string
#[derive(Debug, Clone, PartialEq, Eq, Hash, derive_more::From, derive_more::Into)]
pub struct RecordId(String);

uniffi::custom_newtype!(RecordId, String);

/// Per-wallet cloud backup sync state shown in backup detail
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupWalletStatus {
    Dirty,
    Uploading,
    UploadedPendingConfirmation,
    Confirmed,
    Failed,
    DeletedFromDevice,
    UnsupportedVersion,
    RemoteStateUnknown,
}

/// Wallet row in cloud backup detail, combining local wallet metadata and sync state
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupWalletItem {
    pub name: String,
    pub network: Option<Network>,
    pub wallet_mode: Option<LocalWalletMode>,
    pub wallet_type: Option<WalletType>,
    pub fingerprint: Option<String>,
    pub label_count: Option<u32>,
    pub backup_updated_at: Option<u64>,
    pub sync_status: CloudBackupWalletStatus,
    /// Deterministic cloud record ID for the wallet backup represented by this item
    pub record_id: String,
}

/// Remote detail fetch result that keeps access errors distinguishable from failed detail state
#[derive(Debug)]
pub enum CloudBackupDetailResult {
    Success(CloudBackupDetail),
    AccessError(CloudBackupError),
}

/// Backup detail grouped by local wallet sync status and remote-only inventory
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDetail {
    pub last_sync: Option<u64>,
    pub up_to_date: Vec<CloudBackupWalletItem>,
    pub needs_sync: Vec<CloudBackupWalletItem>,
    /// Number of wallets in the cloud that aren't on this device
    pub cloud_only_count: u32,
    pub other_backups: CloudBackupOtherBackupsState,
}

/// Summary state for backup namespaces that do not match the active device
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOtherBackupsState {
    Loaded { summary: CloudBackupOtherBackupsSummary },
    LoadFailed { error: String },
}

/// Aggregate count of recoverable backup data in other namespaces
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct CloudBackupOtherBackupsSummary {
    pub namespace_count: u32,
    pub wallet_count: u32,
    pub passkey_hints: Vec<CloudBackupPasskeyHint>,
}

/// User-facing passkey hint that avoids exposing credential bytes
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupPasskeyHint {
    pub provider_name: Option<String>,
    pub name_suffix: String,
    pub registered_at: u64,
}

impl CloudBackupPasskeyHint {
    pub(crate) fn from_provider_hint(hint: &cove_cspp::backup_data::PasskeyProviderHint) -> Self {
        Self {
            provider_name: hint.known_provider().map(|provider| provider.display_name().into()),
            name_suffix: hint.name_suffix.clone(),
            registered_at: hint.registered_at,
        }
    }
}

/// Operation state for recovering or deleting other backup namespaces
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OtherBackupsOperation {
    Idle,
    Recovering,
    Recovered { wallets_restored: u32, wallets_failed: u32, failed_wallet_errors: Vec<String> },
    Deleting,
    Deleted,
    Failed { error: String },
}

/// Outcome of deep verification before projection into UI state
#[derive(Debug, Clone, uniffi::Enum)]
pub enum DeepVerificationResult {
    Verified(DeepVerificationReport),
    AwaitingUploadConfirmation(DeepVerificationReport),
    PasskeyConfirmed(Option<CloudBackupDetail>),
    PasskeyMissing(Option<CloudBackupDetail>),
    UserCancelled(Option<CloudBackupDetail>),
    NotEnabled,
    Failed(DeepVerificationFailure),
}

/// Counts and repairs observed during a deep verification pass
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeepVerificationReport {
    /// Cloud master key PRF wrapping was repaired
    pub master_key_wrapper_repaired: bool,
    /// Local keychain was repaired from verified cloud master key
    pub local_master_key_repaired: bool,
    /// credential_id was recovered via discoverable auth
    pub credential_recovered: bool,
    pub wallets_verified: u32,
    pub wallets_failed: u32,
    /// Wallet backups with unsupported version (newer format, skipped)
    pub wallets_unsupported: u32,
    /// May be None if wallet list was missing but master key verified
    pub detail: Option<CloudBackupDetail>,
}

/// Persisted verification metadata projected into prompts and detail state
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupVerificationMetadata {
    NotConfigured,
    ConfiguredNeverVerified,
    Verified(u64),
    NeedsVerification,
}

/// Trust failure that tells the UI which recovery path is valid
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum DeepVerificationFailure {
    /// Transient iCloud/network/passkey error — safe to retry
    Retry {
        message: String,
        detail: Option<CloudBackupDetail>,
        retry_context: Option<CloudBackupRetryContext>,
    },
    /// Manifest missing, master key verified intact — recreate from local wallets
    RecreateManifest { message: String, warning: String, detail: Option<CloudBackupDetail> },
    /// No verified cloud or local master key available — full re-enable needed
    ReinitializeBackup { message: String, warning: String, detail: Option<CloudBackupDetail> },
    /// Backup uses a newer format — do not overwrite
    UnsupportedVersion { message: String, detail: Option<CloudBackupDetail> },
}

/// Retry issue category for a user-visible verification retry
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRetryIssue {
    Connectivity,
}

/// Retry action the UI should dispatch for a retryable verification failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRetryAction {
    Verify,
    VerifyDiscoverable,
}

/// Retry instruction attached to a retryable deep verification failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupRetryContext {
    pub issue: CloudBackupRetryIssue,
    pub action: CloudBackupRetryAction,
}

/// Top-level state snapshot exposed to platform managers
#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudBackupState {
    pub lifecycle: CloudBackupLifecycle,
    pub settings_row_status: CloudBackupSettingsRowStatus,
}

impl Default for CloudBackupState {
    fn default() -> Self {
        Self {
            lifecycle: CloudBackupLifecycle::Disabled,
            settings_row_status: CloudBackupSettingsRowStatus::Disabled,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    #[derive(Debug, Clone)]
    pub(crate) struct CloudBackupModelSnapshot {
        pub(crate) root_prompt: CloudBackupRootPrompt,
        pub(crate) status: CloudBackupStatus,
        pub(crate) sync_health: CloudSyncHealth,
        pub(crate) progress: Option<CloudBackupProgress>,
        pub(crate) restore_progress: Option<CloudBackupRestoreFlow>,
        pub(crate) enable_state: CloudBackupEnableState,
        pub(crate) pending_upload_verification: PendingUploadVerificationState,
        pub(crate) verification_presentation: CloudBackupVerificationPresentation,
        pub(crate) detail: Option<CloudBackupDetail>,
        pub(crate) verification: VerificationState,
    }

    impl Default for CloudBackupModelSnapshot {
        fn default() -> Self {
            Self {
                root_prompt: CloudBackupRootPrompt::None,
                status: CloudBackupStatus::Disabled,
                sync_health: CloudSyncHealth::Unknown,
                progress: None,
                restore_progress: None,
                enable_state: CloudBackupEnableState::Idle,
                pending_upload_verification: PendingUploadVerificationState::Idle,
                verification_presentation: CloudBackupVerificationPresentation::Hidden {
                    source: None,
                },
                detail: None,
                verification: VerificationState::Idle,
            }
        }
    }
}

impl From<&PersistedCloudBackupState> for CloudBackupVerificationMetadata {
    fn from(db_state: &PersistedCloudBackupState) -> Self {
        if db_state.is_unverified() {
            return Self::NeedsVerification;
        }

        if !db_state.is_configured() {
            return Self::NotConfigured;
        }

        match db_state.last_verified_at() {
            Some(last_verified_at) => Self::Verified(last_verified_at),
            None => Self::ConfiguredNeverVerified,
        }
    }
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

impl DeepVerificationFailure {
    pub(crate) fn retry(
        message: impl Into<String>,
        detail: Option<CloudBackupDetail>,
        retry_context: Option<CloudBackupRetryContext>,
    ) -> Self {
        Self::Retry { message: message.into(), detail, retry_context }
    }

    pub(crate) fn detail(&self) -> Option<&CloudBackupDetail> {
        match self {
            Self::Retry { detail, .. }
            | Self::RecreateManifest { detail, .. }
            | Self::ReinitializeBackup { detail, .. }
            | Self::UnsupportedVersion { detail, .. } => detail.as_ref(),
        }
    }

    pub(crate) fn is_connectivity_retry(&self) -> bool {
        matches!(
            self,
            Self::Retry {
                retry_context: Some(CloudBackupRetryContext {
                    issue: CloudBackupRetryIssue::Connectivity,
                    ..
                }),
                ..
            }
        )
    }

    pub(crate) fn connectivity_retry_action(&self) -> Option<CloudBackupRetryAction> {
        match self {
            Self::Retry {
                retry_context:
                    Some(CloudBackupRetryContext { issue: CloudBackupRetryIssue::Connectivity, action }),
                ..
            } => Some(*action),
            _ => None,
        }
    }
}

impl CloudBackupRetryContext {
    pub(crate) fn connectivity(action: CloudBackupRetryAction) -> Self {
        Self { issue: CloudBackupRetryIssue::Connectivity, action }
    }
}

impl CloudBackupDetailResult {
    pub(crate) fn is_connectivity_access_error(&self) -> bool {
        matches!(self, Self::AccessError(error) if is_connectivity_related_issue(error))
    }
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustCloudBackupManager {
    pub state: Arc<RwLock<CloudBackupStateReducer>>,
    pub reconciler: Sender<Message>,
    pub reconcile_receiver: Arc<Receiver<Message>>,
    cloud_only_detail_snapshot: Arc<RwLock<Option<CloudBackupDetail>>>,
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
            other => CloudBackupStatus::Error(other.to_string()),
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

        // keep this DB read so restarts and direct disable recovery preserve the write fence
        Self::load_persisted_state().is_disabling()
    }

    pub(crate) fn ensure_cloud_backup_writes_allowed(&self) -> Result<(), CloudBackupError> {
        if self.cloud_backup_writes_blocked() {
            return Err(CloudBackupError::Deferred(
                "cloud backup writes are paused while disabling cloud backup".into(),
            ));
        }

        Ok(())
    }

    async fn await_cloud_backup_write<T>(
        receiver: CloudBackupWriteResultReceiver<T>,
    ) -> Result<T, CloudBackupError> {
        receiver
            .await
            .map_err_prefix("wait for cloud backup write supervisor", CloudBackupError::Internal)?
            .into_result()
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
        let receiver =
            call!(self.cloud_writes.upload_wallet_backup_with_completion(
                cloud, namespace, record_id, data, completion
            ))
            .await
            .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        Self::await_cloud_backup_write(receiver).await
    }

    pub(crate) async fn complete_cloud_wallet_upload_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> Result<(), CloudBackupError> {
        let receiver = call!(self.cloud_writes.complete_uploaded_wallet_batch(
            cloud,
            namespace_id,
            uploaded_wallets,
            count_refresh
        ))
        .await
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        Self::await_cloud_backup_write(receiver).await
    }

    pub(crate) fn cloud_blob_failure_issue(
        issue: CloudStorageIssue,
    ) -> Option<CloudBlobFailureIssue> {
        match issue {
            CloudStorageIssue::AuthorizationRequired => {
                Some(CloudBlobFailureIssue::AuthorizationRequired)
            }
            CloudStorageIssue::Offline => Some(CloudBlobFailureIssue::Offline),
            CloudStorageIssue::Unavailable => Some(CloudBlobFailureIssue::Unavailable),
            CloudStorageIssue::NotFound => Some(CloudBlobFailureIssue::NotFound),
            CloudStorageIssue::QuotaExceeded => Some(CloudBlobFailureIssue::QuotaExceeded),
            CloudStorageIssue::Other => None,
        }
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
        let (sender, receiver) = flume::bounded(1000);

        let manager = Arc::new_cyclic(|manager| {
            let cloud_writes = spawn_actor(CloudBackupWriteSupervisor::new(manager.clone()));
            Self {
                state: Arc::new(RwLock::new(CloudBackupStateReducer::default())),
                reconciler: sender,
                reconcile_receiver: Arc::new(receiver),
                cloud_only_detail_snapshot: Arc::new(RwLock::new(None)),
                cloud_writes: cloud_writes.clone(),
                supervisor: spawn_actor(CloudBackupSupervisor::new(manager.clone(), cloud_writes)),
            }
        });

        manager.sync_persisted_state();
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

    pub(crate) fn apply_enable_outcome(&self, outcome: CloudBackupEnableOutcome) {
        match outcome {
            CloudBackupEnableOutcome::ProgressCleared => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableProgressReported(None));
            }
            CloudBackupEnableOutcome::ReturnedToIdle => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::Idle,
                ));
            }
            CloudBackupEnableOutcome::CreatingPasskey => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::CreatingPasskey,
                ));
            }
            CloudBackupEnableOutcome::WaitingForPasskeyAvailability => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::WaitingForPasskeyAvailability,
                ));
            }
            CloudBackupEnableOutcome::UploadingBackup => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::UploadingBackup,
                ));
            }
            CloudBackupEnableOutcome::ConfirmingSavedPasskey => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::ConfirmingSavedPasskey,
                ));
            }
            CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(mode) => {
                self.apply_model_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                    CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(mode),
                ));
            }
        }
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

    pub(crate) fn apply_verification_outcome(&self, outcome: CloudBackupVerificationOutcome) {
        let verification = match outcome {
            CloudBackupVerificationOutcome::Idle => VerificationState::Idle,
            CloudBackupVerificationOutcome::Started => VerificationState::Verifying,
            CloudBackupVerificationOutcome::Verified(report) => VerificationState::Verified(report),
            CloudBackupVerificationOutcome::PasskeyConfirmed => VerificationState::PasskeyConfirmed,
            CloudBackupVerificationOutcome::Failed(failure) => VerificationState::Failed(failure),
            CloudBackupVerificationOutcome::Cancelled => VerificationState::Cancelled,
        };

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

    pub(crate) fn apply_sync_outcome(&self, outcome: CloudBackupSyncOutcome) {
        let sync = match outcome {
            CloudBackupSyncOutcome::Started => SyncState::Syncing,
            CloudBackupSyncOutcome::Completed => SyncState::Idle,
            CloudBackupSyncOutcome::Failed(error) => SyncState::Failed(error),
        };

        self.apply_model_event(CloudBackupStateReducerEvent::SyncStateResolved(sync));
    }

    pub(crate) fn apply_recovery_outcome(&self, outcome: CloudBackupRecoveryOutcome) {
        let recovery = match outcome {
            CloudBackupRecoveryOutcome::Idle => RecoveryState::Idle,
            CloudBackupRecoveryOutcome::Started(action) => RecoveryState::Recovering(action),
            CloudBackupRecoveryOutcome::Failed { action, error } => {
                RecoveryState::Failed { action, error }
            }
        };

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
        self.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.observe_sync_health(CloudSyncHealth::Unknown);
        self.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        self.apply_detail_outcome(CloudBackupDetailOutcome::Cleared);
        self.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        self.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
        self.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Reset);
        self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
    }

    pub(crate) fn persist_cloud_backup_state(
        &self,
        state: &PersistedCloudBackupState,
        context: &str,
    ) -> Result<(), CloudBackupError> {
        Database::global()
            .cloud_backup_state
            .set(state)
            .map_err(|error| CloudBackupError::Internal(format!("{context}: {error}")))?;

        self.reconcile_runtime_status(Self::runtime_status_for(state));
        self.refresh_persisted_flags();

        Ok(())
    }

    pub(crate) fn dismiss_verification_prompt_impl(&self) -> Result<(), CloudBackupError> {
        let mut state = Self::load_persisted_state();
        let dismissed_at = crate::manager::cloud_backup_manager::current_timestamp();
        if !state.dismiss_verification_request(dismissed_at) {
            return Ok(());
        }

        self.persist_cloud_backup_state(&state, "persist cloud backup prompt dismissal")
    }

    fn current_namespace_id(&self) -> Result<String, CloudBackupError> {
        CloudBackupKeychain::global()
            .namespace_id()
            .ok_or_else(|| CloudBackupError::Internal("namespace_id not found in keychain".into()))
    }

    pub(crate) fn clear_pending_enable_session(&self) {
        send!(self.supervisor.clear_pending_enable_session());
    }

    pub(crate) fn replace_pending_verification_completion(
        &self,
        completion: PendingVerificationCompletion,
    ) {
        self.replace_pending_verification_completion_for_source(
            completion,
            self.current_verification_source(),
        );
    }

    pub(crate) fn replace_pending_verification_completion_for_source(
        &self,
        completion: PendingVerificationCompletion,
        source: CloudBackupVerificationSource,
    ) {
        if let Some(detail) = completion.report().detail.clone() {
            self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
        }

        let persisted_completion = completion.persisted();

        let mut state = Self::load_persisted_state();
        state.replace_pending_verification_completion(persisted_completion);
        if let Err(error) =
            self.persist_cloud_backup_state(&state, "persist pending verification completion")
        {
            error!("Failed to persist pending verification completion: {error}");
            return;
        }

        send!(self.supervisor.cache_pending_verification_completion(completion));
        self.reconcile_pending_upload_verification_for_source(
            PendingUploadVerificationState::Confirming,
            source,
        );
    }

    pub(crate) fn pending_verification_completion(&self) -> Option<PendingVerificationCompletion> {
        Self::load_persisted_state()
            .pending_verification_completion()
            .cloned()
            .map(PendingVerificationCompletion::from_persisted)
    }

    pub(crate) fn clear_pending_verification_completion(&self) {
        let mut state = Self::load_persisted_state();
        if !state.clear_pending_verification_completion() {
            send!(self.supervisor.clear_pending_verification_completion());
            return;
        }

        if let Err(error) =
            self.persist_cloud_backup_state(&state, "clear pending verification completion")
        {
            error!("Failed to clear pending verification completion: {error}");
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
        self.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        self.reconcile_runtime_status(Self::status_for_operation_error(error));
    }
}

#[cfg(test)]
mod manager_test_support {
    use super::*;

    impl RustCloudBackupManager {
        pub(crate) fn model_snapshot(&self) -> test_support::CloudBackupModelSnapshot {
            self.state.read().snapshot()
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
                    let _ = db.cloud_backup_state.set(&current.with_wallet_count(Some(count)));
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
        if let Some(disabling) = db_state.disabling() {
            send!(self.cloud_writes.block(CloudBackupWriteBlocker::Disabling {
                operation_id: disabling.disable_generation,
            }));
        }
        if !self.has_in_flight_operation() {
            self.reconcile_runtime_status(Self::runtime_status_for(&db_state));
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
        self.start_pending_upload_verification_loop();
        self.refresh_sync_health();
    }

    /// Check if cloud backup is enabled, used as nav guard
    pub fn is_cloud_backup_enabled(&self) -> bool {
        Self::load_persisted_state().is_configured()
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

    /// Reset local cloud backup state (keychain + DB) without touching iCloud
    ///
    /// Debug-only: pair with Swift-side iCloud wipe for full reset
    pub fn debug_reset_cloud_backup_state(&self) {
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
        self.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.refresh_persisted_flags();
        self.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        self.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        self.apply_detail_outcome(CloudBackupDetailOutcome::Cleared);
        self.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        self.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
        self.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Reset);
        self.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Idle);
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
        self.refresh_sync_health();
        send!(self.supervisor.clear_upload_runtime_state());
        info!("Debug: reset cloud backup local state (including master key)");
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
        if !matches!(
            Self::load_persisted_state(),
            PersistedCloudBackupState::Configured(_) | PersistedCloudBackupState::Disabling(_)
        ) {
            return;
        }

        self.handle_wallet_backup_change_and_reverify(metadata.id);
    }
}

impl RustCloudBackupManager {
    pub(crate) fn enable_cloud_backup(&self, context: CloudBackupEnableContext) {
        send!(self.supervisor.start_operation(CloudBackupOperation::Enable(context), None));
    }

    pub(crate) fn enable_cloud_backup_force_new(&self, context: CloudBackupEnableContext) {
        send!(self.supervisor.start_operation(CloudBackupOperation::EnableForceNew(context), None));
    }

    pub(crate) fn enable_cloud_backup_no_discovery(&self, context: CloudBackupEnableContext) {
        send!(
            self.supervisor.start_operation(CloudBackupOperation::EnableNoDiscovery(context), None)
        );
    }

    pub(crate) fn disable_cloud_backup(&self) {
        send!(self.supervisor.start_operation(CloudBackupOperation::Disable, None));
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
        async_test_lock, ensure_cloud_backup_test_tokio_runtime,
        persisted_enabled_cloud_backup_state, test_globals, test_lock,
    };
    use super::*;
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
        PersistedPasskeyState,
    };
    use act_zero::call;
    use cove_device::cloud_storage::CloudStorageError;

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
        let completion = PendingVerificationCompletion {
            report: DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            namespace_id: "namespace".into(),
            uploads: Vec::new(),
            created_at: Some(11),
        };

        assert!(completion.is_expired(10, 60));
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
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
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
}
