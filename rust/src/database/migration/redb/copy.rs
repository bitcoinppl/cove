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
pub(crate) struct RawValue<V: redb::Value>(PhantomData<V>);

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
pub(crate) struct RawKey<K: redb::Key>(PhantomData<K>);

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
pub(crate) fn copy_table<K, V>(
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

    let read_txn = src_db
        .begin_read()
        .with_context(|| format!("failed to begin read on source for {name}"))?;

    let src_table = match read_txn.open_table(raw_def) {
        Ok(table) => table,
        // table doesn't exist in source, nothing to copy
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("failed to open source table {name}")),
    };

    let write_txn = dst_db
        .begin_write()
        .with_context(|| format!("failed to begin write on destination for {name}"))?;
    let mut count = 0u64;

    {
        let mut dst_table = write_txn
            .open_table(raw_def)
            .with_context(|| format!("failed to open destination table {name}"))?;

        for entry in
            src_table.iter().with_context(|| format!("failed to iterate source table {name}"))?
        {
            let (key, value) =
                entry.with_context(|| format!("failed to read entry from {name}"))?;
            dst_table
                .insert(key.value(), value.value())
                .with_context(|| format!("failed to insert entry into {name}"))?;
            count += 1;
        }
    }

    write_txn.commit().with_context(|| format!("failed to commit write for {name}"))?;

    info!("Copied table '{name}': {count} rows");
    Ok(count)
}

pub(crate) struct TableCopyPolicy {
    pub(crate) database_kind: &'static str,
    pub(crate) source_path: std::path::PathBuf,
    pub(crate) current_tables: BTreeSet<&'static str>,
    pub(crate) known_historical_tables: BTreeSet<&'static str>,
    pub(crate) disposable_skipped_tables: BTreeSet<&'static str>,
}

#[derive(Debug)]
pub(crate) struct TableCopyReport {
    pub(crate) skipped_known_historical: Vec<String>,
    pub(crate) skipped_unknown: Vec<String>,
    pub(crate) may_remove_source: bool,
}

impl TableCopyReport {
    pub(crate) fn log_skipped_tables(&self, policy: &TableCopyPolicy) {
        if self.skipped_known_historical.is_empty() && self.skipped_unknown.is_empty() {
            return;
        }

        let source = policy.source_path.display();
        warn!(
            "Skipped non-current redb table(s) during {} migration for {source}; known_historical=[{}]; unknown=[{}]",
            policy.database_kind,
            self.skipped_known_historical.join(", "),
            self.skipped_unknown.join(", "),
        );
    }
}

pub(crate) fn verify_current_source_tables_copied(
    src_db: &redb::Database,
    dst_db: &redb::Database,
    policy: &TableCopyPolicy,
) -> Result<TableCopyReport> {
    let source_tables = table_names(src_db).context("failed to list source tables")?;
    let dest_tables = table_names(dst_db).context("failed to list destination tables")?;
    let current_tables =
        policy.current_tables.iter().map(|table| (*table).to_string()).collect::<BTreeSet<_>>();
    let known_historical_tables = policy
        .known_historical_tables
        .iter()
        .map(|table| (*table).to_string())
        .collect::<BTreeSet<_>>();

    let missing_current = source_tables
        .intersection(&current_tables)
        .filter(|table| !dest_tables.contains(*table))
        .map(String::as_str)
        .collect::<Vec<_>>();

    if !missing_current.is_empty() {
        let source = policy.source_path.display();
        bail!(
            "{} encrypted migration missed current source table(s) at {source}: {}",
            policy.database_kind,
            missing_current.join(", ")
        );
    }

    let skipped_known_historical =
        source_tables.intersection(&known_historical_tables).cloned().collect::<Vec<_>>();
    let skipped_unknown = source_tables
        .difference(&current_tables)
        .filter(|table| !known_historical_tables.contains(*table))
        .cloned()
        .collect::<Vec<_>>();

    let skipped_tables = skipped_known_historical
        .iter()
        .chain(skipped_unknown.iter())
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let may_remove_source =
        skipped_tables.iter().all(|table| policy.disposable_skipped_tables.contains(table));

    Ok(TableCopyReport { skipped_known_historical, skipped_unknown, may_remove_source })
}

pub(crate) fn table_names(db: &redb::Database) -> Result<BTreeSet<String>> {
    let read_txn = db.begin_read().context("failed to begin table listing read")?;
    let tables = read_txn.list_tables().context("failed to list tables")?;

    Ok(tables.map(|handle| handle.name().to_string()).collect())
}

/// Check whether an encrypted redb database can be opened and read
///
/// Returns `Ok(true)` if verified, `Ok(false)` if corrupt, `Err` for I/O errors
pub(crate) fn verify_encrypted_redb_db(path: &Path) -> Result<bool> {
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
