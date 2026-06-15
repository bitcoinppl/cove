use tracing::error;

use super::{PersistedCloudBackupState, PersistedCloudBlobSyncState};
use ::redb::{TableDefinition, TypeName, Value};

pub(crate) const CLOUD_BACKUP_STATE_TABLE: TableDefinition<&'static str, CloudBackupStateJson> =
    TableDefinition::new("cloud_backup_state");
pub(crate) const CLOUD_BLOB_SYNC_STATE_TABLE: TableDefinition<
    &'static str,
    CloudBlobSyncStateJson,
> = TableDefinition::new("cloud_blob_sync_state");

#[derive(Debug)]
pub(crate) struct CloudBackupStateJson;

impl Value for CloudBackupStateJson {
    type SelfType<'a>
        = PersistedCloudBackupState
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        serde_json::from_slice(data).unwrap_or_else(|error| {
            error!("Failed to decode persisted cloud backup state: {error}");
            PersistedCloudBackupState::corrupted("local cloud backup state could not be decoded")
        })
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        serde_json::to_vec(value).expect("failed to serialize")
    }

    fn type_name() -> TypeName {
        // keep the original type path because redb stores it as table metadata
        TypeName::new("SerdeJson<cove::database::cloud_backup::PersistedCloudBackupState>")
    }
}

#[derive(Debug)]
pub(crate) struct CloudBlobSyncStateJson;

impl Value for CloudBlobSyncStateJson {
    type SelfType<'a>
        = PersistedCloudBlobSyncState
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        serde_json::from_slice(data).unwrap_or_else(|error| {
            error!("Failed to decode persisted cloud backup blob sync state: {error}");
            error.into()
        })
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        serde_json::to_vec(value).expect("failed to serialize")
    }

    fn type_name() -> TypeName {
        // keep the original type path because redb stores it as table metadata
        TypeName::new("SerdeJson<cove::database::cloud_backup::PersistedCloudBlobSyncState>")
    }
}

impl From<serde_json::Error> for PersistedCloudBlobSyncState {
    fn from(error: serde_json::Error) -> Self {
        // redb value decoding cannot report an error, so return a typed corrupt tombstone
        PersistedCloudBlobSyncState::corrupted(format!(
            "failed to decode persisted cloud backup blob sync state: {error}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{CloudBlobFailedState, PersistedCloudBlobState};

    #[test]
    fn corrupt_cloud_backup_state_json_decodes_to_corrupted_state() {
        let state = <CloudBackupStateJson as Value>::from_bytes(b"{not json");

        assert_eq!(
            state,
            PersistedCloudBackupState::corrupted("local cloud backup state could not be decoded")
        );
    }

    #[test]
    fn corrupt_cloud_blob_sync_state_json_decodes_to_failed_tombstone() {
        let state = <CloudBlobSyncStateJson as Value>::from_bytes(b"{not json");

        assert!(state.is_corrupted());
        assert!(!state.is_master_key_wrapper());
        assert_eq!(
            state.record_id(),
            crate::database::cloud_backup::state::CORRUPT_BLOB_SYNC_RECORD_ID
        );
        match state.state {
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                retryable,
                issue,
                error,
                ..
            }) => {
                assert!(!retryable);
                assert!(issue.is_none());
                assert!(
                    error.contains("failed to decode persisted cloud backup blob sync state"),
                    "{error}"
                );
            }
            other => panic!("expected failed tombstone, got {other:?}"),
        }
    }
}
