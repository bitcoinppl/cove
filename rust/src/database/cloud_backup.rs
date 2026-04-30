use std::sync::Arc;

use redb::{ReadableTable as _, TableDefinition};
use serde::{Deserialize, Serialize};

use cove_types::redb::Json;
use cove_util::result_ext::ResultExt as _;

use super::Error;
use crate::wallet::metadata::WalletId;

const CURRENT_KEY: &str = "current";

const CLOUD_BACKUP_STATE_TABLE: TableDefinition<&'static str, Json<PersistedCloudBackupState>> =
    TableDefinition::new("cloud_backup_state");
const CLOUD_BLOB_SYNC_STATE_TABLE: TableDefinition<
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCloudBackupState {
    pub status: PersistedCloudBackupStatus,
    #[serde(default)]
    pub last_sync: Option<u64>,
    #[serde(default)]
    pub wallet_count: Option<u32>,
    #[serde(default)]
    pub last_verified_at: Option<u64>,
    #[serde(default)]
    pub last_verification_requested_at: Option<u64>,
    #[serde(default)]
    pub last_verification_dismissed_at: Option<u64>,
    #[serde(default)]
    pub pending_verification_completion: Option<PersistedPendingVerificationCompletion>,
}

impl Default for PersistedCloudBackupState {
    fn default() -> Self {
        Self {
            status: PersistedCloudBackupStatus::Disabled,
            last_sync: None,
            wallet_count: None,
            last_verified_at: None,
            last_verification_requested_at: None,
            last_verification_dismissed_at: None,
            pending_verification_completion: None,
        }
    }
}

impl PersistedCloudBackupState {
    pub fn is_configured(&self) -> bool {
        !matches!(self.status, PersistedCloudBackupStatus::Disabled)
    }

    pub fn is_unverified(&self) -> bool {
        matches!(self.status, PersistedCloudBackupStatus::Unverified)
    }

    pub fn is_passkey_missing(&self) -> bool {
        matches!(self.status, PersistedCloudBackupStatus::PasskeyMissing)
    }

    pub fn should_prompt_verification(&self) -> bool {
        if !self.is_unverified() {
            return false;
        }

        let Some(requested_at) = self.last_verification_requested_at else {
            return false;
        };

        if self.last_verified_at.is_some_and(|verified_at| verified_at >= requested_at) {
            return false;
        }

        if self
            .last_verification_dismissed_at
            .is_some_and(|dismissed_at| dismissed_at >= requested_at)
        {
            return false;
        }

        true
    }

    pub fn with_wallet_count(&self, wallet_count: Option<u32>) -> Self {
        Self { wallet_count, ..self.clone() }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPendingVerificationUpload {
    pub record_id: String,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudUploadKind {
    BackupBlob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCloudBlobSyncState {
    pub kind: CloudUploadKind,
    pub namespace_id: String,
    pub wallet_id: Option<WalletId>,
    pub record_id: String,
    pub state: PersistedCloudBlobState,
}

impl PersistedCloudBlobSyncState {
    pub fn is_dirty(&self) -> bool {
        matches!(self.state, PersistedCloudBlobState::Dirty(_))
    }

    pub fn is_uploaded_pending_confirmation(&self) -> bool {
        matches!(self.state, PersistedCloudBlobState::UploadedPendingConfirmation(_))
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
            table.insert(value.record_id.as_str(), value).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    pub fn set_if_current(
        &self,
        current: &PersistedCloudBlobSyncState,
        next: &PersistedCloudBlobSyncState,
    ) -> Result<bool, Error> {
        debug_assert_eq!(current.record_id, next.record_id);

        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;

            let matches_current = table
                .get(current.record_id.as_str())
                .map_err_str(Error::TableAccess)?
                .map(|stored| stored.value() == *current)
                .unwrap_or(false);

            if !matches_current {
                return Ok(false);
            }

            table.insert(next.record_id.as_str(), next).map_err_str(Error::TableAccess)?;
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

    #[test]
    fn verification_prompt_requires_newer_request() {
        let state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Unverified,
            last_verification_requested_at: Some(20),
            last_verification_dismissed_at: Some(10),
            ..PersistedCloudBackupState::default()
        };

        assert!(state.should_prompt_verification());
    }

    #[test]
    fn verification_prompt_respects_dismissal() {
        let state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Unverified,
            last_verification_requested_at: Some(20),
            last_verification_dismissed_at: Some(20),
            ..PersistedCloudBackupState::default()
        };

        assert!(!state.should_prompt_verification());
    }

    #[test]
    fn blob_sync_state_helpers_reflect_state() {
        let confirmed = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: None,
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                revision_hash: "rev-1".into(),
                confirmed_at: 42,
            }),
        };

        assert!(!confirmed.is_dirty());

        let dirty = PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 10 }),
            ..confirmed.clone()
        };

        assert!(dirty.is_dirty());
    }

    #[test]
    fn uploaded_pending_confirmation_tracks_attempts() {
        let state = PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: None,
            record_id: "wallet-a".into(),
            state: PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 3,
                    last_checked_at: Some(12),
                },
            ),
        };

        assert!(state.is_uploaded_pending_confirmation());
    }

    #[test]
    fn failed_blob_state_defaults_retryable_to_false() {
        let failed_state: CloudBlobFailedState = serde_json::from_value(serde_json::json!({
            "revision_hash": "rev-1",
            "error": "offline",
            "failed_at": 42
        }))
        .unwrap();

        assert!(!failed_state.retryable);
        assert_eq!(failed_state.issue, None);
    }
}
