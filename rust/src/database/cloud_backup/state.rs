use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use serde::{Deserialize, Serialize};

use crate::wallet::metadata::WalletId;

pub(crate) const CORRUPT_BLOB_SYNC_NAMESPACE_ID: &str = "__corrupt_cloud_backup_blob_sync_state__";
pub(crate) const CORRUPT_BLOB_SYNC_RECORD_ID: &str = "__corrupt_cloud_backup_blob_sync_state__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedCloudBackupStatus {
    Disabled,
    Enabled,
    Unverified,
    PasskeyMissing,
    Disabling,
    Corrupted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PersistedCloudBackupState {
    #[default]
    Disabled,
    Configured(PersistedConfiguredCloudBackup),
    ExclusiveTransition(PersistedCloudBackupTransition),
    Corrupted {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One durable operation that exclusively owns cloud backup configuration
pub enum PersistedCloudBackupTransition {
    /// Cloud backup is being deleted and disabled
    Disabling(PersistedDisablingCloudBackup),
    /// Cloud backup is moving between Google Drive accounts
    DriveAccountSwitch(PersistedDriveAccountSwitchState),
}

impl PersistedCloudBackupState {
    pub fn corrupted(error: impl Into<String>) -> Self {
        Self::Corrupted { error: error.into() }
    }

    pub fn status(&self) -> PersistedCloudBackupStatus {
        match self {
            Self::Disabled => PersistedCloudBackupStatus::Disabled,
            Self::Configured(configured) => configured.status(),
            Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(_)) => {
                PersistedCloudBackupStatus::Disabling
            }
            Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
                account_switch,
            )) => account_switch.configured.status(),
            Self::Corrupted { .. } => PersistedCloudBackupStatus::Corrupted,
        }
    }

    pub fn is_configured(&self) -> bool {
        matches!(
            self,
            Self::Configured(_)
                | Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(_))
        )
    }

    pub(crate) fn has_configured_backup(&self) -> bool {
        self.configured().is_some()
    }

    pub fn is_disabling(&self) -> bool {
        matches!(self, Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(_)))
    }

    pub fn disabling(&self) -> Option<&PersistedDisablingCloudBackup> {
        match self {
            Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) => {
                Some(disabling)
            }
            Self::Disabled
            | Self::Configured(_)
            | Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(_))
            | Self::Corrupted { .. } => None,
        }
    }

    pub(crate) fn exclusive_transition(&self) -> Option<&PersistedCloudBackupTransition> {
        match self {
            Self::ExclusiveTransition(transition) => Some(transition),
            Self::Disabled | Self::Configured(_) | Self::Corrupted { .. } => None,
        }
    }

    fn configured(&self) -> Option<&PersistedConfiguredCloudBackup> {
        match self {
            Self::Configured(configured) => Some(configured),
            Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) => {
                Some(&disabling.previous_configured)
            }
            Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
                account_switch,
            )) => Some(&account_switch.configured),
            Self::Disabled | Self::Corrupted { .. } => None,
        }
    }

    fn configured_mut(&mut self) -> Option<&mut PersistedConfiguredCloudBackup> {
        match self {
            Self::Configured(configured) => Some(configured),
            Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) => {
                Some(&mut disabling.previous_configured)
            }
            Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
                account_switch,
            )) => Some(&mut account_switch.configured),
            Self::Disabled | Self::Corrupted { .. } => None,
        }
    }

    pub fn is_unverified(&self) -> bool {
        matches!(self.status(), PersistedCloudBackupStatus::Unverified)
    }

    pub fn is_passkey_missing(&self) -> bool {
        matches!(self.status(), PersistedCloudBackupStatus::PasskeyMissing)
    }

    pub fn last_sync(&self) -> Option<u64> {
        self.configured().and_then(|configured| configured.sync.last_sync)
    }

    pub fn wallet_count(&self) -> Option<u32> {
        self.configured().and_then(|configured| configured.sync.wallet_count)
    }

    pub fn last_verified_at(&self) -> Option<u64> {
        self.configured().and_then(|configured| configured.verification.last_verified_at())
    }

    pub fn last_verification_requested_at(&self) -> Option<u64> {
        self.configured().and_then(|configured| configured.verification.requested_at())
    }

    pub fn last_verification_dismissed_at(&self) -> Option<u64> {
        self.configured().and_then(|configured| configured.verification.dismissed_at())
    }

    pub fn pending_verification_completion(
        &self,
    ) -> Option<&PersistedPendingVerificationCompletion> {
        self.configured().and_then(|configured| configured.pending_verification_completion.as_ref())
    }

    pub fn pending_restore_all(&self) -> Option<&PersistedRestoreAllMarker> {
        self.configured().and_then(|configured| configured.pending_restore_all.as_ref())
    }

    pub(crate) fn drive_account_switch(&self) -> Option<&PersistedDriveAccountSwitch> {
        match self {
            Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
                account_switch,
            )) => Some(&account_switch.transition),
            Self::Disabled
            | Self::Configured(_)
            | Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(_))
            | Self::Corrupted { .. } => None,
        }
    }

    pub fn should_prompt_verification(&self) -> bool {
        if !self.is_unverified() {
            return false;
        }

        let Some(requested_at) = self.last_verification_requested_at() else {
            return false;
        };

        if self.last_verified_at().is_some_and(|verified_at| verified_at >= requested_at) {
            return false;
        }

        if self
            .last_verification_dismissed_at()
            .is_some_and(|dismissed_at| dismissed_at >= requested_at)
        {
            return false;
        }

        true
    }

    pub fn set_wallet_count(&mut self, wallet_count: Option<u32>) {
        let Some(configured) = self.configured_mut() else { return };

        configured.sync.wallet_count = wallet_count;
    }

    pub fn record_successful_sync(&mut self, last_sync: u64, wallet_count: u32) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.passkey = PersistedPasskeyState::Available;
        configured.sync = PersistedBackupSyncState {
            last_sync: Some(last_sync),
            wallet_count: Some(wallet_count),
        };
        true
    }

    pub fn mark_enabled_reset_verification(last_sync: u64, wallet_count: u32) -> Self {
        Self::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification: PersistedBackupVerificationState::Required {
                last_verified_at: None,
                requested_at: None,
                dismissed_at: None,
            },
            sync: PersistedBackupSyncState {
                last_sync: Some(last_sync),
                wallet_count: Some(wallet_count),
            },
            pending_verification_completion: None,
            pending_restore_all: None,
        })
    }

    pub fn configured_after_restore(last_sync: u64, wallet_count: u32) -> Self {
        Self::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification: PersistedBackupVerificationState::NotVerified {
                requested_at: None,
                dismissed_at: None,
            },
            sync: PersistedBackupSyncState {
                last_sync: Some(last_sync),
                wallet_count: Some(wallet_count),
            },
            pending_verification_completion: None,
            pending_restore_all: None,
        })
    }

    pub(crate) fn reset_verification_after_successful_sync(
        &mut self,
        last_sync: u64,
        wallet_count: u32,
    ) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.passkey = PersistedPasskeyState::Available;
        configured.verification = PersistedBackupVerificationState::Required {
            last_verified_at: None,
            requested_at: None,
            dismissed_at: None,
        };
        configured.sync = PersistedBackupSyncState {
            last_sync: Some(last_sync),
            wallet_count: Some(wallet_count),
        };
        configured.pending_verification_completion = None;
        configured.pending_restore_all = None;
        true
    }

    pub fn mark_verified_at(&mut self, verified_at: u64) {
        let Some(configured) = self.configured_mut() else { return };

        configured.passkey = PersistedPasskeyState::Available;
        configured.verification = PersistedBackupVerificationState::Verified {
            last_verified_at: verified_at,
            requested_at: configured.verification.requested_at(),
            dismissed_at: configured.verification.dismissed_at(),
        };
    }

    pub fn mark_passkey_missing(&mut self) {
        let Some(configured) = self.configured_mut() else { return };

        configured.passkey = PersistedPasskeyState::Missing;
    }

    pub fn mark_verification_required(&mut self, requested_at: Option<u64>) {
        let Some(configured) = self.configured_mut() else { return };

        configured.verification = PersistedBackupVerificationState::Required {
            last_verified_at: configured.verification.last_verified_at(),
            requested_at,
            dismissed_at: configured.verification.dismissed_at(),
        };
    }

    pub fn dismiss_verification_request(&mut self, dismissed_at: u64) -> bool {
        let Some(configured) = self.configured_mut() else { return false };
        if configured.verification.requested_at().is_none() {
            return false;
        }

        configured.verification = configured.verification.clone().with_dismissed_at(dismissed_at);
        true
    }

    pub fn replace_pending_verification_completion(
        &mut self,
        completion: PersistedPendingVerificationCompletion,
    ) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.pending_verification_completion = Some(completion);
        true
    }

    pub fn clear_pending_verification_completion(&mut self) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.pending_verification_completion.take().is_some()
    }

    pub fn replace_pending_restore_all(&mut self, marker: PersistedRestoreAllMarker) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.pending_restore_all = Some(marker);
        true
    }

    pub fn clear_pending_restore_all(&mut self) -> bool {
        let Some(configured) = self.configured_mut() else { return false };

        configured.pending_restore_all.take().is_some()
    }

    pub(crate) fn set_drive_account_switch(
        &mut self,
        account_switch: PersistedDriveAccountSwitch,
    ) -> bool {
        let Self::Configured(_) = self else { return false };
        let Self::Configured(configured) = std::mem::take(self) else { unreachable!() };

        *self = Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
            PersistedDriveAccountSwitchState { configured, transition: account_switch },
        ));
        true
    }

    pub(crate) fn set_drive_account_switch_phase(
        &mut self,
        transition_id: DriveAccountSwitchId,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> bool {
        let Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
            account_switch,
        )) = self
        else {
            return false;
        };
        if account_switch.transition.transition_id != transition_id {
            return false;
        }

        account_switch.transition.phase = phase;
        true
    }

    pub(crate) fn clear_drive_account_switch(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> bool {
        let Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
            account_switch,
        )) = self
        else {
            return false;
        };
        if account_switch.transition.transition_id != transition_id {
            return false;
        }

        let Self::ExclusiveTransition(PersistedCloudBackupTransition::DriveAccountSwitch(
            account_switch,
        )) = std::mem::take(self)
        else {
            unreachable!()
        };
        *self = Self::Configured(account_switch.configured);
        true
    }

    pub(crate) fn begin_disabling(
        &mut self,
        namespace_id: String,
        disable_generation: u64,
        started_at: u64,
    ) -> bool {
        let Self::Configured(_) = self else { return false };
        let Self::Configured(configured) = std::mem::take(self) else { unreachable!() };

        *self = Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(
            PersistedDisablingCloudBackup {
                previous_configured: configured,
                namespace_id,
                disable_generation,
                started_at,
                delete_started_at: None,
                last_error: None,
                retry_after: None,
            },
        ));
        true
    }

    pub(crate) fn disabling_transition(disabling: PersistedDisablingCloudBackup) -> Self {
        Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling))
    }

    pub(crate) fn restore_configured_after_disable(&mut self, disable_generation: u64) -> bool {
        let Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) = self
        else {
            return false;
        };
        if disabling.disable_generation != disable_generation {
            return false;
        }

        let Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) =
            std::mem::take(self)
        else {
            unreachable!()
        };
        *self = Self::Configured(disabling.previous_configured);
        true
    }

    pub(crate) fn update_disabling(&mut self, update: &PersistedDisablingCloudBackup) -> bool {
        let Self::ExclusiveTransition(PersistedCloudBackupTransition::Disabling(disabling)) = self
        else {
            return false;
        };
        if disabling.disable_generation != update.disable_generation
            || disabling.namespace_id != update.namespace_id
        {
            return false;
        }

        disabling.delete_started_at = update.delete_started_at;
        disabling.last_error.clone_from(&update.last_error);
        disabling.retry_after = update.retry_after;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDisablingCloudBackup {
    pub previous_configured: PersistedConfiguredCloudBackup,
    pub namespace_id: String,
    pub disable_generation: u64,
    pub started_at: u64,
    #[serde(default)]
    pub delete_started_at: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retry_after: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedConfiguredCloudBackup {
    pub passkey: PersistedPasskeyState,
    pub verification: PersistedBackupVerificationState,
    pub sync: PersistedBackupSyncState,
    #[serde(default)]
    pub pending_verification_completion: Option<PersistedPendingVerificationCompletion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_restore_all: Option<PersistedRestoreAllMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Configured backup state held while a Google Drive account switch is in progress
pub struct PersistedDriveAccountSwitchState {
    /// Backup configuration updated by writes admitted before the switch fence
    pub configured: PersistedConfiguredCloudBackup,
    /// Durable account-switch protocol state
    pub transition: PersistedDriveAccountSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDriveAccountSwitch {
    pub transition_id: DriveAccountSwitchId,
    pub phase: PersistedDriveAccountSwitchPhase,
}

/// Durable identifier for one Google Drive account switch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DriveAccountSwitchId(u64);

impl DriveAccountSwitchId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the platform representation of this transition identifier
    pub fn value(self) -> u64 {
        self.0
    }
}

impl From<u64> for DriveAccountSwitchId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedDriveAccountSwitchPhase {
    AwaitingAccountSelection,
    Reinitializing,
    AwaitingAccountCommitSucceeded,
    #[serde(rename = "AwaitingAccountCommitFailed")]
    AwaitingReinitializationRetry,
    AwaitingAccountRollback,
}

impl PersistedConfiguredCloudBackup {
    fn status(&self) -> PersistedCloudBackupStatus {
        match self.passkey {
            PersistedPasskeyState::Missing => PersistedCloudBackupStatus::PasskeyMissing,
            PersistedPasskeyState::Available => self.verification.status(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedRestoreAllMarker {
    pub namespace_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedPasskeyState {
    Available,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data")]
pub enum PersistedBackupVerificationState {
    NotVerified {
        #[serde(default)]
        requested_at: Option<u64>,
        #[serde(default)]
        dismissed_at: Option<u64>,
    },
    Verified {
        last_verified_at: u64,
        #[serde(default)]
        requested_at: Option<u64>,
        #[serde(default)]
        dismissed_at: Option<u64>,
    },
    Required {
        #[serde(default)]
        last_verified_at: Option<u64>,
        #[serde(default)]
        requested_at: Option<u64>,
        #[serde(default)]
        dismissed_at: Option<u64>,
    },
}

impl PersistedBackupVerificationState {
    fn status(&self) -> PersistedCloudBackupStatus {
        match self {
            Self::NotVerified { .. } | Self::Verified { .. } => PersistedCloudBackupStatus::Enabled,
            Self::Required { .. } => PersistedCloudBackupStatus::Unverified,
        }
    }

    fn last_verified_at(&self) -> Option<u64> {
        match self {
            Self::NotVerified { .. } => None,
            Self::Verified { last_verified_at, .. } => Some(*last_verified_at),
            Self::Required { last_verified_at, .. } => *last_verified_at,
        }
    }

    fn requested_at(&self) -> Option<u64> {
        match self {
            Self::NotVerified { requested_at, .. }
            | Self::Verified { requested_at, .. }
            | Self::Required { requested_at, .. } => *requested_at,
        }
    }

    fn dismissed_at(&self) -> Option<u64> {
        match self {
            Self::NotVerified { dismissed_at, .. }
            | Self::Verified { dismissed_at, .. }
            | Self::Required { dismissed_at, .. } => *dismissed_at,
        }
    }

    fn with_dismissed_at(self, dismissed_at: u64) -> Self {
        match self {
            Self::NotVerified { requested_at, .. } => {
                Self::NotVerified { requested_at, dismissed_at: Some(dismissed_at) }
            }
            Self::Verified { last_verified_at, requested_at, .. } => {
                Self::Verified { last_verified_at, requested_at, dismissed_at: Some(dismissed_at) }
            }
            Self::Required { last_verified_at, requested_at, .. } => {
                Self::Required { last_verified_at, requested_at, dismissed_at: Some(dismissed_at) }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedBackupSyncState {
    #[serde(default)]
    pub last_sync: Option<u64>,
    #[serde(default)]
    pub wallet_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPendingVerificationCompletion {
    pub report: PersistedDeepVerificationReport,
    pub namespace_id: String,
    pub uploads: Vec<PersistedPendingVerificationUpload>,
    #[serde(default)]
    pub created_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDeepVerificationReport {
    pub master_key_wrapper_repaired: bool,
    pub local_master_key_repaired: bool,
    pub credential_recovered: bool,
    pub wallets_verified: u32,
    pub wallets_failed: u32,
    pub wallets_unsupported: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PersistedPendingVerificationUpload {
    MasterKeyWrapper,
    Wallet { record_id: String, expected_revision: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedCloudBlobSyncState {
    pub namespace_id: String,
    record_key: CloudBackupRecordKey,
    pub state: PersistedCloudBlobState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudBackupRecordKey {
    MasterKeyWrapper,
    Wallet(WalletId, String),
    Corrupted(String),
}

impl PersistedCloudBlobSyncState {
    pub fn master_key_wrapper(namespace_id: String, state: PersistedCloudBlobState) -> Self {
        Self { namespace_id, record_key: CloudBackupRecordKey::MasterKeyWrapper, state }
    }

    pub fn wallet(
        namespace_id: String,
        wallet_id: WalletId,
        record_id: String,
        state: PersistedCloudBlobState,
    ) -> Self {
        Self { namespace_id, record_key: CloudBackupRecordKey::Wallet(wallet_id, record_id), state }
    }

    pub fn from_record_key(
        namespace_id: String,
        record_key: CloudBackupRecordKey,
        state: PersistedCloudBlobState,
    ) -> Self {
        Self { namespace_id, record_key, state }
    }

    pub fn corrupted(error: String) -> Self {
        Self {
            namespace_id: CORRUPT_BLOB_SYNC_NAMESPACE_ID.to_string(),
            record_key: CloudBackupRecordKey::Corrupted(CORRUPT_BLOB_SYNC_RECORD_ID.to_string()),
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                retryable: false,
                issue: None,
                error,
                failed_at: 0,
            }),
        }
    }

    pub fn record_key(&self) -> &CloudBackupRecordKey {
        &self.record_key
    }

    pub fn record_id(&self) -> &str {
        self.record_key.record_id()
    }

    pub fn wallet_id(&self) -> Option<&WalletId> {
        self.record_key.wallet_id()
    }

    pub fn with_state(&self, state: PersistedCloudBlobState) -> Self {
        Self { namespace_id: self.namespace_id.clone(), record_key: self.record_key.clone(), state }
    }

    pub fn is_master_key_wrapper(&self) -> bool {
        self.record_key.is_master_key_wrapper()
    }

    pub fn is_wallet_record(&self) -> bool {
        self.record_key.is_wallet()
    }

    pub fn is_corrupted(&self) -> bool {
        self.record_key.is_corrupted()
    }

    pub fn is_dirty(&self) -> bool {
        matches!(self.state, PersistedCloudBlobState::Dirty(_))
    }

    pub fn is_uploaded_pending_confirmation(&self) -> bool {
        matches!(self.state, PersistedCloudBlobState::UploadedPendingConfirmation(_))
    }
}

impl CloudBackupRecordKey {
    pub fn master_key_record_id() -> &'static str {
        MASTER_KEY_RECORD_ID
    }

    pub fn record_id(&self) -> &str {
        match self {
            Self::MasterKeyWrapper => Self::master_key_record_id(),
            Self::Wallet(_, record_id) => record_id,
            Self::Corrupted(record_id) => record_id,
        }
    }

    pub fn wallet_id(&self) -> Option<&WalletId> {
        match self {
            Self::MasterKeyWrapper | Self::Corrupted(_) => None,
            Self::Wallet(wallet_id, _) => Some(wallet_id),
        }
    }

    pub fn is_master_key_wrapper(&self) -> bool {
        matches!(self, Self::MasterKeyWrapper)
    }

    pub fn is_wallet(&self) -> bool {
        matches!(self, Self::Wallet(_, _))
    }

    pub fn is_corrupted(&self) -> bool {
        matches!(self, Self::Corrupted(_))
    }

    pub fn into_parts(self) -> (Option<WalletId>, String) {
        match self {
            Self::MasterKeyWrapper => (None, MASTER_KEY_RECORD_ID.to_string()),
            Self::Wallet(wallet_id, record_id) => (Some(wallet_id), record_id),
            Self::Corrupted(record_id) => (None, record_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedCloudBlobState {
    Dirty(CloudBlobDirtyState),
    Uploading(CloudBlobUploadingState),
    UploadedPendingConfirmation(CloudBlobUploadedPendingConfirmationState),
    Confirmed(CloudBlobConfirmedState),
    Failed(CloudBlobFailedState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobDirtyState {
    pub changed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobUploadingState {
    pub revision_hash: String,
    pub started_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobUploadedPendingConfirmationState {
    pub revision_hash: String,
    pub uploaded_at: u64,
    pub attempt_count: u32,
    pub last_checked_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobConfirmedState {
    pub revision_hash: String,
    pub confirmed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobFailedState {
    pub revision_hash: Option<String>,
    #[serde(default)]
    pub retryable: bool,
    pub error: String,
    #[serde(default)]
    pub issue: Option<CloudStorageIssue>,
    pub failed_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudStorageIssue {
    AuthorizationRequired,
    Offline,
    Unavailable,
    NotFound,
    QuotaExceeded,
    Other,
}

impl CloudStorageIssue {
    pub(crate) fn persistable(self) -> Option<Self> {
        match self {
            Self::Other => None,
            issue => Some(issue),
        }
    }
}
