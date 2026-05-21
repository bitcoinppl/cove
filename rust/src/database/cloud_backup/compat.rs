use std::{cmp::Ordering, marker::PhantomData};

use ::redb::{
    Key, ReadableTable as _, TableDefinition, TableError, TypeName, Value, WriteTransaction,
};

use super::tables::{CloudBackupStateJson, CloudBlobSyncStateJson};

const CLOUD_BACKUP_STATE_TABLE_NAME: &str = "cloud_backup_state";
const CLOUD_BLOB_SYNC_STATE_TABLE_NAME: &str = "cloud_blob_sync_state";
const CLOUD_BACKUP_STATE_COMPAT_TEMP_TABLE_NAME: &str = "__cove_cloud_backup_state_type_compat_tmp";
const CLOUD_BLOB_SYNC_STATE_COMPAT_TEMP_TABLE_NAME: &str =
    "__cove_cloud_blob_sync_state_type_compat_tmp";

pub(crate) fn ensure_table_type_compatibility(
    write_txn: &WriteTransaction,
) -> Result<(), TableError> {
    migrate_legacy_table_type::<CloudBackupStateJson, LegacyCloudBackupStateJson>(
        write_txn,
        CLOUD_BACKUP_STATE_TABLE_NAME,
        CLOUD_BACKUP_STATE_COMPAT_TEMP_TABLE_NAME,
    )?;
    migrate_legacy_table_type::<CloudBlobSyncStateJson, LegacyCloudBlobSyncStateJson>(
        write_txn,
        CLOUD_BLOB_SYNC_STATE_TABLE_NAME,
        CLOUD_BLOB_SYNC_STATE_COMPAT_TEMP_TABLE_NAME,
    )?;

    Ok(())
}

fn migrate_legacy_table_type<Canonical, Legacy>(
    write_txn: &WriteTransaction,
    table_name: &'static str,
    temp_table_name: &'static str,
) -> Result<(), TableError>
where
    Canonical: Value + 'static,
    Legacy: for<'a> Value<SelfType<'a> = &'a [u8], AsBytes<'a> = &'a [u8]> + 'static,
{
    let canonical_def =
        TableDefinition::<RawKey<&'static str>, RawValue<Canonical>>::new(table_name);
    let legacy_def = TableDefinition::<RawKey<&'static str>, Legacy>::new(table_name);
    let temp_def = TableDefinition::<RawKey<&'static str>, Legacy>::new(temp_table_name);

    let original_error = match write_txn.open_table(canonical_def) {
        Ok(_) => return Ok(()),
        Err(error @ TableError::TableTypeMismatch { .. }) => error,
        Err(error) => return Err(error),
    };

    {
        match write_txn.open_table(legacy_def) {
            Ok(_) => {}
            Err(_) => return Err(original_error),
        }
    }

    let _ = write_txn.delete_table(temp_def)?;
    write_txn.rename_table(legacy_def, temp_def)?;

    let rows = {
        let table = write_txn.open_table(temp_def)?;
        let mut rows = Vec::new();

        for entry in table.iter()? {
            let (key, value) = entry?;
            rows.push((key.value().to_vec(), value.value().to_vec()));
        }

        rows
    };

    {
        let mut table = write_txn.open_table(canonical_def)?;

        for (key, value) in rows {
            table.insert(key.as_slice(), value.as_slice())?;
        }
    }

    write_txn.delete_table(temp_def)?;

    Ok(())
}

#[derive(Debug)]
struct RawKey<K: Key>(PhantomData<K>);

impl<K: Key + 'static> Value for RawKey<K> {
    type SelfType<'a>
        = &'a [u8]
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        K::fixed_width()
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        K::type_name()
    }
}

impl<K: Key + 'static> Key for RawKey<K> {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        K::compare(data1, data2)
    }
}

#[derive(Debug)]
struct RawValue<V: Value>(PhantomData<V>);

impl<V: Value + 'static> Value for RawValue<V> {
    type SelfType<'a>
        = &'a [u8]
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        V::fixed_width()
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        V::type_name()
    }
}

#[derive(Debug)]
struct LegacyCloudBackupStateJson;

impl Value for LegacyCloudBackupStateJson {
    type SelfType<'a>
        = &'a [u8]
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        TypeName::new("SerdeJson<cove::database::cloud_backup::state::PersistedCloudBackupState>")
    }
}

#[derive(Debug)]
struct LegacyCloudBlobSyncStateJson;

impl Value for LegacyCloudBlobSyncStateJson {
    type SelfType<'a>
        = &'a [u8]
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        TypeName::new("SerdeJson<cove::database::cloud_backup::state::PersistedCloudBlobSyncState>")
    }
}

#[cfg(test)]
mod tests {
    use ::redb::TableDefinition;
    use tempfile::TempDir;

    use super::*;
    use crate::database::cloud_backup::{
        CLOUD_BACKUP_STATE_TABLE, CLOUD_BLOB_SYNC_STATE_TABLE, PersistedCloudBackupState,
    };

    fn create_db() -> (TempDir, redb::Database) {
        let dir = TempDir::new().unwrap();
        let db = redb::Database::create(dir.path().join("test.redb")).unwrap();

        (dir, db)
    }

    #[test]
    fn missing_tables_are_created_with_canonical_metadata() {
        let (_dir, db) = create_db();
        let write_txn = db.begin_write().unwrap();

        ensure_table_type_compatibility(&write_txn).unwrap();
        write_txn.commit().unwrap();

        let read_txn = db.begin_read().unwrap();
        read_txn.open_table(CLOUD_BACKUP_STATE_TABLE).unwrap();
        read_txn.open_table(CLOUD_BLOB_SYNC_STATE_TABLE).unwrap();
    }

    #[test]
    fn canonical_tables_are_left_readable() {
        let (_dir, db) = create_db();
        let state = PersistedCloudBackupState::Disabled;
        let write_txn = db.begin_write().unwrap();

        {
            let mut table = write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).unwrap();
            table.insert("current", &state).unwrap();
        }

        ensure_table_type_compatibility(&write_txn).unwrap();
        write_txn.commit().unwrap();

        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(CLOUD_BACKUP_STATE_TABLE).unwrap();
        let stored = table.get("current").unwrap().unwrap();
        assert_eq!(stored.value(), state);
    }

    #[test]
    fn legacy_cloud_backup_state_table_migrates_to_canonical_metadata() {
        let (_dir, db) = create_db();
        let state = PersistedCloudBackupState::Disabled;
        let value = serde_json::to_vec(&state).unwrap();
        let legacy_def = TableDefinition::<RawKey<&'static str>, LegacyCloudBackupStateJson>::new(
            CLOUD_BACKUP_STATE_TABLE_NAME,
        );
        let write_txn = db.begin_write().unwrap();

        {
            let mut table = write_txn.open_table(legacy_def).unwrap();
            table.insert(b"current" as &[u8], value.as_slice()).unwrap();
        }

        write_txn.commit().unwrap();

        let write_txn = db.begin_write().unwrap();
        ensure_table_type_compatibility(&write_txn).unwrap();
        write_txn.commit().unwrap();

        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(CLOUD_BACKUP_STATE_TABLE).unwrap();
        let stored = table.get("current").unwrap().unwrap();
        assert_eq!(stored.value(), state);
    }

    #[test]
    fn legacy_cloud_blob_sync_table_migrates_to_canonical_metadata() {
        let (_dir, db) = create_db();
        let value = br#"{"legacy":"payload"}"#;
        let legacy_def = TableDefinition::<RawKey<&'static str>, LegacyCloudBlobSyncStateJson>::new(
            CLOUD_BLOB_SYNC_STATE_TABLE_NAME,
        );
        let canonical_raw_def = TableDefinition::<
            RawKey<&'static str>,
            RawValue<CloudBlobSyncStateJson>,
        >::new(CLOUD_BLOB_SYNC_STATE_TABLE_NAME);
        let write_txn = db.begin_write().unwrap();

        {
            let mut table = write_txn.open_table(legacy_def).unwrap();
            table.insert(b"record-id" as &[u8], value as &[u8]).unwrap();
        }

        write_txn.commit().unwrap();

        let write_txn = db.begin_write().unwrap();
        ensure_table_type_compatibility(&write_txn).unwrap();
        write_txn.commit().unwrap();

        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(canonical_raw_def).unwrap();
        let stored = table.get(b"record-id" as &[u8]).unwrap().unwrap();
        assert_eq!(stored.value(), value);
    }

    #[test]
    fn unrelated_table_type_mismatch_is_preserved() {
        #[derive(Debug)]
        struct WrongCloudBackupStateJson;

        impl Value for WrongCloudBackupStateJson {
            type SelfType<'a>
                = &'a [u8]
            where
                Self: 'a;

            type AsBytes<'a>
                = &'a [u8]
            where
                Self: 'a;

            fn fixed_width() -> Option<usize> {
                None
            }

            fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
            where
                Self: 'a,
            {
                data
            }

            fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
            where
                Self: 'b,
            {
                value
            }

            fn type_name() -> TypeName {
                TypeName::new("wrong-cloud-backup-state-json")
            }
        }

        let (_dir, db) = create_db();
        let wrong_def = TableDefinition::<RawKey<&'static str>, WrongCloudBackupStateJson>::new(
            CLOUD_BACKUP_STATE_TABLE_NAME,
        );
        let write_txn = db.begin_write().unwrap();

        {
            let mut table = write_txn.open_table(wrong_def).unwrap();
            table.insert(b"current" as &[u8], b"{}" as &[u8]).unwrap();
        }

        let error = ensure_table_type_compatibility(&write_txn).unwrap_err();
        assert!(matches!(error, TableError::TableTypeMismatch { .. }));
    }
}
