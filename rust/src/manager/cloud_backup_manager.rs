pub(crate) mod actors;
mod cloud_inventory;
mod detail;
mod keychain;
mod model;
mod ops;
mod pending;
mod store;
mod verify;
mod wallets;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use act_zero::{Addr, call, send};
use cove_cspp::backup_data::{MASTER_KEY_RECORD_ID, wallet_record_id};
use cove_device::cloud_storage::{
    CloudAccessPolicy, CloudStorage, CloudStorageClient, CloudStorageError, CloudSyncHealth,
};
use cove_device::passkey::{PasskeyError, PasskeyFailureReason, PasskeyOperation};
use cove_tokio::task::spawn_actor;
use cove_util::ResultExt as _;
use flume::{Receiver, Sender};
use futures::TryStreamExt as _;
use futures::stream::{self, StreamExt as _};
use parking_lot::RwLock;
use sha2::{Digest as _, Sha256};
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use cove_device::keychain::Keychain;
use cove_types::network::Network;

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, CloudBlobFailureIssue, PersistedCloudBackupState,
    PersistedCloudBackupStatus, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    PersistedDeepVerificationReport, PersistedPendingVerificationCompletion,
    PersistedPendingVerificationUpload,
};
use crate::wallet::metadata::{
    WalletId, WalletMetadata, WalletMode as LocalWalletMode, WalletType,
};

pub(crate) use self::actors::CloudBackupRestoreEvent;
use self::actors::{
    CloudBackupOperation, CloudBackupSupervisor, CloudBackupUploadedWallet,
    CloudBackupWalletCountRefresh, CloudBackupWriteBlocker, CloudBackupWriteClient,
    CloudBackupWriteCompletion, CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor,
};
use self::cloud_inventory::RemoteWalletTruth;
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
pub(crate) use self::keychain::CloudBackupKeychain;
use self::model::{
    CloudBackupAcceptedEnablePrompt, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim, CloudBackupStateReducer, CloudBackupStateReducerEffects,
    CloudBackupStateReducerEvent,
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
pub(crate) use self::store::CloudBackupStore;
use self::verify::coordinator::{
    CloudBackupVerificationCoordinator, CloudBackupVerificationEffect,
};
use self::wallets::wallet_metadata_change_requires_upload;
use self::wallets::{StagedPrfKey, UnpersistedPrfKey, WalletBackupLookup, WalletBackupReader};
use super::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

type LocalWalletSecret = crate::backup::model::WalletSecret;

const PASSKEY_RP_ID: &str = "covebitcoinwallet.com";
pub(crate) const SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE: &str =
    "master key backup is missing from cloud storage";
#[cfg(not(target_os = "android"))]
const PASSKEY_TEMPORARILY_UNAVAILABLE_MESSAGE: &str =
    "Your iPhone is still getting your Cove passkey ready. Wait a moment, then try again.";
#[cfg(target_os = "android")]
const PASSKEY_TEMPORARILY_UNAVAILABLE_MESSAGE: &str =
    "Cove is still getting your passkey ready. Wait a moment, then try again.";
const GENERIC_PASSKEY_ERROR_MESSAGE: &str = "Cove couldn't use this passkey. Please try again.";
const ANDROID_PASSKEY_ASSOCIATION_MESSAGE: &str = concat!(
    "Cove could not verify Android passkey setup yet. Wait a few minutes and try again. ",
    "If this keeps happening, update Cove or contact support."
);
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

/// Typed state delta sent from Rust to Swift and Kotlin reconcilers
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupReconcileMessage {
    Lifecycle(Box<CloudBackupLifecycle>, CloudBackupSettingsRowStatus),
    EnableCompleted(CloudBackupEnableContext),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudStorageIssue {
    AuthorizationRequired,
    Offline,
    Unavailable,
    NotFound,
    QuotaExceeded,
    Other,
}

pub(crate) fn is_connectivity_related_issue(issue: impl Into<CloudStorageIssue>) -> bool {
    matches!(issue.into(), CloudStorageIssue::Offline | CloudStorageIssue::Unavailable)
}

pub(crate) fn blocking_cloud_error(
    step: BlockingCloudStep,
    error: CloudBackupError,
) -> CloudBackupError {
    if CloudStorageIssue::from(&error) == CloudStorageIssue::Offline {
        return offline_error_for_step(step);
    }

    error
}

impl From<CloudBackupError> for CloudStorageIssue {
    fn from(error: CloudBackupError) -> Self {
        Self::from(&error)
    }
}

impl From<&CloudBackupError> for CloudStorageIssue {
    fn from(error: &CloudBackupError) -> Self {
        match error {
            CloudBackupError::Offline(_) | CloudBackupError::Deferred(_) => Self::Offline,
            CloudBackupError::CloudStorage(error) => error.into(),
            CloudBackupError::CloudStorageContext { source, .. } => source.into(),
            CloudBackupError::Cloud(_) => Self::Other,
            CloudBackupError::NotSupported(_)
            | CloudBackupError::UnsupportedPasskeyProvider
            | CloudBackupError::RecoveryRequired(_)
            | CloudBackupError::Passkey(_)
            | CloudBackupError::Crypto(_)
            | CloudBackupError::Internal(_)
            | CloudBackupError::Compatibility(_)
            | CloudBackupError::PasskeyMismatch
            | CloudBackupError::NoBackupFound
            | CloudBackupError::PasskeyDiscoveryCancelled
            | CloudBackupError::Cancelled => Self::Other,
        }
    }
}

impl From<CloudStorageError> for CloudStorageIssue {
    fn from(error: CloudStorageError) -> Self {
        Self::from(&error)
    }
}

impl From<&CloudStorageError> for CloudStorageIssue {
    fn from(error: &CloudStorageError) -> Self {
        match error {
            CloudStorageError::AuthorizationRequired(_) => Self::AuthorizationRequired,
            CloudStorageError::Offline(_) => Self::Offline,
            CloudStorageError::NotAvailable(_) => Self::Unavailable,
            CloudStorageError::NotFound(_) => Self::NotFound,
            CloudStorageError::QuotaExceeded => Self::QuotaExceeded,
            CloudStorageError::UploadFailed(_)
            | CloudStorageError::DownloadFailed(_)
            | CloudStorageError::InvalidNamespace(_) => Self::Other,
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

pub(crate) fn offline_error_for_step(step: BlockingCloudStep) -> CloudBackupError {
    CloudBackupError::Offline(offline_message_for_step(step).into())
}

fn offline_message_for_step(step: BlockingCloudStep) -> &'static str {
    use BlockingCloudStep as B;
    match step {
        B::Enable => "Reconnect to the internet, then try enabling cloud backup again",
        B::Restore => "Reconnect to the internet, then try restoring from cloud backup again",
        B::Verify => "Reconnect to the internet, then try verifying cloud backup again",
        B::Sync => "Reconnect to the internet, then try syncing cloud backup again",
        B::FetchCloudOnly => "Reconnect to the internet, then try loading cloud-only wallets again",
        B::RestoreCloudWallet => {
            "Reconnect to the internet, then try restoring this cloud wallet again"
        }
        B::DeleteCloudWallet => {
            "Reconnect to the internet, then try deleting this cloud wallet again"
        }
        B::RecoverOtherBackups => {
            "Reconnect to the internet, then try recovering the other cloud backups again"
        }
        B::DeleteOtherBackups => {
            "Reconnect to the internet, then try deleting the other cloud backups again"
        }
        B::Disable => "Reconnect to the internet, then try disabling cloud backup again",
        B::RecreateManifest => {
            "Reconnect to the internet, then try recreating the cloud backup manifest again"
        }
        B::RepairPasskey => "Reconnect to the internet, then try repairing cloud backup again",
        B::DetailRefresh => {
            "Reconnect to the internet, then try refreshing cloud backup details again"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingCloudStep {
    Enable,
    Restore,
    Verify,
    Sync,
    FetchCloudOnly,
    RestoreCloudWallet,
    DeleteCloudWallet,
    RecoverOtherBackups,
    DeleteOtherBackups,
    Disable,
    RecreateManifest,
    RepairPasskey,
    DetailRefresh,
}

pub(crate) struct PendingEnableSessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<UnpersistedPrfKey>,
    context: CloudBackupEnableContext,
}

pub(crate) struct PendingSavedPasskeySessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<StagedPrfKey>,
    context: CloudBackupEnableContext,
}

/// Tracks passkey material created during enable before the flow fully completes
#[allow(dead_code)]
pub(crate) enum PendingEnableSession {
    /// A new passkey and master key are staged while the user confirms Create New Backup
    AwaitingForceNewConfirmation(PendingEnableSessionMaterial),
    /// Upload already started and should retry with the same staged passkey material
    RetryUpload(PendingEnableSessionMaterial),
    /// A registered passkey is staged until targeted PRF auth confirms it can be used
    AwaitingSavedPasskeyConfirmation(PendingSavedPasskeySessionMaterial),
}

fn cloud_only_cache_is_stale(
    cloud_only: &CloudOnlyState,
    detail: &CloudBackupDetail,
    detail_snapshot: Option<&CloudBackupDetail>,
) -> bool {
    let CloudOnlyState::Loaded { wallets } = cloud_only else {
        return false;
    };

    if detail_snapshot != Some(detail) {
        return true;
    }

    if wallets.len() as u32 != detail.cloud_only_count {
        return true;
    }

    wallets.iter().any(|cloud_wallet| {
        detail
            .up_to_date
            .iter()
            .chain(detail.needs_sync.iter())
            .any(|local_wallet| local_wallet.record_id == cloud_wallet.record_id)
    })
}

#[derive(Debug, Clone)]
pub(crate) struct PendingVerificationCompletion {
    report: DeepVerificationReport,
    namespace_id: String,
    uploads: Vec<PendingVerificationUpload>,
    created_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingVerificationUpload {
    MasterKeyWrapper,
    Wallet { record_id: String, expected_revision: String },
}

impl std::fmt::Debug for PendingEnableSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingEnableSession").finish_non_exhaustive()
    }
}

impl PendingEnableSessionMaterial {
    fn new(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self { master_key: Zeroizing::new(master_key), passkey: Zeroizing::new(passkey), context }
    }

    fn into_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>) {
        (self.master_key, self.passkey)
    }

    fn namespace_id(&self) -> String {
        self.master_key.namespace_id()
    }

    fn context(&self) -> CloudBackupEnableContext {
        self.context
    }
}

impl PendingSavedPasskeySessionMaterial {
    fn new(
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<StagedPrfKey>,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self { master_key, passkey, context }
    }

    fn into_parts(self) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<StagedPrfKey>) {
        (self.master_key, self.passkey)
    }

    fn namespace_id(&self) -> String {
        self.master_key.namespace_id()
    }

    fn context(&self) -> CloudBackupEnableContext {
        self.context
    }
}

impl PendingEnableSession {
    fn retry_upload(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self::RetryUpload(PendingEnableSessionMaterial::new(master_key, passkey, context))
    }

    fn into_ready_parts(
        self,
    ) -> Result<
        (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>),
        CloudBackupError,
    > {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                Ok(material.into_parts())
            }
            Self::AwaitingSavedPasskeyConfirmation(_) => Err(CloudBackupError::Internal(
                "pending enable session did not contain authenticated passkey material".into(),
            )),
        }
    }

    fn into_staged_parts(
        self,
    ) -> Result<
        (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<StagedPrfKey>),
        CloudBackupError,
    > {
        match self {
            Self::AwaitingSavedPasskeyConfirmation(material) => Ok(material.into_parts()),
            Self::AwaitingForceNewConfirmation(_) | Self::RetryUpload(_) => {
                Err(CloudBackupError::Internal(
                    "pending enable session did not contain staged passkey material".into(),
                ))
            }
        }
    }

    fn namespace_id(&self) -> String {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.namespace_id()
            }
            Self::AwaitingSavedPasskeyConfirmation(material) => material.namespace_id(),
        }
    }

    fn context(&self) -> CloudBackupEnableContext {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.context()
            }
            Self::AwaitingSavedPasskeyConfirmation(material) => material.context(),
        }
    }

    fn is_retry_upload(&self) -> bool {
        matches!(self, Self::RetryUpload(_))
    }

    fn is_awaiting_force_new_confirmation(&self) -> bool {
        matches!(self, Self::AwaitingForceNewConfirmation(_))
    }

    fn is_awaiting_saved_passkey_confirmation(&self) -> bool {
        matches!(self, Self::AwaitingSavedPasskeyConfirmation(_))
    }

    fn awaiting_saved_passkey_confirmation(
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<StagedPrfKey>,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self::AwaitingSavedPasskeyConfirmation(PendingSavedPasskeySessionMaterial::new(
            master_key, passkey, context,
        ))
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

impl PendingVerificationCompletion {
    fn new(
        report: DeepVerificationReport,
        namespace_id: String,
        uploads: Vec<PendingVerificationUpload>,
    ) -> Self {
        Self {
            report,
            namespace_id,
            uploads,
            created_at: Some(crate::manager::cloud_backup_manager::current_timestamp()),
        }
    }

    pub(crate) fn report(&self) -> &DeepVerificationReport {
        &self.report
    }

    pub(crate) fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    pub(crate) fn uploads(&self) -> &[PendingVerificationUpload] {
        &self.uploads
    }

    pub(crate) fn is_expired(&self, now: u64, ttl_seconds: u64) -> bool {
        let Some(created_at) = self.created_at else {
            // legacy persisted completions predate created_at and must be restarted
            return true;
        };

        if created_at > now {
            return true;
        }

        now.saturating_sub(created_at) >= ttl_seconds
    }

    fn persisted(&self) -> PersistedPendingVerificationCompletion {
        PersistedPendingVerificationCompletion {
            report: PersistedDeepVerificationReport::from(&self.report),
            namespace_id: self.namespace_id.clone(),
            created_at: self.created_at,
            uploads: self
                .uploads
                .iter()
                .cloned()
                .map(PersistedPendingVerificationUpload::from)
                .collect(),
        }
    }

    fn from_persisted(completion: PersistedPendingVerificationCompletion) -> Self {
        Self {
            report: DeepVerificationReport::from(completion.report),
            namespace_id: completion.namespace_id,
            created_at: completion.created_at,
            uploads: completion
                .uploads
                .into_iter()
                .map(PendingVerificationUpload::from_persisted)
                .collect(),
        }
    }
}

impl PendingVerificationUpload {
    pub(crate) fn new(record_id: String, expected_revision: String) -> Self {
        Self::Wallet { record_id, expected_revision }
    }

    pub(crate) fn master_key_wrapper() -> Self {
        Self::MasterKeyWrapper
    }

    pub(crate) fn record_id(&self) -> &str {
        match self {
            Self::MasterKeyWrapper => MASTER_KEY_RECORD_ID,
            Self::Wallet { record_id, .. } => record_id,
        }
    }

    pub(crate) fn expected_revision(&self) -> &str {
        match self {
            Self::MasterKeyWrapper => "master-key-wrapper",
            Self::Wallet { expected_revision, .. } => expected_revision,
        }
    }

    pub(crate) fn wallet_record_id(&self) -> Option<&str> {
        match self {
            Self::MasterKeyWrapper => None,
            Self::Wallet { record_id, .. } => Some(record_id),
        }
    }

    pub(crate) fn wallet_revision(&self) -> Option<&str> {
        match self {
            Self::MasterKeyWrapper => None,
            Self::Wallet { expected_revision, .. } => Some(expected_revision),
        }
    }

    pub(crate) fn target_revision(&self, sync_state: Option<&PersistedCloudBlobState>) -> String {
        let Self::Wallet { expected_revision, .. } = self else {
            return self.expected_revision().to_owned();
        };

        sync_state
            .and_then(PersistedCloudBlobState::revision_hash)
            .unwrap_or(expected_revision)
            .to_owned()
    }

    fn from_persisted(upload: PersistedPendingVerificationUpload) -> Self {
        match upload {
            PersistedPendingVerificationUpload::MasterKeyWrapper => Self::MasterKeyWrapper,
            PersistedPendingVerificationUpload::Wallet { record_id, expected_revision } => {
                Self::Wallet { record_id, expected_revision }
            }
        }
    }
}

impl PersistedCloudBlobState {
    pub(crate) fn revision_hash(&self) -> Option<&str> {
        match self {
            Self::Uploading(state) => Some(&state.revision_hash),
            Self::UploadedPendingConfirmation(state) => Some(&state.revision_hash),
            Self::Confirmed(state) => Some(&state.revision_hash),
            Self::Failed(state) => state.revision_hash.as_deref(),
            Self::Dirty(_) => None,
        }
    }
}

impl DeepVerificationReport {
    fn from(report: PersistedDeepVerificationReport) -> Self {
        Self {
            master_key_wrapper_repaired: report.master_key_wrapper_repaired,
            local_master_key_repaired: report.local_master_key_repaired,
            credential_recovered: report.credential_recovered,
            wallets_verified: report.wallets_verified,
            wallets_failed: report.wallets_failed,
            wallets_unsupported: report.wallets_unsupported,
            detail: None,
        }
    }
}

impl From<&DeepVerificationReport> for PersistedDeepVerificationReport {
    fn from(report: &DeepVerificationReport) -> Self {
        Self {
            master_key_wrapper_repaired: report.master_key_wrapper_repaired,
            local_master_key_repaired: report.local_master_key_repaired,
            credential_recovered: report.credential_recovered,
            wallets_verified: report.wallets_verified,
            wallets_failed: report.wallets_failed,
            wallets_unsupported: report.wallets_unsupported,
        }
    }
}

impl From<PendingVerificationUpload> for PersistedPendingVerificationUpload {
    fn from(upload: PendingVerificationUpload) -> Self {
        match upload {
            PendingVerificationUpload::MasterKeyWrapper => Self::MasterKeyWrapper,
            PendingVerificationUpload::Wallet { record_id, expected_revision } => {
                Self::Wallet { record_id, expected_revision }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudBackupError {
    #[error("not supported: {0}")]
    NotSupported(String),

    #[error("passkey provider does not support PRF for Cloud Backup")]
    UnsupportedPasskeyProvider,

    #[error("{0}")]
    RecoveryRequired(String),

    #[error("passkey error: {0}")]
    Passkey(#[from] PasskeyError),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("cloud storage error: {0}")]
    Cloud(String),

    #[error("cloud storage error: {0}")]
    CloudStorage(#[from] CloudStorageError),

    #[error("cloud storage error: {context}: {source}")]
    CloudStorageContext { context: String, source: CloudStorageError },

    #[error("offline: {0}")]
    Offline(String),

    #[error("deferred until connected: {0}")]
    Deferred(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("compatibility error: {0}")]
    Compatibility(String),

    #[error("Passkey didn't match any backups, please try a new one")]
    PasskeyMismatch,

    #[error("no cloud backups found")]
    NoBackupFound,

    #[error("user cancelled passkey discovery")]
    PasskeyDiscoveryCancelled,

    #[error("restore cancelled")]
    Cancelled,
}

impl CloudBackupError {
    pub(crate) fn cloud_storage_context(
        context: impl Into<String>,
        source: CloudStorageError,
    ) -> Self {
        Self::CloudStorageContext { context: context.into(), source }
    }

    pub(crate) fn is_cloud_error(&self) -> bool {
        matches!(self, Self::Cloud(_) | Self::CloudStorage(_) | Self::CloudStorageContext { .. })
    }

    pub(crate) fn is_platform_authorization_failure(&self) -> bool {
        matches!(
            self,
            Self::Passkey(PasskeyError::RequestFailed {
                reason: PasskeyFailureReason::PlatformAuthorizationFailed,
                ..
            })
        )
    }

    pub(crate) fn reader_message(&self) -> String {
        match self {
            Self::Passkey(PasskeyError::RequestFailed {
                reason: PasskeyFailureReason::PlatformAuthorizationFailed,
                ..
            }) => PASSKEY_TEMPORARILY_UNAVAILABLE_MESSAGE.into(),
            Self::Passkey(PasskeyError::RequestFailed {
                operation: PasskeyOperation::Registration,
                reason: PasskeyFailureReason::DeviceNotConfigured,
            }) => ANDROID_PASSKEY_ASSOCIATION_MESSAGE.into(),
            Self::Passkey(PasskeyError::UserCancelled) => {
                Self::PasskeyDiscoveryCancelled.to_string()
            }
            Self::Passkey(PasskeyError::NoCredentialFound) => Self::PasskeyMismatch.to_string(),
            Self::Passkey(PasskeyError::PrfUnsupportedProvider) => {
                Self::UnsupportedPasskeyProvider.to_string()
            }
            Self::Passkey(_) => GENERIC_PASSKEY_ERROR_MESSAGE.into(),
            _ => self.to_string(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CatastrophicRecoveryError {
    #[error("{0}")]
    Failure(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreResult {
    BackupFound,
    NoBackupFound { message: String },
    Offline { message: String },
    Unreadable { message: String },
    Inconclusive { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreProvider {
    ICloudDrive,
    GoogleDrive,
}

impl CatastrophicCloudRestoreProvider {
    fn storage_name(self) -> &'static str {
        match self {
            Self::ICloudDrive => "iCloud",
            Self::GoogleDrive => "Google Drive",
        }
    }

    fn account_name(self) -> &'static str {
        match self {
            Self::ICloudDrive => "iCloud account",
            Self::GoogleDrive => "Google account",
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait CloudBackupManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: CloudBackupReconcileMessage);
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

    fn load_persisted_flags() -> (CloudBackupVerificationMetadata, bool) {
        let db_state = Self::load_persisted_state();
        ((&db_state).into(), db_state.should_prompt_verification())
    }

    pub(super) fn send(&self, message: Message) {
        if let Err(error) = self.reconciler.send(message) {
            error!("unable to send cloud backup message: {error:?}");
        }
    }

    fn apply_model_event(&self, event: CloudBackupStateReducerEvent) -> bool {
        let effects = match self.state.write().apply_event(event) {
            Ok(effects) => effects,
            Err(rejection) => match rejection {},
        };

        self.send_model_effects(effects);
        true
    }

    fn send_model_effects(&self, effects: CloudBackupStateReducerEffects) {
        if let Some(lifecycle) = effects.lifecycle {
            self.send(Message::Lifecycle(
                Box::new(lifecycle.lifecycle),
                lifecycle.settings_row_status,
            ));
        }

        if let Some(context) = effects.enable_completed {
            self.send(Message::EnableCompleted(context));
        }
    }

    pub(crate) fn reconcile_runtime_status(&self, status: CloudBackupStatus) {
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::Enabling) {
            self.clear_runtime_passkey_authorization();
        }

        let event = CloudBackupStateReducerEvent::RuntimeStatusReconciled(status);
        let effects = match self.state.write().apply_event(event) {
            Ok(effects) => effects,
            Err(rejection) => match rejection {},
        };
        let status_changed = effects.status_changed;
        self.send_model_effects(effects);

        if !status_changed {
            return;
        }

        self.apply_model_event(CloudBackupStateReducerEvent::MissingPasskeyDismissalCleared);
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

    pub(crate) fn observe_sync_health(&self, sync_health: CloudSyncHealth) {
        self.apply_model_event(CloudBackupStateReducerEvent::SyncHealthObserved(sync_health));
    }

    pub(crate) fn reconcile_verification_presentation(
        &self,
        presentation: CloudBackupVerificationPresentation,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::VerificationPresentationReconciled(
            presentation,
        ));
    }

    pub(crate) fn current_verification_source(&self) -> CloudBackupVerificationSource {
        CloudBackupVerificationCoordinator::current_source(
            self.state.read().verification_presentation(),
        )
    }

    pub(crate) fn apply_verification_effect(&self, effect: CloudBackupVerificationEffect) {
        if let Some(detail) = effect.detail {
            self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
        }

        if let Some(pending_upload_verification) = effect.pending_upload_verification {
            self.apply_pending_upload_verification_value(pending_upload_verification);
        }

        if let Some(presentation) = effect.presentation {
            self.reconcile_verification_presentation(presentation);
        }

        if let Some(verification) = effect.verification {
            self.apply_verification_outcome(CloudBackupVerificationOutcome::from_state(
                verification,
            ));
        }

        if let Some(recovery) = effect.recovery {
            self.apply_recovery_outcome(CloudBackupRecoveryOutcome::from_state(recovery));
        }

        if effect.refresh_sync_health {
            self.refresh_sync_health();
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

    pub(crate) fn refresh_persisted_flags(&self) {
        let (verification_metadata, should_prompt_verification) = Self::load_persisted_flags();

        self.apply_model_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
            metadata: verification_metadata,
            should_prompt: should_prompt_verification,
        });
    }

    fn apply_pending_upload_verification_value(&self, pending: PendingUploadVerificationState) {
        self.apply_model_event(CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(
            pending,
        ));
    }

    pub(crate) fn reconcile_pending_upload_verification(
        &self,
        pending: PendingUploadVerificationState,
    ) {
        self.reconcile_pending_upload_verification_for_source(
            pending,
            self.current_verification_source(),
        );
    }

    pub(crate) fn reconcile_pending_upload_verification_for_source(
        &self,
        pending: PendingUploadVerificationState,
        source: CloudBackupVerificationSource,
    ) {
        let (verification_metadata, should_prompt_verification) = Self::load_persisted_flags();
        let event = CloudBackupStateReducerEvent::PendingUploadVerificationAndFlagsReconciled {
            pending,
            metadata: verification_metadata,
            should_prompt: should_prompt_verification,
        };
        let effects = match self.state.write().apply_event(event) {
            Ok(effects) => effects,
            Err(rejection) => match rejection {},
        };
        let decision_pending = effects.verification_decision_pending;
        let presentation_changed = effects.verification_presentation_changed;
        self.send_model_effects(effects);

        if presentation_changed || decision_pending {
            return;
        }

        self.apply_verification_effect(CloudBackupVerificationCoordinator::pending_upload_state(
            pending, source,
        ));
    }

    pub(crate) fn refresh_pending_upload_verification_state(&self) {
        self.reconcile_pending_upload_verification(
            self.current_pending_upload_verification_state(),
        );
    }

    pub(crate) fn current_pending_upload_verification_state(
        &self,
    ) -> PendingUploadVerificationState {
        if self.has_pending_cloud_upload_verification() {
            return PendingUploadVerificationState::Confirming;
        }

        if self.pending_verification_completion().is_some() {
            return PendingUploadVerificationState::Confirming;
        }

        PendingUploadVerificationState::Idle
    }

    pub(crate) fn apply_detail_outcome(&self, outcome: CloudBackupDetailOutcome) {
        let detail = match outcome {
            CloudBackupDetailOutcome::Cleared => None,
            CloudBackupDetailOutcome::Refreshed(detail) => Some(detail),
        };
        let detail_snapshot = self.cloud_only_detail_snapshot.read().clone();
        let reset_cloud_only = {
            let state = self.state.read();
            detail.as_ref().is_some_and(|detail| {
                cloud_only_cache_is_stale(&state.cloud_only(), detail, detail_snapshot.as_ref())
            })
        };

        if reset_cloud_only {
            *self.cloud_only_detail_snapshot.write() = None;
        }

        self.apply_model_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
            detail,
            reset_cloud_only,
        });
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

    fn apply_cloud_only_state(&self, cloud_only: CloudOnlyState) {
        if !matches!(cloud_only, CloudOnlyState::Loaded { .. }) {
            *self.cloud_only_detail_snapshot.write() = None;
        }
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(cloud_only));
    }

    fn apply_loaded_cloud_only(&self, wallets: Vec<CloudBackupWalletItem>) {
        let detail = self.state.read().detail().clone();
        *self.cloud_only_detail_snapshot.write() = detail;
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
            CloudOnlyState::Loaded { wallets },
        ));
    }

    pub(crate) fn apply_cloud_only_fetch_outcome(&self, outcome: CloudBackupCloudOnlyFetchOutcome) {
        match outcome {
            CloudBackupCloudOnlyFetchOutcome::Reset => {
                self.apply_cloud_only_state(CloudOnlyState::NotFetched);
                self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
            }
            CloudBackupCloudOnlyFetchOutcome::Started => {
                self.apply_cloud_only_state(CloudOnlyState::Loading);
                self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
            }
            CloudBackupCloudOnlyFetchOutcome::Loaded(wallets) => {
                self.apply_loaded_cloud_only(wallets);
            }
            CloudBackupCloudOnlyFetchOutcome::Failed(error) => {
                self.apply_cloud_only_state(CloudOnlyState::Failed { error });
            }
        }
    }

    fn apply_cloud_only_operation(&self, cloud_only_operation: CloudOnlyOperation) {
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyOperationResolved(
            cloud_only_operation,
        ));
    }

    pub(crate) fn apply_cloud_only_wallet_outcome(
        &self,
        outcome: CloudBackupCloudOnlyWalletOutcome,
    ) {
        match outcome {
            CloudBackupCloudOnlyWalletOutcome::Started { record_id } => {
                self.apply_cloud_only_operation(CloudOnlyOperation::Operating { record_id });
            }
            CloudBackupCloudOnlyWalletOutcome::Restored { record_id, warning } => {
                self.apply_finished_cloud_only_wallet_operation(record_id, warning);
            }
            CloudBackupCloudOnlyWalletOutcome::SkippedDuplicate { record_id } => {
                self.apply_finished_cloud_only_wallet_operation(record_id, None);
            }
            CloudBackupCloudOnlyWalletOutcome::Deleted { record_id } => {
                self.apply_finished_cloud_only_wallet_operation(record_id, None);
            }
            CloudBackupCloudOnlyWalletOutcome::Failed(error) => {
                self.apply_cloud_only_operation(CloudOnlyOperation::Failed { error });
            }
        }
    }

    fn apply_finished_cloud_only_wallet_operation(
        &self,
        record_id: String,
        warning: Option<CloudBackupCloudOnlyOperationWarning>,
    ) {
        if let Some(warning) = warning {
            self.apply_cloud_only_operation(CloudOnlyOperation::Warning {
                message: warning.message,
                error: warning.error,
            });
        } else {
            self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
        }

        let mut cloud_only = self.state.read().cloud_only().clone();
        if let CloudOnlyState::Loaded { wallets } = &mut cloud_only {
            wallets.retain(|wallet| wallet.record_id != record_id);
        }
        self.apply_cloud_only_state(cloud_only);
    }

    pub(crate) fn apply_other_backups_outcome(&self, outcome: CloudBackupOtherBackupsOutcome) {
        let other_backups_operation = match outcome {
            CloudBackupOtherBackupsOutcome::Idle => OtherBackupsOperation::Idle,
            CloudBackupOtherBackupsOutcome::Recovering => OtherBackupsOperation::Recovering,
            CloudBackupOtherBackupsOutcome::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            } => OtherBackupsOperation::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            },
            CloudBackupOtherBackupsOutcome::Deleting => OtherBackupsOperation::Deleting,
            CloudBackupOtherBackupsOutcome::Deleted => OtherBackupsOperation::Deleted,
            CloudBackupOtherBackupsOutcome::Failed(error) => {
                OtherBackupsOperation::Failed { error }
            }
        };

        self.apply_model_event(CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
            other_backups_operation,
        ));
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

    pub(crate) async fn build_cloud_backup_detail_with_remote_truth(
        &self,
        wallet_record_ids: &[String],
        remote_wallet_truth: RemoteWalletTruth,
    ) -> Result<CloudBackupDetail, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let other_backups = self.other_backup_state(&cloud).await;

        Ok(self::cloud_inventory::CloudWalletInventory::load_with_remote_truth(
            wallet_record_ids,
            remote_wallet_truth,
        )
        .await?
        .build_detail(other_backups))
    }

    pub(crate) async fn other_backup_state(
        &self,
        cloud: &CloudStorageClient,
    ) -> CloudBackupOtherBackupsState {
        match self.other_backup_summary(cloud).await {
            Ok(summary) => CloudBackupOtherBackupsState::Loaded { summary },
            Err(error) => {
                warn!("Failed to summarize other cloud backups: {error}");
                CloudBackupOtherBackupsState::LoadFailed { error: error.to_string() }
            }
        }
    }

    pub(crate) async fn other_backup_summary(
        &self,
        cloud: &CloudStorageClient,
    ) -> Result<CloudBackupOtherBackupsSummary, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let local_wallet_record_ids = self.expected_wallet_record_ids().await?;
        let namespaces = self
            .other_backup_namespaces(cloud, &current_namespace, BlockingCloudStep::DetailRefresh)
            .await?;
        let passkey_hints = self.passkey_hints_for_namespaces(cloud, &namespaces).await;

        let mut namespace_count = 0;
        let mut wallet_count = 0;

        for namespace in &namespaces {
            let record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
                Ok(record_ids) => record_ids,
                Err(error) => {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::DetailRefresh,
                        CloudBackupError::cloud_storage_context(
                            format!("count wallets in other backup namespace {namespace}"),
                            error,
                        ),
                    ));
                }
            };

            namespace_count += 1;
            let unrecovered_wallet_count = record_ids
                .iter()
                .filter(|record_id| !local_wallet_record_ids.contains(*record_id))
                .count() as u32;

            wallet_count += unrecovered_wallet_count;
        }

        Ok(CloudBackupOtherBackupsSummary { namespace_count, wallet_count, passkey_hints })
    }

    pub(crate) async fn best_passkey_hint_for_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[String],
    ) -> Option<CloudBackupPasskeyHint> {
        self.passkey_hints_for_namespaces(cloud, namespaces)
            .await
            .into_iter()
            .max_by_key(|hint| hint.registered_at)
    }

    async fn passkey_hints_for_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[String],
    ) -> Vec<CloudBackupPasskeyHint> {
        let mut hints_by_suffix =
            std::collections::HashMap::<String, CloudBackupPasskeyHint>::new();

        for namespace in namespaces {
            let Ok(master_json) =
                cloud.download_master_key_backup(namespace.clone()).await.inspect_err(|error| {
                    warn!("Failed to load passkey hint for namespace {namespace}: {error}")
                })
            else {
                continue;
            };

            let Ok(encrypted) = serde_json::from_slice::<
                cove_cspp::backup_data::EncryptedMasterKeyBackup,
            >(&master_json)
            .inspect_err(|error| {
                warn!("Failed to parse passkey hint for namespace {namespace}: {error}")
            }) else {
                continue;
            };
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                warn!("Failed to normalize passkey hint for namespace {namespace}");
                continue;
            }

            let Some(provider_hint) = encrypted.passkey_provider_hint.as_ref() else {
                continue;
            };
            let hint = CloudBackupPasskeyHint::from_provider_hint(provider_hint);

            hints_by_suffix
                .entry(hint.name_suffix.clone())
                .and_modify(|current| {
                    if hint.registered_at > current.registered_at {
                        *current = hint.clone();
                    }
                })
                .or_insert(hint);
        }

        let mut hints = hints_by_suffix.into_values().collect::<Vec<_>>();
        hints.sort_by_key(|hint| std::cmp::Reverse(hint.registered_at));
        hints
    }

    pub(crate) async fn other_backup_namespaces(
        &self,
        cloud: &CloudStorageClient,
        current_namespace: &str,
        step: BlockingCloudStep,
    ) -> Result<Vec<String>, CloudBackupError> {
        let mut namespaces = cloud.list_namespaces().await.map_err(|error| {
            blocking_cloud_error(
                step,
                CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
            )
        })?;

        namespaces.retain(|namespace| namespace != current_namespace);
        namespaces.sort();

        let mut backup_namespaces = Vec::new();
        for namespace in namespaces {
            match cloud.download_master_key_backup(namespace.clone()).await {
                Ok(_) => backup_namespaces.push(namespace),
                Err(CloudStorageError::NotFound(_)) => {}
                Err(error) => {
                    return Err(blocking_cloud_error(
                        step,
                        CloudBackupError::cloud_storage_context(
                            "inspect cloud backup namespace",
                            error,
                        ),
                    ));
                }
            }
        }

        Ok(backup_namespaces)
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

    pub(crate) async fn compute_sync_health(&self) -> CloudSyncHealth {
        self.compute_sync_health_with_master_key_grace(None).await
    }

    pub(crate) async fn compute_sync_health_with_master_key_grace(
        &self,
        master_key_upload_grace_namespace: Option<&str>,
    ) -> CloudSyncHealth {
        if !Self::load_persisted_state().is_configured() {
            return CloudSyncHealth::Unknown;
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => return CloudSyncHealth::Failed(error.to_string()),
        };
        let expected_wallet_record_ids = match self.expected_wallet_record_ids().await {
            Ok(record_ids) => record_ids,
            Err(error) => return CloudSyncHealth::Failed(error.to_string()),
        };
        let sync_states = match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => {
                if let Some(sync_health) = Self::sync_health_from_corrupt_sync_state(&states) {
                    return sync_health;
                }

                states
                    .into_iter()
                    .filter(|state| {
                        state.namespace_id == namespace
                            && (state.wallet_id().is_none()
                                || expected_wallet_record_ids.contains(state.record_id()))
                    })
                    .collect::<Vec<_>>()
            }
            Err(error) => {
                return CloudSyncHealth::Failed(format!(
                    "failed to read cloud backup sync states: {error}",
                ));
            }
        };

        if let Some(sync_health) = Self::sync_health_from_local_failures(&sync_states) {
            return sync_health;
        }

        if master_key_upload_grace_namespace == Some(namespace.as_str()) {
            return CloudSyncHealth::Uploading;
        }

        if Self::sync_health_has_pending_master_key_upload(&sync_states) {
            return CloudSyncHealth::Uploading;
        }

        let cloud = CloudStorage::global_silent_client();
        let master_key_uploaded = match cloud
            .is_backup_uploaded(namespace.clone(), MASTER_KEY_RECORD_ID.to_string())
            .await
        {
            Ok(is_uploaded) => is_uploaded,
            Err(CloudStorageError::NotFound(_)) => false,
            Err(error) => return Self::sync_health_from_cloud_error(error),
        };

        if expected_wallet_record_ids.is_empty() {
            if master_key_uploaded {
                return CloudSyncHealth::AllUploaded;
            }

            return CloudSyncHealth::NoFiles;
        }

        if !master_key_uploaded {
            return CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into());
        }

        if Self::sync_health_has_pending_wallet_upload(&sync_states) {
            return CloudSyncHealth::Uploading;
        }

        let remote_wallet_record_ids = match cloud.list_wallet_backups(namespace).await {
            Ok(record_ids) => record_ids.into_iter().collect::<HashSet<_>>(),
            Err(CloudStorageError::NotFound(_)) => HashSet::new(),
            Err(error) => return Self::sync_health_from_cloud_error(error),
        };

        let missing_wallet_count = expected_wallet_record_ids
            .iter()
            .filter(|record_id| !remote_wallet_record_ids.contains(*record_id))
            .count();
        if missing_wallet_count > 0 {
            return CloudSyncHealth::Failed(sync_health_missing_wallet_message(
                missing_wallet_count,
            ));
        }

        CloudSyncHealth::AllUploaded
    }

    async fn expected_wallet_record_ids(&self) -> Result<HashSet<String>, CloudBackupError> {
        let local_wallets = CloudBackupStore::global().all_wallets()?;
        let record_ids =
            stream::iter(local_wallets)
                .map(|wallet| async move {
                    Ok::<_, CloudBackupError>(wallet_record_id(wallet.id.as_ref()))
                })
                .buffered(CLOUD_BACKUP_IO_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .await?;

        Ok(record_ids.into_iter().collect())
    }

    fn sync_health_from_local_failures(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Option<CloudSyncHealth> {
        if let Some(sync_health) = sync_states.iter().find_map(|sync_state| {
            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return None;
            };

            if failed_state.issue == Some(CloudBlobFailureIssue::AuthorizationRequired) {
                return Some(CloudSyncHealth::AuthorizationRequired(sync_health_failed_message(
                    sync_state,
                    failed_state,
                )));
            }

            None
        }) {
            return Some(sync_health);
        }

        sync_states.iter().find_map(|sync_state| {
            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return None;
            };

            Some(CloudSyncHealth::Failed(sync_health_failed_message(sync_state, failed_state)))
        })
    }

    fn sync_health_from_corrupt_sync_state(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Option<CloudSyncHealth> {
        sync_states.iter().find_map(|sync_state| {
            if !sync_state.is_corrupted() {
                return None;
            }

            let PersistedCloudBlobState::Failed(failed_state) = &sync_state.state else {
                return Some(CloudSyncHealth::Failed(
                    "cloud backup sync state could not be decoded".into(),
                ));
            };

            Some(CloudSyncHealth::Failed(sync_health_failed_message(sync_state, failed_state)))
        })
    }

    fn sync_health_has_pending_wallet_upload(sync_states: &[PersistedCloudBlobSyncState]) -> bool {
        sync_states.iter().any(|sync_state| {
            sync_state.is_wallet_record()
                && matches!(
                    sync_state.state,
                    PersistedCloudBlobState::Dirty(_)
                        | PersistedCloudBlobState::Uploading(_)
                        | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                )
        })
    }

    fn sync_health_has_pending_master_key_upload(
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> bool {
        sync_states.iter().any(|sync_state| {
            sync_state.is_master_key_wrapper()
                && sync_state.record_id() == MASTER_KEY_RECORD_ID
                && matches!(
                    sync_state.state,
                    PersistedCloudBlobState::Dirty(_)
                        | PersistedCloudBlobState::Uploading(_)
                        | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                )
        })
    }

    fn sync_health_from_cloud_error(error: CloudStorageError) -> CloudSyncHealth {
        match error {
            CloudStorageError::AuthorizationRequired(message) => {
                CloudSyncHealth::AuthorizationRequired(message)
            }
            CloudStorageError::NotAvailable(_) => CloudSyncHealth::Unavailable,
            CloudStorageError::Offline(message) => CloudSyncHealth::Failed(message),
            CloudStorageError::QuotaExceeded => {
                CloudSyncHealth::Failed("cloud storage quota was exceeded".into())
            }
            CloudStorageError::UploadFailed(message)
            | CloudStorageError::DownloadFailed(message)
            | CloudStorageError::NotFound(message)
            | CloudStorageError::InvalidNamespace(message) => CloudSyncHealth::Failed(message),
        }
    }

    pub(crate) fn mark_wallet_blob_dirty(&self, wallet_id: WalletId) {
        // disabling can be canceled, so wallet changes still need queued uploads
        if !matches!(
            Self::load_persisted_state(),
            PersistedCloudBackupState::Configured(_) | PersistedCloudBackupState::Disabling(_)
        ) {
            return;
        }

        let Ok(namespace_id) = self.current_namespace_id() else {
            warn!("Cloud backup dirty mark skipped, namespace is unavailable");
            return;
        };

        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();
        let record_id = wallet_record_id(wallet_id.as_ref());
        let sync_state = PersistedCloudBlobSyncState::wallet(
            namespace_id,
            wallet_id.clone(),
            record_id,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
        );

        if let Err(error) = Database::global().cloud_blob_sync_states.set(&sync_state) {
            error!("Failed to persist dirty cloud backup state: {error}");
            return;
        }

        if self.is_known_offline() {
            return;
        }

        self.schedule_wallet_upload(wallet_id, false);
    }

    pub(crate) fn mark_wallet_blobs_dirty_for_background_upload<I>(
        &self,
        wallet_ids: I,
    ) -> Result<(), CloudBackupError>
    where
        I: IntoIterator<Item = WalletId>,
    {
        let namespace_id = self.current_namespace_id()?;
        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();

        for wallet_id in wallet_ids {
            let record_id = wallet_record_id(wallet_id.as_ref());
            let sync_state = PersistedCloudBlobSyncState::wallet(
                namespace_id.clone(),
                wallet_id,
                record_id,
                PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            );

            Database::global()
                .cloud_blob_sync_states
                .set(&sync_state)
                .map_err_prefix("persist dirty cloud backup state", CloudBackupError::Internal)?;
        }

        self.refresh_sync_health();

        Ok(())
    }

    pub(crate) fn handle_wallet_metadata_update(
        &self,
        before: &WalletMetadata,
        after: &WalletMetadata,
    ) {
        if wallet_metadata_change_requires_upload(before, after) {
            self.mark_wallet_blob_dirty(after.id.clone());
        }
    }

    pub(crate) fn handle_wallet_backup_change(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
    }

    pub(crate) fn handle_wallet_backup_change_and_reverify(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
        self.mark_verification_required_after_wallet_change();
    }

    pub(crate) fn handle_wallet_set_change(&self) {
        self.mark_verification_required_after_wallet_change();
    }

    fn schedule_wallet_upload(&self, wallet_id: WalletId, immediate: bool) {
        if self.cloud_backup_writes_blocked() {
            return;
        }

        send!(self.supervisor.schedule_wallet_upload(wallet_id, immediate));
    }

    fn downgrade_interrupted_upload_to_dirty(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
    ) -> bool {
        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();

        match self.replace_blob_state_if_current(
            sync_state,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            "persist interrupted upload dirty state",
        ) {
            Ok(wrote_dirty) => wrote_dirty,
            Err(error) => {
                error!("Failed to downgrade interrupted upload state: {error}");
                false
            }
        }
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

    async fn load_remote_wallet_truth(
        &self,
        wallet_record_ids: &[String],
        cloud: CloudStorageClient,
    ) -> Result<RemoteWalletTruth, CloudBackupError> {
        let namespace = self.current_namespace_id()?;
        let db = Database::global();
        let local_wallets = CloudBackupStore::new(&db).all_wallets()?;
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let Some(master_key) = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
        else {
            return Ok(RemoteWalletTruth {
                unknown_record_ids: wallet_record_ids.iter().cloned().collect(),
                ..RemoteWalletTruth::default()
            });
        };

        let critical_key = master_key.critical_data_key();
        let mut remote_wallet_truth = RemoteWalletTruth::default();

        let mut summaries = stream::iter(local_wallets)
            .map(|wallet| {
                let cloud = cloud.clone();
                let namespace = namespace.clone();

                async move {
                    let record_id = wallet_record_id(wallet.id.as_ref());
                    let reader =
                        WalletBackupReader::new(cloud, namespace, Zeroizing::new(critical_key));
                    let result = reader.summary(&record_id).await;
                    (record_id, result)
                }
            })
            .buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, result)) = summaries.next().await {
            match result {
                Ok(WalletBackupLookup::Found(summary)) => {
                    remote_wallet_truth.summaries_by_record_id.insert(record_id, summary);
                }
                Ok(WalletBackupLookup::NotFound) => {}
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!(
                        "Cloud backup remote truth found unsupported wallet backup version {version} for record_id={record_id}"
                    );
                    remote_wallet_truth.unsupported_record_ids.insert(record_id);
                }
                Err(error) => {
                    warn!("Cloud backup remote truth failed for record_id={record_id}: {error}");
                    remote_wallet_truth.unknown_record_ids.insert(record_id);
                }
            }
        }

        Ok(remote_wallet_truth)
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

    pub fn listen_for_updates(&self, reconciler: Box<dyn CloudBackupManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                reconciler.reconcile(field);
            }
        });
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

/// Reset local state for the database-encryption-key-mismatch recovery flow
///
/// Removes wallet keychain items, deletes local databases, then reinitializes
/// the database handle so bootstrap can start from a clean state
#[uniffi::export]
pub fn reset_local_data_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    wipe_local_data_for_catastrophic_recovery()?;
    clear_in_process_cloud_backup_state_for_catastrophic_recovery();
    reinit_database_after_catastrophic_recovery()
}

#[uniffi::export]
pub async fn check_catastrophic_cloud_restore_backup(
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    catastrophic_cloud_restore_check_result(
        CloudStorage::global().has_restorable_cloud_backup(CloudAccessPolicy::ConsentAllowed).await,
        provider,
    )
}

fn catastrophic_cloud_restore_check_result(
    result: Result<bool, CloudStorageError>,
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    match result {
        Ok(true) => CatastrophicCloudRestoreResult::BackupFound,
        Ok(false) => CatastrophicCloudRestoreResult::NoBackupFound {
            message: format!(
                "No Cloud Backup was found for the selected {}.",
                provider.account_name()
            ),
        },
        Err(error) => catastrophic_cloud_restore_error(error, provider),
    }
}

fn catastrophic_cloud_restore_error(
    error: CloudStorageError,
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    match error {
        CloudStorageError::AuthorizationRequired(message) => {
            if message.trim().is_empty() {
                return CatastrophicCloudRestoreResult::Inconclusive {
                    message: format!(
                        "{} access is required before local data can be reset.",
                        provider.storage_name()
                    ),
                };
            }

            CatastrophicCloudRestoreResult::Inconclusive { message }
        }
        CloudStorageError::Offline(message) => CatastrophicCloudRestoreResult::Offline {
            message: format!("Cannot check {} while offline: {message}", provider.storage_name()),
        },
        CloudStorageError::NotFound(_) => CatastrophicCloudRestoreResult::NoBackupFound {
            message: format!(
                "No Cloud Backup was found for the selected {}.",
                provider.account_name()
            ),
        },
        CloudStorageError::DownloadFailed(message) => CatastrophicCloudRestoreResult::Unreadable {
            message: format!("Cloud Backup data could not be read: {message}"),
        },
        CloudStorageError::InvalidNamespace(_) => CatastrophicCloudRestoreResult::Unreadable {
            message: "Cloud Backup data could not be read.".into(),
        },
        CloudStorageError::QuotaExceeded => CatastrophicCloudRestoreResult::Inconclusive {
            message: format!(
                "{} quota is exceeded. Cove could not check for a Cloud Backup.",
                provider.storage_name()
            ),
        },
        CloudStorageError::NotAvailable(message) => CatastrophicCloudRestoreResult::Inconclusive {
            message: format!("{} is unavailable: {message}", provider.storage_name()),
        },
        CloudStorageError::UploadFailed(message) => {
            CatastrophicCloudRestoreResult::Inconclusive { message }
        }
    }
}

fn wipe_local_data_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    use crate::database::migration::log_remove_file;

    wipe_wallet_keychain_items_for_catastrophic_recovery()?;
    CloudBackupKeychain::global()
        .clear_local_state()
        .map_err_str(CatastrophicRecoveryError::Failure)?;

    let root = &*cove_common::consts::ROOT_DATA_DIR;

    log_remove_file(&root.join("cove.encrypted.db"));
    log_remove_file(&root.join("cove.db"));

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("bdk_wallet") {
                log_remove_file(&entry.path());
            }
        }
    }

    let wallet_dir = &*cove_common::consts::WALLET_DATA_DIR;
    if wallet_dir.exists()
        && let Err(error) = std::fs::remove_dir_all(wallet_dir)
    {
        error!("Failed to remove wallet data dir: {error}");
    }

    Ok(())
}

fn clear_in_process_cloud_backup_state_for_catastrophic_recovery() {
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

    if let Some(manager) = LazyLock::get(&CLOUD_BACKUP_MANAGER) {
        manager.clear_in_process_state_for_local_reset();
    }
}

fn reinit_database_after_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    crate::database::wallet_data::DATABASE_CONNECTIONS.write().clear();
    Database::try_reinit()
        .map_err_prefix("reinitialize database", CatastrophicRecoveryError::Failure)
}

#[uniffi::export]
pub fn cspp_master_key_record_id() -> String {
    MASTER_KEY_RECORD_ID.to_string()
}

pub(crate) fn master_key_wrapper_revision_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[uniffi::export]
pub fn cspp_master_key_filename() -> String {
    cove_cspp::backup_data::master_key_filename()
}

#[uniffi::export]
pub fn cspp_wallet_filename_from_record_id(record_id: String) -> String {
    cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)
}

#[uniffi::export]
pub fn cspp_master_key_directory() -> String {
    cove_cspp::backup_data::remote_layout::MASTER_KEY_DIRECTORY.to_string()
}

#[uniffi::export]
pub fn cspp_wallets_directory() -> String {
    cove_cspp::backup_data::remote_layout::WALLETS_DIRECTORY.to_string()
}

#[uniffi::export]
pub fn cspp_wallet_file_prefix() -> String {
    cove_cspp::backup_data::WALLET_FILE_PREFIX.to_string()
}

#[uniffi::export]
pub fn cspp_namespaces_subdirectory() -> String {
    cove_cspp::backup_data::NAMESPACES_SUBDIRECTORY.to_string()
}

pub(super) const LIVE_UPLOAD_DEBOUNCE: Duration = Duration::from_secs(5);
const MAX_LIVE_UPLOAD_RETRY_DELAY: Duration = Duration::from_secs(60);

fn live_upload_retry_delay_for_attempt(retry_count: u32) -> Duration {
    let backoff_multiplier = 1u64 << retry_count.min(4);
    let delay_secs = LIVE_UPLOAD_DEBOUNCE
        .as_secs()
        .saturating_mul(backoff_multiplier)
        .min(MAX_LIVE_UPLOAD_RETRY_DELAY.as_secs());
    Duration::from_secs(delay_secs)
}

fn wipe_wallet_keychain_items_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    let keychain = Keychain::global();
    let wallet_ids = catastrophic_wipe_wallet_ids(
        persisted_wallet_ids_for_catastrophic_wipe(),
        &cove_common::consts::WALLET_DATA_DIR,
    );
    let mut failed_wallet_ids = Vec::new();

    for wallet_id in wallet_ids {
        if !keychain.delete_wallet_items(&wallet_id) {
            failed_wallet_ids.push(wallet_id.to_string());
        }
    }

    if failed_wallet_ids.is_empty() {
        return Ok(());
    }

    let failed_wallet_ids = failed_wallet_ids.join(", ");
    error!("Failed to delete wallet keychain items for: {failed_wallet_ids}");
    Err(CatastrophicRecoveryError::Failure(format!(
        "failed to delete wallet keychain items for: {failed_wallet_ids}"
    )))
}

fn persisted_wallet_ids_for_catastrophic_wipe() -> Option<Vec<WalletId>> {
    let Some(db_swap) = crate::database::DATABASE.get() else {
        warn!("Database not initialized, deriving wipe wallet ids from wallet data dir");
        return None;
    };

    let db = db_swap.load();
    match CloudBackupStore::new(&db).all_wallets() {
        Ok(wallets) => Some(wallets.into_iter().map(|wallet| wallet.id).collect()),
        Err(error) => {
            warn!(
                "Failed to read wallet ids for catastrophic recovery, deriving from wallet data dir: {error}"
            );
            None
        }
    }
}

fn catastrophic_wipe_wallet_ids(
    persisted_wallet_ids: Option<Vec<WalletId>>,
    wallet_data_dir: &Path,
) -> Vec<WalletId> {
    if let Some(wallet_ids) = persisted_wallet_ids {
        return wallet_ids;
    }

    wallet_ids_from_wallet_data_dir(wallet_data_dir)
}

fn wallet_ids_from_wallet_data_dir(wallet_data_dir: &Path) -> Vec<WalletId> {
    let mut wallet_ids = std::collections::BTreeSet::new();
    let entries = match std::fs::read_dir(wallet_data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!("Failed to read wallet data dir during catastrophic wipe: {error}");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(wallet_id) = file_name.to_str() else {
            continue;
        };
        wallet_ids.insert(wallet_id.to_owned());
    }

    wallet_ids.into_iter().map(WalletId::from).collect()
}

fn sync_health_failed_message(
    sync_state: &PersistedCloudBlobSyncState,
    failed_state: &crate::database::cloud_backup::CloudBlobFailedState,
) -> String {
    if failed_state.error.is_empty() {
        return format!("cloud backup upload failed for record_id={}", sync_state.record_id());
    }

    failed_state.error.clone()
}

fn sync_health_missing_wallet_message(missing_wallet_count: usize) -> String {
    if missing_wallet_count == 1 {
        return "1 wallet backup is missing from cloud storage".into();
    }

    format!("{missing_wallet_count} wallet backups are missing from cloud storage")
}

pub(crate) async fn current_namespace_wallet_record_ids(
    cloud: &CloudStorageClient,
    current_namespace: &str,
    step: BlockingCloudStep,
) -> Result<Vec<String>, CloudBackupError> {
    match cloud.list_wallet_backups(current_namespace.to_owned()).await {
        Ok(record_ids) => Ok(record_ids),
        Err(error) => Err(blocking_cloud_error(
            step,
            CloudBackupError::cloud_storage_context("list wallet backups", error),
        )),
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
    use tempfile::TempDir;

    fn init_manager() -> Arc<RustCloudBackupManager> {
        ensure_cloud_backup_test_tokio_runtime();
        test_globals().reset();
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        RustCloudBackupManager::init()
    }

    fn cloud_backup_wallet_item(record_id: &str) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: record_id.into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: None,
            sync_status: CloudBackupWalletStatus::DeletedFromDevice,
            record_id: record_id.into(),
        }
    }

    fn cloud_backup_detail(cloud_only_count: u32) -> CloudBackupDetail {
        CloudBackupDetail {
            last_sync: None,
            up_to_date: Vec::new(),
            needs_sync: Vec::new(),
            cloud_only_count,
            other_backups: CloudBackupOtherBackupsState::Loaded { summary: Default::default() },
        }
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
    fn detail_refresh_resets_empty_cloud_only_cache_when_remote_count_increases() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager
            .apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(Vec::new()));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(cloud_backup_detail(1)));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_loaded_cloud_only_cache_when_remote_count_drops_to_zero() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(vec![
            cloud_backup_wallet_item("wallet-1"),
        ]));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(cloud_backup_detail(0)));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_cloud_only_cache_when_loaded_wallet_is_now_local() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-1");
        let mut detail = cloud_backup_detail(1);
        detail.up_to_date.push(wallet.clone());

        manager
            .apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(vec![wallet]));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
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
    fn catastrophic_cloud_restore_check_result_reports_backup_found() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Ok(true),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::BackupFound
        );
    }

    #[test]
    fn catastrophic_cloud_restore_check_result_reports_no_backup_found() {
        assert!(matches!(
            catastrophic_cloud_restore_check_result(
                Ok(false),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::NoBackupFound { .. }
        ));
    }

    #[test]
    fn catastrophic_cloud_restore_error_requires_access_for_blank_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired(" ".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "iCloud access is required before local data can be reset.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_preserves_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired("sign in before continuing".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "sign in before continuing".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_offline_state() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::Offline("offline".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Offline {
                message: "Cannot check Google Drive while offline: offline".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_treats_not_found_as_no_backup() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::NotFound("namespace".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::NoBackupFound {
                message: "No Cloud Backup was found for the selected iCloud account.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_unreadable_download_failure() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::DownloadFailed("bad json".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Unreadable {
                message: "Cloud Backup data could not be read: bad json".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_unreadable_invalid_namespace() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::InvalidNamespace("bad namespace".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Unreadable {
                message: "Cloud Backup data could not be read.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_quota_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::QuotaExceeded),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "iCloud quota is exceeded. Cove could not check for a Cloud Backup."
                    .into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_provider_unavailable_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::NotAvailable("service unavailable".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "Google Drive is unavailable: service unavailable".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_upload_failure_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::UploadFailed("upload failed".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive { message: "upload failed".into() }
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
    fn sync_health_from_local_failures_prefers_authorization_required() {
        let generic_failure = PersistedCloudBlobSyncState::wallet(
            "namespace".into(),
            "generic".into(),
            "generic".into(),
            PersistedCloudBlobState::Failed(crate::database::cloud_backup::CloudBlobFailedState {
                revision_hash: None,
                retryable: true,
                error: "generic failure".into(),
                issue: None,
                failed_at: 1,
            }),
        );
        let authorization_failure = PersistedCloudBlobSyncState::wallet(
            "namespace".into(),
            "authorization".into(),
            "authorization".into(),
            PersistedCloudBlobState::Failed(crate::database::cloud_backup::CloudBlobFailedState {
                revision_hash: None,
                retryable: true,
                error: "authorization required".into(),
                issue: Some(CloudBlobFailureIssue::AuthorizationRequired),
                failed_at: 2,
            }),
        );

        assert_eq!(
            RustCloudBackupManager::sync_health_from_local_failures(&[
                generic_failure,
                authorization_failure
            ]),
            Some(CloudSyncHealth::AuthorizationRequired("authorization required".into())),
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
    fn live_upload_retry_delay_increases_with_attempts() {
        assert_eq!(live_upload_retry_delay_for_attempt(0), Duration::from_secs(5));
        assert_eq!(live_upload_retry_delay_for_attempt(1), Duration::from_secs(10));
        assert_eq!(live_upload_retry_delay_for_attempt(2), Duration::from_secs(20));
        assert_eq!(live_upload_retry_delay_for_attempt(3), Duration::from_secs(40));
    }

    #[test]
    fn live_upload_retry_delay_caps_at_maximum() {
        assert_eq!(live_upload_retry_delay_for_attempt(4), MAX_LIVE_UPLOAD_RETRY_DELAY);
        assert_eq!(live_upload_retry_delay_for_attempt(10), MAX_LIVE_UPLOAD_RETRY_DELAY);
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
    fn platform_authorization_failure_uses_temporary_reader_message() {
        let error = CloudBackupError::Passkey(PasskeyError::RequestFailed {
            operation: PasskeyOperation::DiscoverAssertion,
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
        });

        assert_eq!(error.reader_message(), PASSKEY_TEMPORARILY_UNAVAILABLE_MESSAGE);
        assert_eq!(
            RustCloudBackupManager::status_for_operation_error(&error),
            CloudBackupStatus::Error(PASSKEY_TEMPORARILY_UNAVAILABLE_MESSAGE.into())
        );
    }

    #[test]
    fn raw_passkey_diagnostic_cannot_reach_reader_message() {
        let diagnostic = "Q8UP8C53Y8.org.bitcoinppl.cove webcredentials:covebitcoinwallet.com could not be validated";
        let error = CloudBackupError::Passkey(PasskeyError::RequestFailed {
            operation: PasskeyOperation::DiscoverAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: diagnostic.into() },
        });
        let message = error.reader_message();

        assert_eq!(message, GENERIC_PASSKEY_ERROR_MESSAGE);
        for marker in
            ["Q8UP8C53Y8", "org.bitcoinppl.cove", "covebitcoinwallet.com", "webcredentials"]
        {
            assert!(!message.contains(marker));
        }
    }

    #[test]
    fn reader_message_keeps_distinct_passkey_outcomes() {
        assert_ne!(
            CloudBackupError::PasskeyDiscoveryCancelled.reader_message(),
            CloudBackupError::PasskeyMismatch.reader_message()
        );
        assert_ne!(
            CloudBackupError::PasskeyMismatch.reader_message(),
            CloudBackupError::UnsupportedPasskeyProvider.reader_message()
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

    #[test]
    fn catastrophic_wipe_wallet_ids_prefers_persisted_wallet_ids() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-from-dir")).unwrap();

        let wallet_ids = catastrophic_wipe_wallet_ids(
            Some(vec![WalletId::from("wallet-from-db".to_string())]),
            dir.path(),
        );

        assert_eq!(wallet_ids, vec![WalletId::from("wallet-from-db".to_string())]);
    }

    #[test]
    fn catastrophic_wipe_wallet_ids_falls_back_to_wallet_data_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-from-dir")).unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-two")).unwrap();

        let wallet_ids = catastrophic_wipe_wallet_ids(None, dir.path());

        assert_eq!(
            wallet_ids,
            vec![
                WalletId::from("wallet-from-dir".to_string()),
                WalletId::from("wallet-two".to_string()),
            ]
        );
    }

    #[test]
    fn wallet_ids_from_wallet_data_dir_uses_directory_names() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("AbCd123")).unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-two")).unwrap();
        std::fs::write(dir.path().join("bdk_wallet_abcd123.db"), "").unwrap();

        let wallet_ids = wallet_ids_from_wallet_data_dir(dir.path());

        assert_eq!(
            wallet_ids,
            vec![WalletId::from("AbCd123".to_string()), WalletId::from("wallet-two".to_string()),],
        );
    }

    #[test]
    fn wallet_ids_from_wallet_data_dir_returns_empty_for_missing_dir() {
        let dir = TempDir::new().unwrap();
        let wallet_ids = wallet_ids_from_wallet_data_dir(&dir.path().join("missing"));

        assert!(wallet_ids.is_empty());
    }
}
