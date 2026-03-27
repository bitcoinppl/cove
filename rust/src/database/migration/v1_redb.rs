use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::{Context as _, Result};
use redb::{ReadableTable as _, TableDefinition, TableHandle as _, TypeName};
use tracing::{error, info, warn};

use crate::bootstrap::Migration;
use crate::database::encrypted_backend::EncryptedBackend;
use cove_common::consts::{ROOT_DATA_DIR, WALLET_DATA_DIR};

use super::log_remove_file;

/// Wrapper that reads/writes raw bytes while matching V's type_name
///
/// During migration we only move bytes between databases, no deserialization
/// needed. This avoids panics from stale enum variants or missing serde fields
/// that would trigger `expect()` in `Json<T>::from_bytes` / `Cbor<T>::from_bytes`
#[derive(Debug)]
struct RawValue<V: redb::Value>(PhantomData<V>);

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
struct RawKey<K: redb::Key>(PhantomData<K>);

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
fn copy_table<K, V>(
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

/// Check whether an encrypted redb database can be opened and read
///
/// Returns `Ok(true)` if verified, `Ok(false)` if corrupt, `Err` for I/O errors
fn verify_encrypted_redb_db(path: &Path) -> Result<bool> {
    let path_display = path.display();

    let key = crate::database::encrypted_backend::encryption_key().ok_or_else(|| {
        eyre::eyre!("no encryption key available for verification of {path_display}")
    })?;

    let backend = match EncryptedBackend::open(path, key) {
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

/// Recover from interrupted migrations by checking for .bak/.enc.tmp files
pub fn recover_interrupted_migrations() -> Result<()> {
    recover_at_path(&ROOT_DATA_DIR.join("cove.db"))?;

    let entries = match std::fs::read_dir(&*WALLET_DATA_DIR) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(e).context("failed to read wallet data directory for recovery");
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry: {e}");
                continue;
            }
        };
        recover_at_path(&wallet_db_path(&entry.path()))?;
    }

    Ok(())
}

fn migration_paths(db_path: &Path) -> (PathBuf, PathBuf) {
    let extension = db_path.extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default();
    (
        db_path.with_extension(format!("{extension}.bak")),
        db_path.with_extension(format!("{extension}.enc.tmp")),
    )
}

fn recover_at_path(db_path: &Path) -> Result<()> {
    let (bak_path, tmp_path) = migration_paths(db_path);

    // only backup exists: migration completed but final rename didn't happen
    if bak_path.exists() && !db_path.exists() && !tmp_path.exists() {
        let bak = bak_path.display();
        warn!("Only backup exists at {bak} -- restoring from backup");
        std::fs::rename(&bak_path, db_path)
            .context(format!("failed to restore from backup at {bak}"))?;
        return Ok(());
    }

    if tmp_path.exists() && bak_path.exists() && !db_path.exists() {
        // crash after old→.bak, before tmp→final: finish the rename
        let path = db_path.display();
        info!("Recovering interrupted migration at {path}");
        std::fs::rename(&tmp_path, db_path)
            .context(format!("failed to finish interrupted migration at {path}"))?;
    }

    // clean up leftover .enc.tmp (crash during copy)
    if tmp_path.exists() {
        log_remove_file(&tmp_path);
    }

    // clean up leftover .bak only after verifying the encrypted DB works
    if bak_path.exists() && db_path.exists() {
        match verify_encrypted_redb_db(db_path) {
            Ok(true) => {
                log_remove_file(&bak_path);
            }
            Ok(false) => {
                let path = db_path.display();
                warn!("Encrypted DB at {path} appears corrupt, restoring from backup");
                log_remove_file(db_path);
                std::fs::rename(&bak_path, db_path).context("failed to restore from backup")?;
            }
            Err(e) => {
                // I/O error — preserve both files so nothing is lost
                let path = db_path.display();
                warn!("Cannot verify encrypted DB at {path}: {e:#} — preserving both files");
            }
        }
    }

    Ok(())
}

/// Check whether the main database needs migration (exists and is unencrypted)
pub fn main_database_needs_migration() -> bool {
    let db_path = ROOT_DATA_DIR.join("cove.db");
    db_path.exists() && !EncryptedBackend::is_encrypted(&db_path)
}

fn wallet_db_path(wallet_dir: &Path) -> PathBuf {
    wallet_dir.join("wallet_data.json")
}

/// Check whether a wallet subdirectory has an unencrypted wallet_data.json
fn needs_redb_migration(wallet_dir: &Path) -> bool {
    let wallet_db = wallet_db_path(wallet_dir);
    wallet_db.exists() && !EncryptedBackend::is_encrypted(&wallet_db)
}

/// Count wallet subdirectories that have an unencrypted wallet_data.json
///
/// Best-effort: returns 0 if the directory is unreadable. The actual
/// migration in `WalletMigration::run()` will surface any real I/O errors
pub fn count_redb_wallets_needing_migration() -> u32 {
    let entries = match std::fs::read_dir(&*WALLET_DATA_DIR) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            error!("Failed to read wallet data directory for migration count: {e}");
            return 0;
        }
    };

    let mut count = 0u32;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry during redb migration count: {e}");
                continue;
            }
        };
        if needs_redb_migration(&entry.path()) {
            count += 1;
        }
    }
    count
}

/// Migrate the main redb database from plaintext to encrypted if needed
///
/// Returns Ok(true) if migration was performed, Ok(false) if already encrypted or new
pub fn migrate_main_database_if_needed() -> Result<bool> {
    let db_path = ROOT_DATA_DIR.join("cove.db");
    if !db_path.exists() || EncryptedBackend::is_encrypted(&db_path) {
        return Ok(false);
    }

    info!("Migrating main database to encrypted format");
    migrate_main_database(&db_path)
}

/// Create a new encrypted redb database at `tmp_path` for migration
fn create_encrypted_dst(tmp_path: &Path) -> Result<redb::Database> {
    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before migration"))?;

    let backend =
        EncryptedBackend::create(tmp_path, key).context("failed to create encrypted database")?;

    redb::Database::builder()
        .create_with_file_format_v3(true)
        .create_with_backend(backend)
        .context("failed to init encrypted database")
}

/// Verify that an encrypted redb database at `path` can be opened and read
fn verify_encrypted_dst(path: &Path) -> Result<()> {
    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before migration"))?;
    let verify_backend = EncryptedBackend::open(path, key)
        .context("verification: cannot reopen encrypted database")?;
    let verify_db = redb::Database::builder()
        .create_with_backend(verify_backend)
        .context("verification: cannot init encrypted database")?;
    let _read = verify_db.begin_read().context("verification: encrypted database not readable")?;
    Ok(())
}

/// Two-phase swap: old → .bak, then tmp → original
///
/// The .bak is retained until the next recovery pass verifies the encrypted DB works
fn two_phase_swap(db_path: &Path, bak_path: &Path, tmp_path: &Path) -> Result<()> {
    std::fs::rename(db_path, bak_path).context("failed to rename old database to .bak")?;
    std::fs::rename(tmp_path, db_path).context("failed to rename encrypted database into place")?;
    Ok(())
}

fn migrate_main_database(db_path: &Path) -> Result<bool> {
    let src_db = redb::Database::open(db_path).context("failed to open plaintext main database")?;
    let (bak_path, tmp_path) = migration_paths(db_path);
    let dst_db = create_encrypted_dst(&tmp_path)?;

    copy_table(&src_db, &dst_db, crate::database::global_flag::TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::global_config::TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::global_cache::TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::wallet::TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::unsigned_transactions::MAIN_TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::unsigned_transactions::BY_WALLET_TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::historical_price::TABLE)?;

    drop(src_db);
    drop(dst_db);

    verify_encrypted_dst(&tmp_path)?;
    two_phase_swap(db_path, &bak_path, &tmp_path)?;

    info!("Main database migration complete");
    Ok(true)
}

pub struct WalletMigration {
    dir: PathBuf,
    migration: Arc<Migration>,
}

impl WalletMigration {
    pub fn new(migration: Arc<Migration>) -> Self {
        Self { dir: WALLET_DATA_DIR.to_path_buf(), migration }
    }

    pub fn run(&self) -> Result<()> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e).context("failed to read wallet data directory");
            }
        };

        let mut errors: Vec<String> = Vec::new();

        for entry in entries {
            if self.migration.is_cancelled() {
                info!("Wallet database migration cancelled");
                eyre::bail!("wallet migration cancelled");
            }

            let entry = entry.context("failed to read directory entry during wallet migration")?;
            if needs_redb_migration(&entry.path()) {
                let wallet_db = wallet_db_path(&entry.path());
                let db_display = wallet_db.display();
                info!("Migrating wallet database at {db_display}");
                match migrate_wallet_database(&wallet_db) {
                    Ok(()) => self.migration.tick(),
                    Err(e) => {
                        error!("Failed to migrate wallet database {db_display}: {e:#}");
                        errors.push(format!("{db_display}: {e:#}"));
                        // tick even on failure to keep progress bar advancing and prevent watchdog timeout
                        self.migration.tick();
                    }
                }
            }
        }

        if !errors.is_empty() {
            let count = errors.len();
            let details = errors.join("; ");
            eyre::bail!("failed to migrate {count} wallet database(s): {details}");
        }

        Ok(())
    }
}

fn migrate_wallet_database(db_path: &Path) -> Result<()> {
    let src_db =
        redb::Database::open(db_path).context("failed to open plaintext wallet database")?;
    let (bak_path, tmp_path) = migration_paths(db_path);
    let dst_db = create_encrypted_dst(&tmp_path)?;

    copy_table(&src_db, &dst_db, crate::database::wallet_data::TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::wallet_data::label::TXN_TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::wallet_data::label::ADDRESS_TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::wallet_data::label::INPUT_TABLE)?;
    copy_table(&src_db, &dst_db, crate::database::wallet_data::label::OUTPUT_TABLE)?;

    drop(src_db);
    drop(dst_db);

    verify_encrypted_dst(&tmp_path)?;
    two_phase_swap(db_path, &bak_path, &tmp_path)?;

    let path = db_path.display();
    info!("Wallet database migration complete at {path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use redb::ReadableTableMetadata as _;

    use super::*;
    use crate::database::{
        encrypted_backend, global_cache, global_config, global_flag, historical_price,
        unsigned_transactions, wallet, wallet_data,
    };
    use tempfile::TempDir;

    impl WalletMigration {
        fn with_dir(dir: PathBuf, migration: Arc<Migration>) -> Self {
            Self { dir, migration }
        }
    }

    fn setup_test_key() {
        encrypted_backend::set_test_encryption_key();
    }

    fn create_encrypted_redb_at(path: &Path) {
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::create(path, key).unwrap();
        let db = redb::Database::builder()
            .create_with_file_format_v3(true)
            .create_with_backend(backend)
            .unwrap();
        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(global_flag::TABLE).unwrap();
            table.insert("test_flag", true).unwrap();
        }
        write_txn.commit().unwrap();
    }

    fn create_plaintext_main_db(dir: &TempDir) -> PathBuf {
        let path = dir.path().join("cove.db");
        let db = redb::Database::create(&path).unwrap();

        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(global_flag::TABLE).unwrap();
            table.insert("test_flag", true).unwrap();
        }
        {
            let mut table = write_txn.open_table(global_config::TABLE).unwrap();
            table.insert("test_config", String::from("test_value")).unwrap();
        }
        write_txn.commit().unwrap();

        drop(db);
        path
    }

    fn create_plaintext_wallet_db(dir: &TempDir) -> PathBuf {
        let wallet_dir = dir.path().join("test_wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();
        let path = wallet_dir.join("wallet_data.json");

        let db = redb::Database::create(&path).unwrap();
        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(wallet_data::TABLE).unwrap();
            table
                .insert(
                    "scan_state_native_segwit",
                    wallet_data::WalletData::ScanState(wallet_data::ScanState::Completed),
                )
                .unwrap();
        }
        write_txn.commit().unwrap();

        drop(db);
        path
    }

    #[test]
    fn copy_table_basic() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let src_path = dir.path().join("src.db");
        let dst_path = dir.path().join("dst.db");

        let table_def: TableDefinition<&str, &str> = TableDefinition::new("test");

        let src_db = redb::Database::create(&src_path).unwrap();
        {
            let write_txn = src_db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                table.insert("key1", "value1").unwrap();
                table.insert("key2", "value2").unwrap();
            }
            write_txn.commit().unwrap();
        }

        let dst_db = redb::Database::create(&dst_path).unwrap();
        let count = copy_table(&src_db, &dst_db, table_def).unwrap();
        assert_eq!(count, 2);

        let read_txn = dst_db.begin_read().unwrap();
        let table = read_txn.open_table(table_def).unwrap();
        assert_eq!(table.get("key1").unwrap().unwrap().value(), "value1");
        assert_eq!(table.get("key2").unwrap().unwrap().value(), "value2");
    }

    #[test]
    fn copy_table_missing_source_table() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let src_path = dir.path().join("src.db");
        let dst_path = dir.path().join("dst.db");

        let table_def: TableDefinition<&str, &str> = TableDefinition::new("nonexistent");

        let src_db = redb::Database::create(&src_path).unwrap();
        let dst_db = redb::Database::create(&dst_path).unwrap();

        let count = copy_table(&src_db, &dst_db, table_def).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migrate_main_database_roundtrip() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);

        assert!(!EncryptedBackend::is_encrypted(&db_path));

        migrate_main_database(&db_path).unwrap();

        assert!(EncryptedBackend::is_encrypted(&db_path));

        // verify data survived migration
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&db_path, key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();

        let read_txn = db.begin_read().unwrap();
        {
            let table = read_txn.open_table(global_flag::TABLE).unwrap();
            assert!(table.get("test_flag").unwrap().unwrap().value());
        }
        {
            let table = read_txn.open_table(global_config::TABLE).unwrap();
            assert_eq!(table.get("test_config").unwrap().unwrap().value(), "test_value");
        }
    }

    #[test]
    fn migrate_wallet_database_roundtrip() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_wallet_db(&dir);

        assert!(!EncryptedBackend::is_encrypted(&db_path));

        migrate_wallet_database(&db_path).unwrap();

        assert!(EncryptedBackend::is_encrypted(&db_path));

        // verify data survived
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&db_path, key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();

        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(wallet_data::TABLE).unwrap();
        assert!(table.get("scan_state_native_segwit").unwrap().is_some());
    }

    #[test]
    fn migrate_idempotent() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);

        // first migration
        migrate_main_database(&db_path).unwrap();
        assert!(EncryptedBackend::is_encrypted(&db_path));

        // second attempt should recognize it's already encrypted
        // open_or_create_database handles this, but the migration check itself:
        assert!(EncryptedBackend::is_encrypted(&db_path));
    }

    #[test]
    fn recover_crash_during_copy() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let tmp_path = dir.path().join("cove.db.enc.tmp");

        // simulate crash during copy: original exists, partial .enc.tmp exists
        std::fs::write(&db_path, b"original").unwrap();
        std::fs::write(&tmp_path, b"partial").unwrap();

        recover_at_path(&db_path).unwrap();

        // .enc.tmp should be cleaned up, original untouched
        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"original");
    }

    #[test]
    fn recover_crash_after_bak_rename() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let (bak_path, tmp_path) = migration_paths(&db_path);

        // simulate crash after old→.bak, before tmp→final
        std::fs::write(&bak_path, b"old_data").unwrap();
        create_encrypted_redb_at(&tmp_path);

        recover_at_path(&db_path).unwrap();

        // should finish the rename and verify, then clean up .bak
        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert!(!bak_path.exists());
        assert!(verify_encrypted_redb_db(&db_path).unwrap());
    }

    #[test]
    fn recover_leftover_bak() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);

        // migrate to create a real encrypted DB
        migrate_main_database(&db_path).unwrap();

        // simulate leftover .bak from a completed migration
        let (bak_path, _) = migration_paths(&db_path);
        assert!(bak_path.exists(), ".bak should exist after migration");

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!bak_path.exists(), ".bak should be cleaned after verification");
    }

    #[test]
    fn recover_restores_bak_when_encrypted_db_invalid() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let (bak_path, _) = migration_paths(&db_path);

        // write garbage to db_path (simulating corruption)
        std::fs::write(&db_path, b"corrupt_encrypted_data").unwrap();
        std::fs::write(&bak_path, b"old_plaintext").unwrap();

        recover_at_path(&db_path).unwrap();

        // should have restored from backup
        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"old_plaintext");
    }

    #[test]
    fn bak_retained_after_migration() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);
        let (bak_path, _) = migration_paths(&db_path);

        migrate_main_database(&db_path).unwrap();

        assert!(db_path.exists(), "encrypted DB should exist");
        assert!(bak_path.exists(), ".bak should be retained after migration");
        assert!(EncryptedBackend::is_encrypted(&db_path));
    }

    #[test]
    fn bak_cleaned_on_recovery_when_encrypted_db_valid() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);
        let (bak_path, _) = migration_paths(&db_path);

        migrate_main_database(&db_path).unwrap();
        assert!(bak_path.exists());

        // simulate next launch recovery
        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists(), "encrypted DB should still exist");
        assert!(!bak_path.exists(), ".bak should be cleaned after verification");
    }

    #[test]
    fn recover_wallet_crash_during_copy() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let wallet_dir = dir.path().join("wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();

        let db_path = wallet_dir.join("wallet_data.json");
        let tmp_path = wallet_dir.join("wallet_data.json.enc.tmp");

        std::fs::write(&db_path, b"original").unwrap();
        std::fs::write(&tmp_path, b"partial").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"original");
    }

    #[test]
    fn recover_wallet_crash_after_bak_rename() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let wallet_dir = dir.path().join("wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();

        let db_path = wallet_dir.join("wallet_data.json");
        let (bak_path, tmp_path) = migration_paths(&db_path);

        std::fs::write(&bak_path, b"old_data").unwrap();
        create_encrypted_redb_at(&tmp_path);

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert!(!bak_path.exists());
        assert!(verify_encrypted_redb_db(&db_path).unwrap());
    }

    #[test]
    fn copy_table_with_malformed_json_values() {
        use cove_types::redb::Json;

        setup_test_key();

        let dir = TempDir::new().unwrap();
        let src_path = dir.path().join("src.db");
        let dst_path = dir.path().join("dst.db");

        // write raw bytes that would panic if deserialized as WalletData
        // simulates old records with stale enum variants like {"Balance":...}
        let typed_def = wallet_data::TABLE;

        let src_db = redb::Database::create(&src_path).unwrap();
        {
            let write_txn = src_db.begin_write().unwrap();
            {
                // first create the table with correct type so redb stores the type metadata
                let mut table = write_txn.open_table(typed_def).unwrap();
                table
                    .insert(
                        "valid_key",
                        wallet_data::WalletData::ScanState(wallet_data::ScanState::Completed),
                    )
                    .unwrap();
            }
            write_txn.commit().unwrap();

            // now write a malformed JSON value directly using raw bytes
            // this requires opening the table with a raw value type that matches
            let write_txn = src_db.begin_write().unwrap();
            {
                let raw_table_def = TableDefinition::<
                    RawKey<&str>,
                    RawValue<Json<wallet_data::WalletData>>,
                >::new("wallet_data.json");
                let mut table = write_txn.open_table(raw_table_def).unwrap();
                table
                    .insert(b"stale_key" as &[u8], br#"{"Balance":{"total":1000}}"# as &[u8])
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let dst_db = redb::Database::create(&dst_path).unwrap();

        // raw byte copy should succeed without panicking
        let count = copy_table(&src_db, &dst_db, typed_def).unwrap();
        assert_eq!(count, 2);

        // verify both rows exist and malformed payload is byte-for-byte preserved
        let raw_table_def =
            TableDefinition::<RawKey<&str>, RawValue<Json<wallet_data::WalletData>>>::new(
                "wallet_data.json",
            );
        let read_txn = dst_db.begin_read().unwrap();
        let table = read_txn.open_table(raw_table_def).unwrap();
        assert_eq!(table.len().unwrap(), 2);

        let malformed = table.get(b"stale_key" as &[u8]).unwrap().unwrap();
        assert_eq!(malformed.value(), br#"{"Balance":{"total":1000}}"#);
    }

    #[test]
    fn recover_bak_only() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let bak_path = dir.path().join("cove.db.bak");

        // only .bak exists, no db or tmp
        std::fs::write(&bak_path, b"backup_data").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"backup_data");
    }

    #[test]
    fn migrate_main_database_copies_all_tables() {
        use redb::TableHandle as _;

        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_main_db(&dir);

        // ensure all known main tables exist in source
        {
            let db = redb::Database::open(&db_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            write_txn.open_table(global_flag::TABLE).unwrap();
            write_txn.open_table(global_config::TABLE).unwrap();
            write_txn.open_table(global_cache::TABLE).unwrap();
            write_txn.open_table(wallet::TABLE).unwrap();
            write_txn.open_table(unsigned_transactions::MAIN_TABLE).unwrap();
            write_txn.open_table(unsigned_transactions::BY_WALLET_TABLE).unwrap();
            write_txn.open_table(historical_price::TABLE).unwrap();
            write_txn.commit().unwrap();
        }

        let source_tables: std::collections::BTreeSet<String> = {
            let db = redb::Database::open(&db_path).unwrap();
            let read_txn = db.begin_read().unwrap();
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect()
        };
        assert!(!source_tables.is_empty(), "source DB should have tables");

        migrate_main_database(&db_path).unwrap();

        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&db_path, key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();
        let read_txn = db.begin_read().unwrap();

        let dest_tables: std::collections::BTreeSet<String> =
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect();

        let missing: Vec<_> = source_tables.difference(&dest_tables).collect();
        assert!(
            missing.is_empty(),
            "Tables in source but not in destination (migration missed them): {missing:?}"
        );
    }

    #[test]
    fn migrate_wallet_database_copies_all_tables() {
        use redb::TableHandle as _;

        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_wallet_db(&dir);

        // ensure all known wallet tables exist in source
        {
            let db = redb::Database::open(&db_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            write_txn.open_table(wallet_data::TABLE).unwrap();
            write_txn.open_table(wallet_data::label::TXN_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::ADDRESS_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::INPUT_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::OUTPUT_TABLE).unwrap();
            write_txn.commit().unwrap();
        }

        let source_tables: std::collections::BTreeSet<String> = {
            let db = redb::Database::open(&db_path).unwrap();
            let read_txn = db.begin_read().unwrap();
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect()
        };
        assert!(!source_tables.is_empty(), "source DB should have tables");

        migrate_wallet_database(&db_path).unwrap();

        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&db_path, key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();
        let read_txn = db.begin_read().unwrap();

        let dest_tables: std::collections::BTreeSet<String> =
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect();

        let missing: Vec<_> = source_tables.difference(&dest_tables).collect();
        assert!(
            missing.is_empty(),
            "Tables in source but not in destination (migration missed them): {missing:?}"
        );
    }

    #[test]
    fn wallet_migration_stops_on_cancellation() {
        setup_test_key();

        let dir = TempDir::new().unwrap();

        // create multiple wallet subdirs with plaintext wallet_data.json
        for name in ["wallet_aaa", "wallet_bbb", "wallet_ccc"] {
            let wallet_dir = dir.path().join(name);
            std::fs::create_dir_all(&wallet_dir).unwrap();
            let db_path = wallet_dir.join("wallet_data.json");
            let db = redb::Database::create(&db_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(wallet_data::TABLE).unwrap();
                table
                    .insert(
                        "scan_state_native_segwit",
                        wallet_data::WalletData::ScanState(wallet_data::ScanState::Completed),
                    )
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let cancelled = Arc::new(AtomicBool::new(true));
        let migration = Arc::new(Migration::new(3, cancelled));
        let result = WalletMigration::with_dir(dir.path().to_path_buf(), migration).run();

        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("cancelled"), "error should mention cancellation: {err_msg}");
    }

    #[test]
    fn wallet_migration_continues_past_bad_wallet() {
        setup_test_key();

        let dir = TempDir::new().unwrap();

        // create a corrupt wallet (valid dir but corrupt DB file)
        let bad_dir = dir.path().join("wallet_bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        let bad_db = bad_dir.join("wallet_data.json");
        // write garbage that is not a valid redb file but isn't encrypted either
        std::fs::write(&bad_db, b"not a valid redb database").unwrap();

        // create a valid wallet
        let good_dir = dir.path().join("wallet_good");
        std::fs::create_dir_all(&good_dir).unwrap();
        let good_db = good_dir.join("wallet_data.json");
        let db = redb::Database::create(&good_db).unwrap();
        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(wallet_data::TABLE).unwrap();
            table
                .insert(
                    "scan_state_native_segwit",
                    wallet_data::WalletData::ScanState(wallet_data::ScanState::Completed),
                )
                .unwrap();
        }
        write_txn.commit().unwrap();
        drop(db);

        let cancelled = Arc::new(AtomicBool::new(false));
        let migration = Arc::new(Migration::new(2, cancelled));
        let result =
            WalletMigration::with_dir(dir.path().to_path_buf(), Arc::clone(&migration)).run();

        // should report error for the bad wallet
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("wallet_bad"), "error should mention the bad wallet: {err_msg}");

        // both databases should tick (even the failed one) to keep watchdog happy
        assert_eq!(migration.progress().current, 2, "should tick for both success and failure");

        // good wallet should still have been migrated
        assert!(EncryptedBackend::is_encrypted(&good_db), "good wallet should be encrypted");
    }

    #[test]
    fn recover_restores_backup_when_encrypted_db_corrupt() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let bak_path = db_path.with_extension("db.bak");

        // corrupt encrypted DB
        std::fs::write(&db_path, b"corrupt_data").unwrap();
        // valid plaintext backup
        let backup_content = b"valid_backup_data";
        std::fs::write(&bak_path, backup_content).unwrap();

        // verify_encrypted_redb_db returns Ok(false) for corrupt data,
        // triggering the restore-from-backup path
        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists(), "db should exist after restore");
        assert!(!bak_path.exists(), "bak should be removed after restore");
        assert_eq!(
            std::fs::read(&db_path).unwrap(),
            backup_content,
            "db should contain backup data"
        );
    }
}
