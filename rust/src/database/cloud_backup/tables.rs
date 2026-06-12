use tracing::error;

use super::{
    CloudBlobFailedState, PersistedCloudBackupState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use ::redb::{TableDefinition, TypeName, Value};

pub(crate) const CLOUD_BACKUP_STATE_TABLE: TableDefinition<&'static str, CloudBackupStateJson> =
    TableDefinition::new("cloud_backup_state");
pub(crate) const CLOUD_BLOB_SYNC_STATE_TABLE: TableDefinition<
    &'static str,
    CloudBlobSyncStateJson,
> = TableDefinition::new("cloud_blob_sync_state");

const CORRUPT_BLOB_SYNC_NAMESPACE_ID: &str = "__corrupt_cloud_backup_blob_sync_state__";

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
            PersistedCloudBackupState::Disabled
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
        // redb value decoding cannot report an error, so return a non-trusting tombstone
        PersistedCloudBlobSyncState::master_key_wrapper(
            CORRUPT_BLOB_SYNC_NAMESPACE_ID.into(),
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                retryable: false,
                issue: None,
                error: format!("failed to decode persisted cloud backup blob sync state: {error}"),
                failed_at: 0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_cloud_backup_state_json_decodes_to_disabled() {
        let state = <CloudBackupStateJson as Value>::from_bytes(b"{not json");

        assert_eq!(state, PersistedCloudBackupState::Disabled);
    }

    #[test]
    fn corrupt_cloud_blob_sync_state_json_decodes_to_failed_tombstone() {
        let state = <CloudBlobSyncStateJson as Value>::from_bytes(b"{not json");

        assert_eq!(state.namespace_id, CORRUPT_BLOB_SYNC_NAMESPACE_ID);
        assert!(state.is_master_key_wrapper());
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
