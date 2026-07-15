use std::sync::Arc;

use ::redb::{Database as RedbDatabase, ReadableTable as _, WriteTransaction};

use cove_util::result_ext::ResultExt as _;

use super::Error;

mod compat;
mod compatibility;
mod state;
mod tables;

pub(crate) use compat::ensure_table_type_compatibility;
pub use state::{
    CloudBackupRecordKey, CloudBlobConfirmedState, CloudBlobDirtyState, CloudBlobFailedState,
    CloudBlobFailureIssue, CloudBlobUploadedPendingConfirmationState, CloudBlobUploadingState,
    PersistedBackupSyncState, PersistedBackupVerificationState, PersistedCloudBackupState,
    PersistedCloudBackupStatus, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    PersistedConfiguredCloudBackup, PersistedDeepVerificationReport, PersistedDisablingCloudBackup,
    PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase, PersistedPasskeyState,
    PersistedPendingVerificationCompletion, PersistedPendingVerificationUpload,
};
pub(crate) use tables::{CLOUD_BACKUP_STATE_TABLE, CLOUD_BLOB_SYNC_STATE_TABLE};

const CURRENT_KEY: &str = "current";

#[derive(Debug, Clone)]
pub struct CloudBackupStateTable {
    db: Arc<RedbDatabase>,
}

impl CloudBackupStateTable {
    pub fn new(db: Arc<RedbDatabase>, write_txn: &WriteTransaction) -> Self {
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
    db: Arc<RedbDatabase>,
}

impl CloudBlobSyncStateTable {
    pub fn new(db: Arc<RedbDatabase>, write_txn: &WriteTransaction) -> Self {
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
            drive_account_switch: None,
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
