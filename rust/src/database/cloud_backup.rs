use std::sync::Arc;

use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use redb::{ReadableTable as _, TableDefinition};
use serde::{Deserialize, Serialize};

use cove_types::redb::Json;
use cove_util::result_ext::ResultExt as _;

use super::Error;
use crate::wallet::metadata::WalletId;

mod compatibility;

const CURRENT_KEY: &str = "current";

pub(crate) const CLOUD_BACKUP_STATE_TABLE: TableDefinition<
    &'static str,
    Json<PersistedCloudBackupState>,
> = TableDefinition::new("cloud_backup_state");
pub(crate) const CLOUD_BLOB_SYNC_STATE_TABLE: TableDefinition<
    &'static str,
    Json<PersistedCloudBlobSyncState>,
> = TableDefinition::new("cloud_blob_sync_state");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedCloudBackupStatus {
    Disabled,
    Enabled,
    Unverified,
    PasskeyMissing,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PersistedCloudBackupState {
    #[default]
    Disabled,
    Configured(PersistedConfiguredCloudBackup),
}

impl PersistedCloudBackupState {
    pub fn status(&self) -> PersistedCloudBackupStatus {
        match self {
            Self::Disabled => PersistedCloudBackupStatus::Disabled,
            Self::Configured(configured) => configured.status(),
        }
    }

    pub fn is_configured(&self) -> bool {
        matches!(self, Self::Configured(_))
    }

    pub fn is_unverified(&self) -> bool {
        matches!(self.status(), PersistedCloudBackupStatus::Unverified)
    }

    pub fn is_passkey_missing(&self) -> bool {
        matches!(self.status(), PersistedCloudBackupStatus::PasskeyMissing)
    }

    pub fn last_sync(&self) -> Option<u64> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.sync.last_sync,
        }
    }

    pub fn wallet_count(&self) -> Option<u32> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.sync.wallet_count,
        }
    }

    pub fn last_verified_at(&self) -> Option<u64> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.verification.last_verified_at(),
        }
    }

    pub fn last_verification_requested_at(&self) -> Option<u64> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.verification.requested_at(),
        }
    }

    pub fn last_verification_dismissed_at(&self) -> Option<u64> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.verification.dismissed_at(),
        }
    }

    pub fn pending_verification_completion(
        &self,
    ) -> Option<&PersistedPendingVerificationCompletion> {
        match self {
            Self::Disabled => None,
            Self::Configured(configured) => configured.pending_verification_completion.as_ref(),
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

    pub fn with_wallet_count(&self, wallet_count: Option<u32>) -> Self {
        let mut state = self.clone();
        state.set_wallet_count(wallet_count);
        state
    }

    pub fn set_wallet_count(&mut self, wallet_count: Option<u32>) {
        let Self::Configured(configured) = self else {
            return;
        };

        configured.sync.wallet_count = wallet_count;
    }

    pub fn mark_enabled_preserving_verification(&self, last_sync: u64, wallet_count: u32) -> Self {
        let verification = match self {
            Self::Configured(configured) => configured.verification.clone(),
            Self::Disabled => PersistedBackupVerificationState::NotVerified {
                requested_at: None,
                dismissed_at: None,
            },
        };

        Self::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification,
            sync: PersistedBackupSyncState {
                last_sync: Some(last_sync),
                wallet_count: Some(wallet_count),
            },
            pending_verification_completion: self.pending_verification_completion().cloned(),
        })
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
        })
    }

    pub fn mark_verified_at(&mut self, verified_at: u64) {
        let Self::Configured(configured) = self else {
            return;
        };

        configured.passkey = PersistedPasskeyState::Available;
        configured.verification = PersistedBackupVerificationState::Verified {
            last_verified_at: verified_at,
            requested_at: configured.verification.requested_at(),
            dismissed_at: configured.verification.dismissed_at(),
        };
    }

    pub fn mark_passkey_missing(&mut self) {
        let Self::Configured(configured) = self else {
            return;
        };

        configured.passkey = PersistedPasskeyState::Missing;
    }

    pub fn mark_verification_required(&mut self, requested_at: Option<u64>) {
        let Self::Configured(configured) = self else {
            return;
        };

        configured.verification = PersistedBackupVerificationState::Required {
            last_verified_at: configured.verification.last_verified_at(),
            requested_at,
            dismissed_at: configured.verification.dismissed_at(),
        };
    }

    pub fn dismiss_verification_request(&mut self, dismissed_at: u64) -> bool {
        let Self::Configured(configured) = self else {
            return false;
        };
        if configured.verification.requested_at().is_none() {
            return false;
        }

        configured.verification = configured.verification.clone().with_dismissed_at(dismissed_at);
        true
    }

    pub fn replace_pending_verification_completion(
        &mut self,
        completion: PersistedPendingVerificationCompletion,
    ) {
        let Self::Configured(configured) = self else {
            return;
        };

        configured.pending_verification_completion = Some(completion);
    }

    pub fn clear_pending_verification_completion(&mut self) -> bool {
        let Self::Configured(configured) = self else {
            return false;
        };

        configured.pending_verification_completion.take().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedConfiguredCloudBackup {
    pub passkey: PersistedPasskeyState,
    pub verification: PersistedBackupVerificationState,
    pub sync: PersistedBackupSyncState,
    #[serde(default)]
    pub pending_verification_completion: Option<PersistedPendingVerificationCompletion>,
}

impl PersistedConfiguredCloudBackup {
    fn status(&self) -> PersistedCloudBackupStatus {
        match self.passkey {
            PersistedPasskeyState::Missing => PersistedCloudBackupStatus::PasskeyMissing,
            PersistedPasskeyState::Available => self.verification.status(),
        }
    }
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
        }
    }

    pub fn wallet_id(&self) -> Option<&WalletId> {
        match self {
            Self::MasterKeyWrapper => None,
            Self::Wallet(wallet_id, _) => Some(wallet_id),
        }
    }

    pub fn is_master_key_wrapper(&self) -> bool {
        matches!(self, Self::MasterKeyWrapper)
    }

    pub fn is_wallet(&self) -> bool {
        matches!(self, Self::Wallet(_, _))
    }

    pub fn into_parts(self) -> (Option<WalletId>, String) {
        match self {
            Self::MasterKeyWrapper => (None, MASTER_KEY_RECORD_ID.to_string()),
            Self::Wallet(wallet_id, record_id) => (Some(wallet_id), record_id),
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
    pub issue: Option<CloudBlobFailureIssue>,
    pub failed_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudBlobFailureIssue {
    AuthorizationRequired,
    Offline,
    Unavailable,
    NotFound,
    QuotaExceeded,
}

#[derive(Debug, Clone)]
pub struct CloudBackupStateTable {
    db: Arc<redb::Database>,
}

impl CloudBackupStateTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        write_txn
            .open_table(CLOUD_BACKUP_STATE_TABLE)
            .expect("failed to create cloud backup state table");

        Self { db }
    }

    pub fn get(&self) -> Result<PersistedCloudBackupState, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;
        let table =
            read_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;

        Ok(table
            .get(CURRENT_KEY)
            .map_err_str(Error::TableAccess)?
            .map(|value| value.value())
            .unwrap_or_default())
    }

    pub fn set(&self, value: &PersistedCloudBackupState) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table =
                write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;
            table.insert(CURRENT_KEY, value).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    pub fn delete(&self) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table =
                write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;
            table.remove(CURRENT_KEY).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CloudBlobSyncStateTable {
    db: Arc<redb::Database>,
}

impl CloudBlobSyncStateTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        write_txn
            .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
            .expect("failed to create cloud blob sync state table");

        Self { db }
    }

    pub fn get(&self, record_id: &str) -> Result<Option<PersistedCloudBlobSyncState>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;
        let table =
            read_txn.open_table(CLOUD_BLOB_SYNC_STATE_TABLE).map_err_str(Error::TableAccess)?;

        Ok(table.get(record_id).map_err_str(Error::TableAccess)?.map(|value| value.value()))
    }

    pub fn list(&self) -> Result<Vec<PersistedCloudBlobSyncState>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;
        let table =
            read_txn.open_table(CLOUD_BLOB_SYNC_STATE_TABLE).map_err_str(Error::TableAccess)?;

        let mut states = Vec::new();
        let iter = table.iter().map_err_str(Error::TableAccess)?;
        for entry in iter {
            let (_, value) = entry.map_err_str(Error::TableAccess)?;
            states.push(value.value());
        }

        Ok(states)
    }

    pub fn set(&self, value: &PersistedCloudBlobSyncState) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;
            table.insert(value.record_id(), value).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    pub fn set_if_current(
        &self,
        current: &PersistedCloudBlobSyncState,
        next: &PersistedCloudBlobSyncState,
    ) -> Result<bool, Error> {
        debug_assert_eq!(current.record_id(), next.record_id());

        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;

            let matches_current = table
                .get(current.record_id())
                .map_err_str(Error::TableAccess)?
                .map(|stored| stored.value() == *current)
                .unwrap_or(false);

            if !matches_current {
                return Ok(false);
            }

            table.insert(next.record_id(), next).map_err_str(Error::TableAccess)?;
        }
        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(true)
    }

    pub fn delete(&self, record_id: &str) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;
            table.remove(record_id).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    pub fn delete_all(&self) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;
            // collect keys before removal because redb iterators borrow table from write_txn,
            // so CLOUD_BLOB_SYNC_STATE_TABLE cannot be mutated while iterating
            let keys = table
                .iter()
                .map_err_str(Error::TableAccess)?
                .map(|entry| {
                    let (key, _) = entry.map_err_str(Error::TableAccess)?;
                    Ok(key.value().to_string())
                })
                .collect::<Result<Vec<_>, Error>>()?;

            for key in keys {
                table.remove(key.as_str()).map_err_str(Error::TableAccess)?;
            }
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_state(
        passkey: PersistedPasskeyState,
        verification: PersistedBackupVerificationState,
        last_sync: Option<u64>,
        wallet_count: Option<u32>,
    ) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey,
            verification,
            sync: PersistedBackupSyncState { last_sync, wallet_count },
            pending_verification_completion: None,
        })
    }

    #[test]
    fn verification_prompt_requires_newer_request() {
        let state = configured_state(
            PersistedPasskeyState::Available,
            PersistedBackupVerificationState::Required {
                last_verified_at: None,
                requested_at: Some(20),
                dismissed_at: Some(10),
            },
            None,
            None,
        );

        assert!(state.should_prompt_verification());
    }

    #[test]
    fn verification_prompt_respects_dismissal() {
        let state = configured_state(
            PersistedPasskeyState::Available,
            PersistedBackupVerificationState::Required {
                last_verified_at: None,
                requested_at: Some(20),
                dismissed_at: Some(20),
            },
            None,
            None,
        );

        assert!(!state.should_prompt_verification());
    }

    #[test]
    fn blob_sync_state_helpers_reflect_state() {
        let confirmed = PersistedCloudBlobSyncState::wallet(
            "ns-1".into(),
            "wallet-a".into(),
            "wallet-a".into(),
            PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                revision_hash: "rev-1".into(),
                confirmed_at: 42,
            }),
        );

        assert!(!confirmed.is_dirty());

        let dirty = confirmed
            .with_state(PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 10 }));

        assert!(dirty.is_dirty());
    }

    #[test]
    fn uploaded_pending_confirmation_tracks_attempts() {
        let state = PersistedCloudBlobSyncState::wallet(
            "ns-1".into(),
            "wallet-a".into(),
            "wallet-a".into(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 3,
                    last_checked_at: Some(12),
                },
            ),
        );

        assert!(state.is_uploaded_pending_confirmation());
    }
}
