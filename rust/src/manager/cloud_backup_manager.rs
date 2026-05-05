mod cloud_inventory;
mod detail;
mod keychain;
mod ops;
mod pending;
mod prompt;
mod store;
mod verify;
mod wallets;
pub(crate) mod workers;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use act_zero::{Addr, call, send};
use cove_cspp::backup_data::{MASTER_KEY_RECORD_ID, wallet_record_id};
use cove_device::cloud_storage::{
    CloudStorage, CloudStorageClient, CloudStorageError, CloudSyncHealth,
};
use cove_tokio::task::spawn_actor;
use cove_util::ResultExt as _;
use flume::{Receiver, Sender};
use futures::TryStreamExt as _;
use futures::stream::{self, StreamExt as _};
use parking_lot::RwLock;
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

use self::cloud_inventory::RemoteWalletTruth;
pub use self::detail::{
    CloudOnlyOperation, CloudOnlyState, RecoveryAction, RecoveryState, SyncState, VerificationState,
};
pub(crate) use self::keychain::CloudBackupKeychain;
use self::prompt::CloudBackupPromptState;
pub(crate) use self::store::CloudBackupStore;
use self::wallets::wallet_metadata_change_requires_upload;
use self::wallets::{UnpersistedPrfKey, WalletBackupLookup, WalletBackupReader};
use self::workers::{CloudBackupOperation, CloudBackupSupervisor, RestoreOperation};
use super::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

type LocalWalletSecret = crate::backup::model::WalletSecret;

const PASSKEY_RP_ID: &str = "covebitcoinwallet.com";
pub(crate) const SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE: &str =
    "master key backup is missing from cloud storage";
pub(super) const CLOUD_BACKUP_IO_CONCURRENCY: usize = 4;
type Message = CloudBackupReconcileMessage;

pub static CLOUD_BACKUP_MANAGER: LazyLock<Arc<RustCloudBackupManager>> =
    LazyLock::new(RustCloudBackupManager::init);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupStatus {
    Disabled,
    Enabling,
    Restoring,
    Enabled,
    PasskeyMissing,
    UnsupportedPasskeyProvider,
    Error(String),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupPasskeyChoiceFlow {
    Enable,
    RepairPasskey,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupPromptIntent {
    None,
    ExistingBackupFound,
    PasskeyChoice(CloudBackupPasskeyChoiceFlow),
    MissingPasskeyReminder,
    VerificationPrompt,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupManagerAction {
    EnableCloudBackup,
    EnableCloudBackupForceNew,
    EnableCloudBackupNoDiscovery,
    DiscardPendingEnableCloudBackup,
    DismissPasskeyChoicePrompt,
    DismissMissingPasskeyReminder,
    RestoreFromCloudBackup,
    CancelRestore,
    StartVerification,
    StartVerificationDiscoverable,
    DismissVerificationPrompt,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
    RepairPasskeyNoDiscovery,
    SyncUnsynced,
    FetchCloudOnly,
    RestoreCloudWallet { record_id: String },
    DeleteCloudWallet { record_id: String },
    RecoverOtherBackups,
    DeleteOtherBackups,
    RefreshDetail,
    EnterDetail,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupReconcileMessage {
    Status(CloudBackupStatus),
    SyncHealth(CloudSyncHealth),
    Progress(Option<CloudBackupProgress>),
    RestoreProgress(Option<CloudBackupRestoreProgress>),
    RestoreReport(Option<CloudBackupRestoreReport>),
    SyncError(Option<String>),
    VerificationPrompt(bool),
    VerificationMetadata(CloudBackupVerificationMetadata),
    PendingUploadVerification(bool),
    Detail(Option<CloudBackupDetail>),
    Verification(VerificationState),
    Sync(SyncState),
    Recovery(RecoveryState),
    CloudOnly(CloudOnlyState),
    CloudOnlyOperation(CloudOnlyOperation),
    OtherBackupsOperation(OtherBackupsOperation),
    PromptIntent(CloudBackupPromptIntent),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRestoreReport {
    pub wallets_restored: u32,
    pub wallets_failed: u32,
    pub failed_wallet_errors: Vec<String>,
    pub labels_failed_wallet_names: Vec<String>,
    pub labels_failed_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupProgress {
    pub completed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRestoreStage {
    Finding,
    Downloading,
    Restoring,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupRestoreProgress {
    pub stage: CloudBackupRestoreStage,
    pub completed: u32,
    pub total: Option<u32>,
}

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

#[derive(Debug)]
pub enum CloudBackupDetailResult {
    Success(CloudBackupDetail),
    AccessError(CloudBackupError),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDetail {
    pub last_sync: Option<u64>,
    pub up_to_date: Vec<CloudBackupWalletItem>,
    pub needs_sync: Vec<CloudBackupWalletItem>,
    /// Number of wallets in the cloud that aren't on this device
    pub cloud_only_count: u32,
    pub other_backups: CloudBackupOtherBackupsSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct CloudBackupOtherBackupsSummary {
    pub namespace_count: u32,
    pub wallet_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OtherBackupsOperation {
    Idle,
    Recovering,
    Recovered { wallets_restored: u32, wallets_failed: u32, failed_wallet_errors: Vec<String> },
    Deleting,
    Deleted,
    Failed { error: String },
}

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

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupVerificationMetadata {
    NotConfigured,
    ConfiguredNeverVerified,
    Verified(u64),
    NeedsVerification,
}

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

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRetryIssue {
    Connectivity,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupRetryAction {
    Verify,
    VerifyDiscoverable,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudBackupRetryContext {
    pub issue: CloudBackupRetryIssue,
    pub action: CloudBackupRetryAction,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudBackupState {
    pub status: CloudBackupStatus,
    pub sync_health: CloudSyncHealth,
    pub prompt_intent: CloudBackupPromptIntent,
    pub progress: Option<CloudBackupProgress>,
    pub restore_progress: Option<CloudBackupRestoreProgress>,
    pub restore_report: Option<CloudBackupRestoreReport>,
    pub sync_error: Option<String>,
    pub has_pending_upload_verification: bool,
    pub should_prompt_verification: bool,
    pub verification_metadata: CloudBackupVerificationMetadata,
    pub detail: Option<CloudBackupDetail>,
    pub verification: VerificationState,
    pub sync: SyncState,
    pub recovery: RecoveryState,
    pub cloud_only: CloudOnlyState,
    pub cloud_only_operation: CloudOnlyOperation,
    pub other_backups_operation: OtherBackupsOperation,
}

impl Default for CloudBackupState {
    fn default() -> Self {
        Self {
            status: CloudBackupStatus::Disabled,
            sync_health: CloudSyncHealth::Unknown,
            prompt_intent: CloudBackupPromptIntent::None,
            progress: None,
            restore_progress: None,
            restore_report: None,
            sync_error: None,
            has_pending_upload_verification: false,
            should_prompt_verification: false,
            verification_metadata: CloudBackupVerificationMetadata::NotConfigured,
            detail: None,
            verification: VerificationState::Idle,
            sync: SyncState::Idle,
            recovery: RecoveryState::Idle,
            cloud_only: CloudOnlyState::NotFetched,
            cloud_only_operation: CloudOnlyOperation::Idle,
            other_backups_operation: OtherBackupsOperation::Idle,
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

pub(crate) fn is_connectivity_related_issue(issue: CloudStorageIssue) -> bool {
    matches!(issue, CloudStorageIssue::Offline | CloudStorageIssue::Unavailable)
}

pub(crate) fn blocking_cloud_error(
    step: BlockingCloudStep,
    error: CloudBackupError,
) -> CloudBackupError {
    if is_connectivity_related_issue(error.cloud_storage_issue()) {
        return offline_error_for_step(step);
    }

    error
}

impl From<CloudBackupError> for CloudStorageIssue {
    fn from(error: CloudBackupError) -> Self {
        error.cloud_storage_issue()
    }
}

impl CloudBackupError {
    pub(crate) fn cloud_storage_issue(&self) -> CloudStorageIssue {
        match self {
            CloudBackupError::Offline(_) | CloudBackupError::Deferred(_) => {
                CloudStorageIssue::Offline
            }
            CloudBackupError::CloudStorage(error) => error.into(),
            CloudBackupError::CloudStorageContext { source, .. } => source.into(),
            CloudBackupError::Cloud(_) => CloudStorageIssue::Other,
            CloudBackupError::NotSupported(_)
            | CloudBackupError::UnsupportedPasskeyProvider
            | CloudBackupError::RecoveryRequired(_)
            | CloudBackupError::Passkey(_)
            | CloudBackupError::Crypto(_)
            | CloudBackupError::Internal(_)
            | CloudBackupError::Compatibility(_)
            | CloudBackupError::PasskeyMismatch
            | CloudBackupError::PasskeyDiscoveryCancelled
            | CloudBackupError::Cancelled => CloudStorageIssue::Other,
        }
    }
}

impl From<CloudStorageError> for CloudStorageIssue {
    fn from(error: CloudStorageError) -> Self {
        error.cloud_storage_issue()
    }
}

impl From<&CloudStorageError> for CloudStorageIssue {
    fn from(error: &CloudStorageError) -> Self {
        error.cloud_storage_issue()
    }
}

pub(crate) trait CloudStorageErrorIssueExt {
    fn cloud_storage_issue(&self) -> CloudStorageIssue;
}

impl From<&PersistedCloudBackupState> for CloudBackupVerificationMetadata {
    fn from(db_state: &PersistedCloudBackupState) -> Self {
        if db_state.is_unverified() {
            return Self::NeedsVerification;
        }

        if !db_state.is_configured() {
            return Self::NotConfigured;
        }

        match db_state.last_verified_at {
            Some(last_verified_at) => Self::Verified(last_verified_at),
            None => Self::ConfiguredNeverVerified,
        }
    }
}

impl CloudStorageErrorIssueExt for CloudStorageError {
    fn cloud_storage_issue(&self) -> CloudStorageIssue {
        match self {
            CloudStorageError::AuthorizationRequired(_) => CloudStorageIssue::AuthorizationRequired,
            CloudStorageError::Offline(_) => CloudStorageIssue::Offline,
            CloudStorageError::NotAvailable(_) => CloudStorageIssue::Unavailable,
            CloudStorageError::NotFound(_) => CloudStorageIssue::NotFound,
            CloudStorageError::QuotaExceeded => CloudStorageIssue::QuotaExceeded,
            CloudStorageError::UploadFailed(_) | CloudStorageError::DownloadFailed(_) => {
                CloudStorageIssue::Other
            }
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
    RecreateManifest,
    RepairPasskey,
    DetailRefresh,
}

pub(crate) struct PendingEnableSessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<UnpersistedPrfKey>,
}

/// Tracks passkey material created during enable before the flow fully completes
pub(crate) enum PendingEnableSession {
    /// A new passkey and master key are staged while the user confirms Create New Backup
    AwaitingForceNewConfirmation(PendingEnableSessionMaterial),
    /// Upload already started and should retry with the same staged passkey material
    RetryUpload(PendingEnableSessionMaterial),
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
    fn new(master_key: cove_cspp::master_key::MasterKey, passkey: UnpersistedPrfKey) -> Self {
        Self { master_key: Zeroizing::new(master_key), passkey: Zeroizing::new(passkey) }
    }

    fn into_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>) {
        (self.master_key, self.passkey)
    }

    fn namespace_id(&self) -> String {
        self.master_key.namespace_id()
    }
}

impl PendingEnableSession {
    fn awaiting_confirmation(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
    ) -> Self {
        Self::AwaitingForceNewConfirmation(PendingEnableSessionMaterial::new(master_key, passkey))
    }

    fn retry_upload(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
    ) -> Self {
        Self::RetryUpload(PendingEnableSessionMaterial::new(master_key, passkey))
    }

    fn into_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>) {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.into_parts()
            }
        }
    }

    fn namespace_id(&self) -> String {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.namespace_id()
            }
        }
    }

    fn is_retry_upload(&self) -> bool {
        matches!(self, Self::RetryUpload(_))
    }

    fn is_awaiting_force_new_confirmation(&self) -> bool {
        matches!(self, Self::AwaitingForceNewConfirmation(_))
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
        matches!(self, Self::AccessError(error) if is_connectivity_related_issue(error.cloud_storage_issue()))
    }
}

impl PendingVerificationCompletion {
    fn new(
        report: DeepVerificationReport,
        namespace_id: String,
        uploads: Vec<PendingVerificationUpload>,
    ) -> Self {
        Self { report, namespace_id, uploads }
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

    fn persisted(&self) -> PersistedPendingVerificationCompletion {
        PersistedPendingVerificationCompletion {
            report: PersistedDeepVerificationReport::from(&self.report),
            namespace_id: self.namespace_id.clone(),
            uploads: self.uploads.iter().map(PersistedPendingVerificationUpload::from).collect(),
        }
    }

    fn from_persisted(completion: PersistedPendingVerificationCompletion) -> Self {
        Self {
            report: DeepVerificationReport::from(completion.report),
            namespace_id: completion.namespace_id,
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

impl From<&PendingVerificationUpload> for PersistedPendingVerificationUpload {
    fn from(upload: &PendingVerificationUpload) -> Self {
        match upload {
            PendingVerificationUpload::MasterKeyWrapper => Self::MasterKeyWrapper,
            PendingVerificationUpload::Wallet { record_id, expected_revision } => Self::Wallet {
                record_id: record_id.clone(),
                expected_revision: expected_revision.clone(),
            },
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
    Passkey(String),

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
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CatastrophicRecoveryError {
    #[error("{0}")]
    Failure(String),
}

#[uniffi::export(callback_interface)]
pub trait CloudBackupManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: CloudBackupReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustCloudBackupManager {
    pub state: Arc<RwLock<CloudBackupState>>,
    pub reconciler: Sender<Message>,
    pub reconcile_receiver: Arc<Receiver<Message>>,
    prompt_state: Arc<parking_lot::Mutex<CloudBackupPromptState>>,
    cloud_only_detail_snapshot: Arc<RwLock<Option<CloudBackupDetail>>>,
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
        match state.status {
            PersistedCloudBackupStatus::Disabled => CloudBackupStatus::Disabled,
            PersistedCloudBackupStatus::Enabled | PersistedCloudBackupStatus::Unverified => {
                CloudBackupStatus::Enabled
            }
            PersistedCloudBackupStatus::PasskeyMissing => CloudBackupStatus::PasskeyMissing,
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

        let manager = Arc::new_cyclic(|manager| Self {
            state: Arc::new(RwLock::new(CloudBackupState::default())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
            prompt_state: Arc::new(parking_lot::Mutex::new(CloudBackupPromptState::default())),
            cloud_only_detail_snapshot: Arc::new(RwLock::new(None)),
            supervisor: spawn_actor(CloudBackupSupervisor::new(manager.clone())),
        });

        manager.start_connectivity_listener();
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

    fn set_and_notify_field<T>(
        &self,
        value: T,
        field: impl FnOnce(&mut CloudBackupState) -> &mut T,
        notify: fn(T) -> Message,
    ) where
        T: PartialEq + Clone,
    {
        {
            let mut state = self.state.write();
            let slot = field(&mut state);
            if *slot == value {
                return;
            }

            *slot = value.clone();
        }

        self.send(notify(value));
    }

    pub(crate) fn set_status(&self, status: CloudBackupStatus) {
        if !matches!(status, CloudBackupStatus::Enabled | CloudBackupStatus::Enabling) {
            self.clear_runtime_passkey_authorization();
        }

        let status_changed = {
            let mut state = self.state.write();
            if state.status == status {
                false
            } else {
                state.status = status.clone();
                true
            }
        };

        if !status_changed {
            return;
        }

        self.prompt_state.lock().clear_missing_passkey_dismissal();

        self.send(Message::Status(status));
        self.refresh_prompt_intent();
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

        self.clear_sync_error_if_no_failed_wallet_uploads();
        send!(self.supervisor.resume_wallet_uploads_from_persisted_state());
        send!(self.supervisor.wake_pending_upload_verifier());
        self.start_pending_upload_verification_loop();
        self.resume_failed_connectivity_verification();
    }

    fn resume_failed_connectivity_verification(&self) {
        let retry_action = {
            let state = self.state.read();
            match &state.verification {
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

    pub(crate) fn set_sync_health(&self, sync_health: CloudSyncHealth) {
        self.set_and_notify_field(sync_health, |state| &mut state.sync_health, Message::SyncHealth);
    }

    pub(crate) fn set_prompt_intent(&self, prompt_intent: CloudBackupPromptIntent) {
        self.set_and_notify_field(
            prompt_intent,
            |state| &mut state.prompt_intent,
            Message::PromptIntent,
        );
    }

    pub(crate) fn refresh_prompt_intent(&self) {
        let prompt_intent = {
            let prompt_state = self.prompt_state.lock().clone();
            let state = self.state.read().clone();
            prompt_state.resolve(&state)
        };

        self.set_prompt_intent(prompt_intent);
    }

    pub(crate) fn set_existing_backup_found_prompt(&self) {
        self.prompt_state.lock().set_existing_backup_found();
        self.refresh_prompt_intent();
    }

    pub(crate) fn clear_existing_backup_found_prompt(&self) {
        self.prompt_state.lock().clear_existing_backup_found();
        self.refresh_prompt_intent();
    }

    pub(crate) fn set_passkey_choice_prompt(&self, flow: CloudBackupPasskeyChoiceFlow) {
        self.prompt_state.lock().set_passkey_choice(flow);
        self.refresh_prompt_intent();
    }

    pub(crate) fn clear_passkey_choice_prompt(&self) {
        self.prompt_state.lock().clear_passkey_choice();
        self.refresh_prompt_intent();
    }

    pub(crate) fn dismiss_missing_passkey_prompt(&self) {
        self.prompt_state.lock().dismiss_missing_passkey();
        self.refresh_prompt_intent();
    }

    pub(crate) fn set_progress(&self, progress: Option<CloudBackupProgress>) {
        self.set_and_notify_field(progress, |state| &mut state.progress, Message::Progress);
    }

    pub(crate) fn set_restore_progress(&self, progress: Option<CloudBackupRestoreProgress>) {
        self.set_and_notify_field(
            progress,
            |state| &mut state.restore_progress,
            Message::RestoreProgress,
        );
    }

    pub(crate) fn set_restore_report(&self, report: Option<CloudBackupRestoreReport>) {
        self.set_and_notify_field(
            report,
            |state| &mut state.restore_report,
            Message::RestoreReport,
        );
    }

    pub(crate) fn set_sync_error(&self, sync_error: Option<String>) {
        self.set_and_notify_field(sync_error, |state| &mut state.sync_error, Message::SyncError);
    }

    pub(crate) fn refresh_sync_health(&self) {
        send!(self.supervisor.request_sync_health_refresh());
    }

    pub(crate) fn refresh_persisted_flags(&self) {
        let (verification_metadata, should_prompt_verification) = Self::load_persisted_flags();

        let (metadata_changed, prompt_changed) = {
            let mut state = self.state.write();

            let metadata_changed = state.verification_metadata != verification_metadata;
            if metadata_changed {
                state.verification_metadata = verification_metadata.clone();
            }

            let prompt_changed = state.should_prompt_verification != should_prompt_verification;
            if prompt_changed {
                state.should_prompt_verification = should_prompt_verification;
            }

            (metadata_changed, prompt_changed)
        };

        if metadata_changed {
            self.send(Message::VerificationMetadata(verification_metadata));
        }

        if prompt_changed {
            self.send(Message::VerificationPrompt(should_prompt_verification));
        }

        self.refresh_prompt_intent();
    }

    pub(crate) fn set_pending_upload_verification(&self, pending: bool) {
        self.set_and_notify_field(
            pending,
            |state| &mut state.has_pending_upload_verification,
            Message::PendingUploadVerification,
        );
        self.refresh_prompt_intent();
    }

    pub(crate) fn set_detail(&self, detail: Option<CloudBackupDetail>) {
        let detail_snapshot = self.cloud_only_detail_snapshot.read().clone();
        let (detail_changed, reset_cloud_only) = {
            let mut state = self.state.write();

            let detail_changed = state.detail != detail;
            if detail_changed {
                state.detail.clone_from(&detail);
            }

            let reset_cloud_only = detail.as_ref().is_some_and(|detail| {
                cloud_only_cache_is_stale(&state.cloud_only, detail, detail_snapshot.as_ref())
            });

            if reset_cloud_only {
                state.cloud_only = CloudOnlyState::NotFetched;
            }

            (detail_changed, reset_cloud_only)
        };

        if reset_cloud_only {
            *self.cloud_only_detail_snapshot.write() = None;
            self.send(Message::CloudOnly(CloudOnlyState::NotFetched));
        }

        if detail_changed {
            self.send(Message::Detail(detail));
        }
    }

    pub(crate) fn set_verification(&self, verification: VerificationState) {
        if matches!(
            verification,
            VerificationState::Idle | VerificationState::Failed(_) | VerificationState::Cancelled
        ) {
            self.clear_runtime_passkey_authorization();
        }

        self.set_and_notify_field(
            verification,
            |state| &mut state.verification,
            Message::Verification,
        );
        self.refresh_prompt_intent();
    }

    pub(crate) fn set_sync(&self, sync: SyncState) {
        self.set_and_notify_field(sync, |state| &mut state.sync, Message::Sync);
    }

    pub(crate) fn set_recovery(&self, recovery: RecoveryState) {
        if !matches!(recovery, RecoveryState::Idle) {
            self.clear_runtime_passkey_authorization();
        }

        self.set_and_notify_field(recovery, |state| &mut state.recovery, Message::Recovery);
        self.refresh_prompt_intent();
    }

    pub(crate) async fn record_runtime_passkey_authorization(
        &self,
        namespace_id: String,
        credential_id: Vec<u8>,
        prf_salt: [u8; 32],
    ) -> Result<(), CloudBackupError> {
        call!(self.supervisor.record_runtime_passkey_authorization(
            namespace_id,
            credential_id,
            prf_salt
        ))
        .await
        .map_err_str(CloudBackupError::Internal)
    }

    pub(crate) fn clear_runtime_passkey_authorization(&self) {
        send!(self.supervisor.clear_runtime_passkey_authorization());
    }

    pub(crate) fn set_cloud_only(&self, cloud_only: CloudOnlyState) {
        if !matches!(cloud_only, CloudOnlyState::Loaded { .. }) {
            *self.cloud_only_detail_snapshot.write() = None;
        }
        self.set_and_notify_field(cloud_only, |state| &mut state.cloud_only, Message::CloudOnly);
    }

    pub(crate) fn set_loaded_cloud_only(&self, wallets: Vec<CloudBackupWalletItem>) {
        let detail = self.state.read().detail.clone();
        *self.cloud_only_detail_snapshot.write() = detail;
        self.set_and_notify_field(
            CloudOnlyState::Loaded { wallets },
            |state| &mut state.cloud_only,
            Message::CloudOnly,
        );
    }

    pub(crate) fn set_cloud_only_operation(&self, cloud_only_operation: CloudOnlyOperation) {
        self.set_and_notify_field(
            cloud_only_operation,
            |state| &mut state.cloud_only_operation,
            Message::CloudOnlyOperation,
        );
    }

    pub(crate) fn set_other_backups_operation(
        &self,
        other_backups_operation: OtherBackupsOperation,
    ) {
        self.set_and_notify_field(
            other_backups_operation,
            |state| &mut state.other_backups_operation,
            Message::OtherBackupsOperation,
        );
    }

    pub(crate) fn clear_in_process_state_for_local_reset(&self) {
        let supervisor = self.supervisor.clone();
        if let Err(error) = cove_tokio::task::block_on(async move {
            act_zero::call!(supervisor.clear_upload_runtime_state()).await
        }) {
            error!("Failed to clear cloud backup runtime state during local reset: {error}");
        }

        self.clear_prompt_state();
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_restore_report(None);
        self.set_sync_error(None);
        self.set_sync_health(CloudSyncHealth::Unknown);
        self.set_pending_upload_verification(false);
        self.set_detail(None);
        self.set_verification(VerificationState::Idle);
        self.set_sync(SyncState::Idle);
        self.set_recovery(RecoveryState::Idle);
        self.set_cloud_only(CloudOnlyState::NotFetched);
        self.set_cloud_only_operation(CloudOnlyOperation::Idle);
        self.set_other_backups_operation(OtherBackupsOperation::Idle);
        self.set_status(CloudBackupStatus::Disabled);
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

        self.set_status(Self::runtime_status_for(state));
        self.refresh_persisted_flags();

        Ok(())
    }

    pub(crate) async fn persist_cloud_backup_state_for_restore_operation(
        &self,
        operation: &RestoreOperation,
        state: &PersistedCloudBackupState,
        context: &str,
    ) -> Result<(), CloudBackupError> {
        operation.persist_cloud_backup_state(state.clone(), context.to_string()).await
    }

    pub(crate) async fn ensure_current_restore_operation(
        &self,
        operation: &RestoreOperation,
    ) -> Result<(), CloudBackupError> {
        operation.ensure_current().await
    }

    pub(crate) async fn set_status_for_restore_operation(
        &self,
        operation: &RestoreOperation,
        status: CloudBackupStatus,
    ) -> Result<(), CloudBackupError> {
        operation.set_status(status).await
    }

    pub(crate) async fn set_restore_progress_for_restore_operation(
        &self,
        operation: &RestoreOperation,
        progress: Option<CloudBackupRestoreProgress>,
    ) -> Result<(), CloudBackupError> {
        operation.set_progress(progress).await
    }

    pub(crate) async fn set_restore_report_for_restore_operation(
        &self,
        operation: &RestoreOperation,
        report: Option<CloudBackupRestoreReport>,
    ) -> Result<(), CloudBackupError> {
        operation.set_report(report).await
    }

    pub(crate) async fn build_cloud_backup_detail_with_remote_truth(
        &self,
        wallet_record_ids: &[String],
        remote_wallet_truth: RemoteWalletTruth,
    ) -> Result<CloudBackupDetail, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let other_backups = self.other_backup_summary(&cloud).await?;

        Ok(self::cloud_inventory::CloudWalletInventory::load_with_remote_truth(
            wallet_record_ids,
            remote_wallet_truth,
        )
        .await?
        .build_detail(other_backups))
    }

    pub(crate) async fn other_backup_summary(
        &self,
        cloud: &CloudStorageClient,
    ) -> Result<CloudBackupOtherBackupsSummary, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let current_wallet_record_ids: HashSet<_> = current_namespace_wallet_record_ids(
            cloud,
            &current_namespace,
            BlockingCloudStep::DetailRefresh,
        )
        .await?
        .into_iter()
        .collect();
        let namespaces = self
            .other_backup_namespaces(cloud, &current_namespace, BlockingCloudStep::DetailRefresh)
            .await?;

        let mut namespace_count = 0;
        let mut wallet_count = 0;

        for namespace in &namespaces {
            match cloud.list_wallet_backups(namespace.clone()).await {
                Ok(record_ids) => {
                    let unrecovered_wallet_count = record_ids
                        .iter()
                        .filter(|record_id| !current_wallet_record_ids.contains(*record_id))
                        .count() as u32;

                    if unrecovered_wallet_count > 0 {
                        namespace_count += 1;
                        wallet_count += unrecovered_wallet_count;
                    }
                }
                Err(CloudStorageError::NotFound(_)) => {}
                Err(error) => {
                    warn!("Failed to count other backup namespace {namespace}: {error}");
                }
            }
        }

        Ok(CloudBackupOtherBackupsSummary { namespace_count, wallet_count })
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
        if state.last_verification_requested_at.is_none() {
            return Ok(());
        }

        state.last_verification_dismissed_at =
            Some(jiff::Timestamp::now().as_second().try_into().unwrap_or(0));

        self.persist_cloud_backup_state(&state, "persist cloud backup prompt dismissal")
    }

    fn current_namespace_id(&self) -> Result<String, CloudBackupError> {
        CloudBackupKeychain::global()
            .namespace_id()
            .ok_or_else(|| CloudBackupError::Internal("namespace_id not found in keychain".into()))
    }

    pub(crate) async fn compute_sync_health(&self) -> CloudSyncHealth {
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
            Ok(states) => states
                .into_iter()
                .filter(|state| {
                    state.namespace_id == namespace
                        && (state.wallet_id.is_none()
                            || expected_wallet_record_ids.contains(&state.record_id))
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                return CloudSyncHealth::Failed(format!(
                    "failed to read cloud backup sync states: {error}",
                ));
            }
        };

        if let Some(sync_health) = Self::sync_health_from_local_failures(&sync_states) {
            return sync_health;
        }

        if Self::sync_health_has_pending_upload(&sync_states) {
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

    fn sync_health_has_pending_upload(sync_states: &[PersistedCloudBlobSyncState]) -> bool {
        sync_states.iter().any(|sync_state| {
            matches!(
                sync_state.state,
                PersistedCloudBlobState::Dirty(_)
                    | PersistedCloudBlobState::Uploading(_)
                    | PersistedCloudBlobState::UploadedPendingConfirmation(_)
            )
        })
    }

    fn sync_health_has_pending_wallet_upload(sync_states: &[PersistedCloudBlobSyncState]) -> bool {
        sync_states.iter().any(|sync_state| {
            sync_state.wallet_id.is_some()
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
            | CloudStorageError::NotFound(message) => CloudSyncHealth::Failed(message),
        }
    }

    pub(crate) fn mark_wallet_blob_dirty(&self, wallet_id: WalletId) {
        if !Self::load_persisted_state().is_configured() {
            return;
        }

        let Ok(namespace_id) = self.current_namespace_id() else {
            warn!("Cloud backup dirty mark skipped, namespace is unavailable");
            return;
        };

        let changed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let record_id = wallet_record_id(wallet_id.as_ref());
        let sync_state = PersistedCloudBlobSyncState {
            namespace_id,
            wallet_id: Some(wallet_id.clone()),
            record_id,
            state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
        };

        if let Err(error) = Database::global().cloud_blob_sync_states.set(&sync_state) {
            error!("Failed to persist dirty cloud backup state: {error}");
            return;
        }

        if self.is_known_offline() {
            return;
        }

        self.schedule_wallet_upload(wallet_id, false);
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
        send!(self.supervisor.schedule_wallet_upload(wallet_id, immediate));
    }

    fn downgrade_interrupted_upload_to_dirty(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
    ) -> bool {
        let changed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

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

    pub(crate) async fn replace_pending_enable_session(&self, session: PendingEnableSession) {
        act_zero::call!(self.supervisor.replace_pending_enable_session(session))
            .await
            .expect("replace pending enable session");
    }

    pub(crate) async fn take_retry_pending_enable_session(&self) -> Option<PendingEnableSession> {
        act_zero::call!(self.supervisor.take_retry_pending_enable_session())
            .await
            .expect("take retry pending enable session")
    }

    pub(crate) async fn has_awaiting_force_new_pending_enable_session(&self) -> bool {
        act_zero::call!(self.supervisor.has_awaiting_force_new_pending_enable_session())
            .await
            .expect("check pending enable session")
    }

    pub(crate) async fn take_pending_enable_session(&self) -> Option<PendingEnableSession> {
        act_zero::call!(self.supervisor.take_pending_enable_session())
            .await
            .expect("take pending enable session")
    }

    pub(crate) fn clear_pending_enable_session(&self) {
        send!(self.supervisor.clear_pending_enable_session());
    }

    pub(crate) fn replace_pending_verification_completion(
        &self,
        completion: PendingVerificationCompletion,
    ) {
        let persisted_completion = completion.persisted();

        let mut state = Self::load_persisted_state();
        state.pending_verification_completion = Some(persisted_completion);
        if let Err(error) =
            self.persist_cloud_backup_state(&state, "persist pending verification completion")
        {
            error!("Failed to persist pending verification completion: {error}");
            return;
        }

        send!(self.supervisor.cache_pending_verification_completion(completion));
    }

    pub(crate) fn pending_verification_completion(&self) -> Option<PendingVerificationCompletion> {
        Self::load_persisted_state()
            .pending_verification_completion
            .map(PendingVerificationCompletion::from_persisted)
    }

    pub(crate) fn clear_pending_verification_completion(&self) {
        let mut state = Self::load_persisted_state();
        if state.pending_verification_completion.is_none() {
            send!(self.supervisor.clear_pending_verification_completion());
            return;
        }

        state.pending_verification_completion = None;
        if let Err(error) =
            self.persist_cloud_backup_state(&state, "clear pending verification completion")
        {
            error!("Failed to clear pending verification completion: {error}");
            return;
        }

        send!(self.supervisor.clear_pending_verification_completion());
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

    pub(super) fn begin_background_operation(
        &self,
        operation_name: &str,
        entering_status: Option<CloudBackupStatus>,
    ) -> bool {
        if let Some(status) = entering_status.clone() {
            let (
                progress_changed,
                restore_progress_changed,
                restore_report_changed,
                status_changed,
            ) = {
                let mut state = self.state.write();
                let current_status = state.status.clone();
                if matches!(
                    current_status,
                    CloudBackupStatus::Enabling | CloudBackupStatus::Restoring
                ) {
                    warn!("{operation_name} called while {current_status:?}, ignoring");
                    return false;
                }

                let progress_changed = state.progress.take().is_some();
                let restore_progress_changed = state.restore_progress.take().is_some();
                let restore_report_changed =
                    matches!(status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring)
                        && state.restore_report.take().is_some();
                let status_changed = state.status != status;
                if status_changed {
                    state.status = status.clone();
                }

                (progress_changed, restore_progress_changed, restore_report_changed, status_changed)
            };

            if progress_changed {
                self.send(Message::Progress(None));
            }
            if restore_progress_changed {
                self.send(Message::RestoreProgress(None));
            }
            if restore_report_changed {
                self.send(Message::RestoreReport(None));
            }
            if status_changed {
                self.send(Message::Status(status));
            }
        } else {
            let status = self.state.read().status.clone();
            if matches!(status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring) {
                warn!("{operation_name} called while {status:?}, ignoring");
                return false;
            }
        }

        self.refresh_prompt_intent();
        true
    }

    pub(super) fn finish_background_operation_error(&self, error: &CloudBackupError) {
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_status(Self::status_for_operation_error(error));
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

    pub fn current_status(&self) -> CloudBackupStatus {
        self.state.read().status.clone()
    }

    pub fn state(&self) -> CloudBackupState {
        let db_state = Self::load_persisted_state();
        let mut state = self.state.read().clone();
        state.status = Self::runtime_status_for(&db_state);
        state.verification_metadata = CloudBackupVerificationMetadata::from(&db_state);
        state.should_prompt_verification = db_state.should_prompt_verification();
        state.has_pending_upload_verification = self.has_pending_cloud_upload_verification();
        state.prompt_intent = self.prompt_state.lock().resolve(&state);
        state
    }

    /// Number of wallets in the cloud backup
    pub fn backup_wallet_count(&self) -> Option<u32> {
        let db = Database::global();
        let current = Self::load_persisted_state();

        match current.wallet_count {
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
        self.set_status(Self::runtime_status_for(&db_state));
        self.refresh_persisted_flags();
        self.set_pending_upload_verification(self.has_pending_cloud_upload_verification());
    }

    pub fn cloud_storage_did_change(&self) {
        self.clear_sync_error_if_no_failed_wallet_uploads();
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
        if Self::load_persisted_state().pending_verification_completion.is_some() {
            return true;
        }

        Database::global().cloud_blob_sync_states.list().ok().is_some_and(|states| {
            states.into_iter().any(|state| state.is_uploaded_pending_confirmation())
        })
    }

    pub(super) fn clear_sync_error_if_no_failed_wallet_uploads(&self) {
        if self.has_failed_wallet_uploads() {
            return;
        }

        self.set_sync_error(None);
    }

    fn has_failed_wallet_uploads(&self) -> bool {
        match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => states
                .into_iter()
                .any(|state| matches!(state.state, PersistedCloudBlobState::Failed(_))),
            Err(error) => {
                error!("Failed to read cloud blob sync states while clearing sync error: {error}");
                true
            }
        }
    }

    pub fn resume_pending_cloud_upload_verification(&self) {
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
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_restore_report(None);
        self.set_sync_error(None);
        self.refresh_persisted_flags();
        self.set_pending_upload_verification(false);
        self.set_detail(None);
        self.set_verification(VerificationState::Idle);
        self.set_sync(SyncState::Idle);
        self.set_recovery(RecoveryState::Idle);
        self.set_cloud_only(CloudOnlyState::NotFetched);
        self.set_cloud_only_operation(CloudOnlyOperation::Idle);
        self.set_other_backups_operation(OtherBackupsOperation::Idle);
        self.set_status(CloudBackupStatus::Disabled);
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
    /// Returns immediately if cloud backup isn't enabled (e.g. during restore)
    pub fn backup_new_wallet(&self, metadata: crate::wallet::metadata::WalletMetadata) {
        if !Self::load_persisted_state().is_configured() {
            return;
        }

        self.handle_wallet_backup_change_and_reverify(metadata.id);
    }
}

impl RustCloudBackupManager {
    pub(crate) fn enable_cloud_backup(&self) {
        send!(self.supervisor.start_operation(CloudBackupOperation::Enable, None));
    }

    pub(crate) fn enable_cloud_backup_force_new(&self) {
        send!(self.supervisor.start_operation(CloudBackupOperation::EnableForceNew, None));
    }

    pub(crate) fn enable_cloud_backup_no_discovery(&self) {
        send!(self.supervisor.start_operation(CloudBackupOperation::EnableNoDiscovery, None));
    }

    /// Dismiss staged enable state for the existing-backup confirmation flow
    pub(crate) fn discard_pending_enable_cloud_backup(&self) {
        send!(self.supervisor.discard_pending_enable_cloud_backup());
        self.clear_existing_backup_found_prompt();
    }

    pub(crate) fn cancel_restore(&self) {
        send!(self.supervisor.cancel_restore());
    }

    pub(crate) fn restore_from_cloud_backup(&self) {
        info!("restore_from_cloud_backup: enqueueing restore task");
        send!(self.supervisor.start_restore_from_cloud_backup());
    }

    fn clear_prompt_state(&self) {
        {
            let mut prompt_state = self.prompt_state.lock();
            prompt_state.clear_existing_backup_found();
            prompt_state.clear_passkey_choice();
            prompt_state.clear_missing_passkey_dismissal();
        }

        self.refresh_prompt_intent();
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

#[uniffi::export]
pub fn cspp_master_key_filename() -> String {
    cove_cspp::backup_data::master_key_filename()
}

#[uniffi::export]
pub fn cspp_wallet_filename_from_record_id(record_id: String) -> String {
    cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)
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

#[cfg(test)]
pub(crate) fn cloud_backup_test_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: std::sync::OnceLock<parking_lot::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(parking_lot::Mutex::default)
}

#[cfg(test)]
pub(crate) fn ensure_cloud_backup_test_tokio_runtime() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("cloud-backup-test-tokio".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("create cloud backup test tokio runtime");

                let drive_runtime = tokio::runtime::Runtime::block_on;
                drive_runtime(&runtime, async move {
                    cove_tokio::init();
                    sender.send(()).expect("signal cloud backup test tokio runtime");
                    std::future::pending::<()>().await;
                });
            })
            .expect("spawn cloud backup test tokio runtime thread");
        receiver.recv().expect("wait for cloud backup test tokio runtime");
    });
}

fn sync_health_failed_message(
    sync_state: &PersistedCloudBlobSyncState,
    failed_state: &crate::database::cloud_backup::CloudBlobFailedState,
) -> String {
    if failed_state.error.is_empty() {
        return format!("cloud backup upload failed for record_id={}", sync_state.record_id);
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
        Err(CloudStorageError::NotFound(_)) => Ok(Vec::new()),
        Err(error) => Err(blocking_cloud_error(
            step,
            CloudBackupError::cloud_storage_context("list wallet backups", error),
        )),
    }
}

#[cfg(test)]
impl RustCloudBackupManager {
    pub(crate) async fn clear_wallet_upload_debouncers_for_test(&self) {
        act_zero::call!(self.supervisor.clear_upload_runtime_state())
            .await
            .expect("clear upload runtime state");
    }

    pub(crate) async fn verify_pending_uploads_once_for_test(&self) -> bool {
        !matches!(
            self.verify_pending_uploads_once().await,
            pending::PendingUploadVerificationStatus::Idle
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use act_zero::call;
    use tempfile::TempDir;

    fn test_lock() -> &'static parking_lot::Mutex<()> {
        super::cloud_backup_test_lock()
    }

    fn init_manager() -> Arc<RustCloudBackupManager> {
        super::ensure_cloud_backup_test_tokio_runtime();
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
            other_backups: CloudBackupOtherBackupsSummary::default(),
        }
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
        super::ensure_cloud_backup_test_tokio_runtime();
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

        manager.set_cloud_only(CloudOnlyState::Loaded { wallets: Vec::new() });
        manager.set_detail(Some(cloud_backup_detail(1)));

        assert!(matches!(manager.state.read().cloud_only, CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_loaded_cloud_only_cache_when_remote_count_drops_to_zero() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.set_cloud_only(CloudOnlyState::Loaded {
            wallets: vec![cloud_backup_wallet_item("wallet-1")],
        });
        manager.set_detail(Some(cloud_backup_detail(0)));

        assert!(matches!(manager.state.read().cloud_only, CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_cloud_only_cache_when_loaded_wallet_is_now_local() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-1");
        let mut detail = cloud_backup_detail(1);
        detail.up_to_date.push(wallet.clone());

        manager.set_cloud_only(CloudOnlyState::Loaded { wallets: vec![wallet] });
        manager.set_detail(Some(detail));

        assert!(matches!(manager.state.read().cloud_only, CloudOnlyState::NotFetched));
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
        let generic_failure = PersistedCloudBlobSyncState {
            namespace_id: "namespace".into(),
            wallet_id: None,
            record_id: "generic".into(),
            state: PersistedCloudBlobState::Failed(
                crate::database::cloud_backup::CloudBlobFailedState {
                    revision_hash: None,
                    retryable: true,
                    error: "generic failure".into(),
                    issue: None,
                    failed_at: 1,
                },
            ),
        };
        let authorization_failure = PersistedCloudBlobSyncState {
            namespace_id: "namespace".into(),
            wallet_id: None,
            record_id: "authorization".into(),
            state: PersistedCloudBlobState::Failed(
                crate::database::cloud_backup::CloudBlobFailedState {
                    revision_hash: None,
                    retryable: true,
                    error: "authorization required".into(),
                    issue: Some(CloudBlobFailureIssue::AuthorizationRequired),
                    failed_at: 2,
                },
            ),
        };

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
        let result = wallets::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::Mnemonic(ref m) if m == "abandon"));
    }

    #[test]
    fn convert_cloud_secret_tap_signer() {
        let secret = cove_cspp::backup_data::WalletSecret::TapSignerBackup(vec![1, 2, 3]);
        let result = wallets::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::TapSignerBackup(ref b) if b == &[1, 2, 3]));
    }

    #[test]
    fn convert_cloud_secret_descriptor_to_none() {
        let secret = cove_cspp::backup_data::WalletSecret::Descriptor("wpkh(...)".into());
        let result = wallets::convert_cloud_secret(&secret);
        assert!(matches!(result, LocalWalletSecret::None));
    }

    #[test]
    fn convert_cloud_secret_watch_only_to_none() {
        let result =
            wallets::convert_cloud_secret(&cove_cspp::backup_data::WalletSecret::WatchOnly);
        assert!(matches!(result, LocalWalletSecret::None));
    }

    #[test]
    fn restore_progress_updates_state() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let progress = CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Downloading,
            completed: 1,
            total: Some(2),
        };

        manager.set_restore_progress(Some(progress.clone()));

        assert_eq!(manager.state.read().restore_progress, Some(progress));
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
        let db_state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Enabled,
            ..PersistedCloudBackupState::default()
        };

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::ConfiguredNeverVerified,
        );
    }

    #[test]
    fn verification_metadata_is_verified_with_timestamp() {
        let db_state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Enabled,
            last_verified_at: Some(21),
            ..PersistedCloudBackupState::default()
        };

        assert_eq!(
            CloudBackupVerificationMetadata::from(&db_state),
            CloudBackupVerificationMetadata::Verified(21),
        );
    }

    #[test]
    fn verification_metadata_is_needs_verification_when_unverified() {
        let db_state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Unverified,
            last_verified_at: Some(21),
            ..PersistedCloudBackupState::default()
        };

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
    fn restore_complete_clears_restore_progress() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        manager.set_restore_progress(Some(CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Restoring,
            completed: 1,
            total: Some(2),
        }));
        manager.set_restore_progress(None);
        manager.set_restore_report(Some(CloudBackupRestoreReport {
            wallets_restored: 1,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        }));

        assert!(manager.state.read().restore_progress.is_none());
    }

    #[test]
    fn terminal_status_clears_restore_progress_and_keeps_report() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 2,
            failed_wallet_errors: vec!["download failed".into()],
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        manager.set_restore_progress(Some(CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Restoring,
            completed: 1,
            total: Some(2),
        }));
        manager.set_restore_progress(None);
        manager.set_restore_report(Some(report.clone()));
        manager.set_status(CloudBackupStatus::Error("all wallets failed".into()));

        let state = manager.state.read();
        assert!(state.restore_progress.is_none());
        assert_eq!(state.restore_report, Some(report));
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
        let progress = CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Downloading,
            completed: 1,
            total: Some(3),
        };

        let error = run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let progress = progress.clone();
            async move {
                manager
                    .set_restore_progress_for_restore_operation(&stale_operation, Some(progress))
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(manager.state.read().restore_progress, None);

        run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let progress = progress.clone();
            async move {
                manager
                    .set_restore_progress_for_restore_operation(&current_operation, Some(progress))
                    .await
                    .unwrap()
            }
        });

        assert_eq!(manager.state.read().restore_progress, Some(progress));
    }

    #[test]
    fn stale_restore_operation_cannot_update_status() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);

        let error = run_on_cloud_backup_runtime({
            let manager = manager.clone();
            async move {
                manager
                    .set_status_for_restore_operation(
                        &stale_operation,
                        CloudBackupStatus::Restoring,
                    )
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(manager.state.read().status, CloudBackupStatus::Disabled);

        run_on_cloud_backup_runtime({
            let manager = manager.clone();
            async move {
                manager
                    .set_status_for_restore_operation(
                        &current_operation,
                        CloudBackupStatus::Restoring,
                    )
                    .await
                    .unwrap()
            }
        });

        assert_eq!(manager.state.read().status, CloudBackupStatus::Restoring);
    }

    #[test]
    fn stale_restore_operation_cannot_update_restore_report() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);
        let report = CloudBackupRestoreReport {
            wallets_restored: 1,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        let error = run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let report = report.clone();
            async move {
                manager
                    .set_restore_report_for_restore_operation(&stale_operation, Some(report))
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(manager.state.read().restore_report, None);

        run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let report = report.clone();
            async move {
                manager
                    .set_restore_report_for_restore_operation(&current_operation, Some(report))
                    .await
                    .unwrap()
            }
        });

        assert_eq!(manager.state.read().restore_report, Some(report));
    }

    #[test]
    fn stale_restore_operation_cannot_persist_cloud_backup_state() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let db = Database::global();
        db.cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        manager.set_status(CloudBackupStatus::Disabled);

        let stale_operation = new_restore_operation(&manager);
        let current_operation = new_restore_operation(&manager);
        let persisted_state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Enabled,
            ..PersistedCloudBackupState::default()
        };

        let error = run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let persisted_state = persisted_state.clone();
            async move {
                manager
                    .persist_cloud_backup_state_for_restore_operation(
                        &stale_operation,
                        &persisted_state,
                        "test stale restore persist",
                    )
                    .await
                    .unwrap_err()
            }
        });

        assert!(matches!(error, CloudBackupError::Cancelled));
        assert_eq!(db.cloud_backup_state.get().unwrap(), PersistedCloudBackupState::default());
        assert_eq!(manager.state.read().status, CloudBackupStatus::Disabled);

        run_on_cloud_backup_runtime({
            let manager = manager.clone();
            let persisted_state = persisted_state.clone();
            async move {
                manager
                    .persist_cloud_backup_state_for_restore_operation(
                        &current_operation,
                        &persisted_state,
                        "test current restore persist",
                    )
                    .await
                    .unwrap()
            }
        });

        assert_eq!(db.cloud_backup_state.get().unwrap(), persisted_state);
        assert_eq!(manager.state.read().status, CloudBackupStatus::Enabled);
    }

    #[test]
    fn invalidated_restore_operation_becomes_cancelled() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let operation = new_restore_operation(&manager);

        invalidate_restore_operation(&manager);

        let error = run_on_cloud_backup_runtime({
            let manager = manager.clone();
            async move { manager.ensure_current_restore_operation(&operation).await.unwrap_err() }
        });
        assert!(matches!(error, CloudBackupError::Cancelled));
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
    async fn start_background_operation_claims_enabling_synchronously() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        manager.set_status(CloudBackupStatus::Disabled);
        manager.set_progress(None);
        manager.set_restore_progress(None);
        manager.set_restore_report(None);

        assert!(
            manager.begin_background_operation("first_enable", Some(CloudBackupStatus::Enabling),)
        );

        assert_eq!(manager.state.read().status, CloudBackupStatus::Enabling);
        manager.set_status(CloudBackupStatus::Disabled);
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
