use cove_types::network::Network;

use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::wallet::metadata::{WalletMode as LocalWalletMode, WalletType};

use super::CloudBackupVerificationSource;

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
#[derive(Debug, Clone)]
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

/// Trust failure that tells the UI which recovery path is valid
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl CloudBackupRetryContext {
    pub(crate) fn connectivity(action: CloudBackupRetryAction) -> Self {
        Self { issue: CloudBackupRetryIssue::Connectivity, action }
    }
}

/// Top-level state snapshot exposed to platform managers
#[derive(Debug, Clone)]
pub struct CloudBackupState {
    pub lifecycle: super::CloudBackupLifecycle,
    pub settings_row_status: CloudBackupSettingsRowStatus,
}

impl Default for CloudBackupState {
    fn default() -> Self {
        Self {
            lifecycle: super::CloudBackupLifecycle::Disabled,
            settings_row_status: CloudBackupSettingsRowStatus::Disabled,
        }
    }
}
