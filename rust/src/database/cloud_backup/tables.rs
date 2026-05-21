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
        serde_json::from_slice(data).expect("failed to deserialize")
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
        serde_json::from_slice(data).expect("failed to deserialize")
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
