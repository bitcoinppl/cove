use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::path::Path;

use eyre::{Context as _, Result, bail};
use redb::{ReadableTable as _, TableDefinition, TableHandle as _, TypeName};
use tracing::{info, warn};

use crate::database::encrypted_backend::EncryptedBackend;

/// Wrapper that reads/writes raw bytes while matching V's type_name
///
/// During migration we only move bytes between databases, no deserialization
/// needed. This avoids panics from stale enum variants or missing serde fields
/// that would trigger `expect()` in `Json<T>::from_bytes` / `Cbor<T>::from_bytes`
#[derive(Debug)]
pub(super) struct RawValue<V: redb::Value>(PhantomData<V>);

impl<V: redb::Value + 'static> redb::Value for RawValue<V> {
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

    fn from_bytes<'a>(data: &'a [u8]) -> &'a [u8]
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> &'a [u8]
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        V::type_name()
    }
}

/// Wrapper that reads/writes raw key bytes while matching K's type_name and compare
#[derive(Debug)]
pub(super) struct RawKey<K: redb::Key>(PhantomData<K>);

impl<K: redb::Key + 'static> redb::Value for RawKey<K> {
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

    fn from_bytes<'a>(data: &'a [u8]) -> &'a [u8]
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> &'a [u8]
    where
        Self: 'b,
    {
        value
    }

    fn type_name() -> TypeName {
        K::type_name()
    }
}

impl<K: redb::Key + 'static> redb::Key for RawKey<K> {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        // delegates to original comparator for BTree ordering correctness
        // safe for migration: key bytes in the source BTree were successfully
        // written and iterated, so K::compare won't encounter unknown formats
        // (key schemas don't evolve like value schemas do)
        K::compare(data1, data2)
    }
}

/// Copy all rows from one table in src_db to the same table in dst_db
///
/// Uses raw byte wrappers to avoid deserializing keys/values during copy;
/// this prevents panics from stale records with outdated schema
pub(super) fn copy_table<K, V>(
    src_db: &redb::Database,
    dst_db: &redb::Database,
    table_def: TableDefinition<K, V>,
) -> Result<u64>
where
    K: redb::Key + 'static,
    V: redb::Value + 'static,
{
    let name = table_def.name();
    let raw_def = TableDefinition::<RawKey<K>, RawValue<V>>::new(name);

    let read_txn = src_db.begin_read().context("failed to begin read on source")?;

    let src_table = match read_txn.open_table(raw_def) {
        Ok(table) => table,
        // table doesn't exist in source, nothing to copy
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
        Err(e) => return Err(e).context("failed to open source table"),
    };

    let write_txn = dst_db.begin_write().context("failed to begin write on destination")?;
    let mut count = 0u64;

    {
        let mut dst_table = write_txn.open_table(raw_def).context("failed to open dest table")?;

        for entry in src_table.iter().context("failed to iterate source table")? {
            let (key, value) = entry.context("failed to read entry")?;
            dst_table.insert(key.value(), value.value()).context("failed to insert entry")?;
            count += 1;
        }
    }

    write_txn.commit().context("failed to commit write")?;

    info!("Copied table '{name}': {count} rows");
    Ok(count)
}

pub(crate) fn verify_all_source_tables_copied(
    src_db: &redb::Database,
    dst_db: &redb::Database,
) -> Result<()> {
    let source_tables = table_names(src_db).context("failed to list source tables")?;
    let dest_tables = table_names(dst_db).context("failed to list destination tables")?;

    let missing = source_tables.difference(&dest_tables).map(String::as_str).collect::<Vec<_>>();

    if !missing.is_empty() {
        bail!("encrypted migration missed source table(s): {}", missing.join(", "));
    }

    Ok(())
}

fn table_names(db: &redb::Database) -> Result<BTreeSet<String>> {
    let read_txn = db.begin_read().context("failed to begin table listing read")?;
    let tables = read_txn.list_tables().context("failed to list tables")?;

    Ok(tables.map(|handle| handle.name().to_string()).collect())
}

/// Check whether an encrypted redb database can be opened and read
///
/// Returns `Ok(true)` if verified, `Ok(false)` if corrupt, `Err` for I/O errors
pub(super) fn verify_encrypted_redb_db(path: &Path) -> Result<bool> {
    let path_display = path.display();

    let key = crate::database::encrypted_backend::encryption_key().ok_or_else(|| {
        eyre::eyre!("no encryption key available for verification of {path_display}")
    })?;

    let backend = match EncryptedBackend::open(path, &key) {
        Ok(b) => b,
        Err(e) => {
            warn!("Verification failed for {path_display}: could not open encrypted backend: {e}");
            return Ok(false);
        }
    };

    let db = match redb::Database::builder().create_with_backend(backend) {
        Ok(db) => db,
        Err(e) => {
            warn!("Verification failed for {path_display}: could not create database: {e}");
            return Ok(false);
        }
    };

    let read_txn = match db.begin_read() {
        Ok(txn) => txn,
        Err(e) => {
            warn!("Verification failed for {path_display}: could not begin read transaction: {e}");
            return Ok(false);
        }
    };

    if let Err(e) = read_txn.list_tables() {
        warn!("Verification failed for {path_display}: could not list tables: {e}");
        return Ok(false);
    }

    Ok(true)
}
