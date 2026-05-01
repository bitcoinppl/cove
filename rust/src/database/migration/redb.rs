mod copy;
mod recovery;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::{Context as _, Result};
use redb::{ReadableTable as _, TableHandle as _};
use tracing::{error, info, warn};

use crate::bootstrap::Migration;
use crate::database::encrypted_backend::EncryptedBackend;
use cove_common::consts::{ROOT_DATA_DIR, WALLET_DATA_DIR};
use cove_types::{WalletId, redb::Json};

use self::copy::{
    RawKey, RawValue, copy_table, verify_all_source_tables_copied, verify_encrypted_redb_db,
};
use self::recovery::{
    recover_interrupted_main_migration as recover_interrupted_main_migration_impl,
    recover_interrupted_wallet_migrations as recover_interrupted_wallet_migrations_impl,
};
use super::MigrationFailure;

#[cfg(test)]
use self::recovery::{recover_legacy_at_path, recover_main_migration, recover_wallet_migration};

#[derive(Debug, thiserror::Error)]
enum WalletMigrationError {
    #[error("wallet migration cancelled")]
    Cancelled,

    #[error("failed to migrate {} wallet database(s)", .failures.len())]
    Failed { failures: Vec<MigrationFailure> },
}

const LEGACY_MAIN_DB: &str = "cove.db";
const LEGACY_WALLET_DB: &str = "wallet_data.json";
const ENCRYPTED_MAIN_DB: &str = "cove.encrypted.db";
const ENCRYPTED_WALLET_DB: &str = "wallet_data.encrypted.json.redb";

pub(super) struct DatabasePaths {
    source: PathBuf,
    dest: PathBuf,
    tmp: PathBuf,
}

fn main_database_paths(root_dir: &Path) -> DatabasePaths {
    let dest = root_dir.join(ENCRYPTED_MAIN_DB);
    DatabasePaths {
        source: root_dir.join(LEGACY_MAIN_DB),
        tmp: dest.with_extension("db.tmp"),
        dest,
    }
}

fn wallet_database_paths(wallet_dir: &Path) -> DatabasePaths {
    let dest = wallet_dir.join(ENCRYPTED_WALLET_DB);
    DatabasePaths {
        source: wallet_dir.join(LEGACY_WALLET_DB),
        tmp: dest.with_extension("redb.tmp"),
        dest,
    }
}

pub fn recover_interrupted_main_migration() -> Result<()> {
    recover_interrupted_main_migration_impl()
}

pub fn recover_interrupted_wallet_migrations() -> Result<()> {
    recover_interrupted_wallet_migrations_impl()
}

/// Check whether the main database needs migration (legacy plaintext exists, encrypted does not)
pub fn main_database_needs_migration() -> bool {
    let paths = main_database_paths(&ROOT_DATA_DIR);
    paths.source.exists() && !paths.dest.exists() && !EncryptedBackend::is_encrypted(&paths.source)
}

/// Check whether a wallet subdirectory needs plaintext-to-encrypted migration
fn needs_redb_migration(wallet_dir: &Path) -> bool {
    let paths = wallet_database_paths(wallet_dir);
    paths.source.exists() && !paths.dest.exists() && !EncryptedBackend::is_encrypted(&paths.source)
}

/// Check whether a wallet subdirectory has an already-encrypted legacy DB that just needs renaming
fn needs_legacy_rename(wallet_dir: &Path) -> bool {
    let paths = wallet_database_paths(wallet_dir);
    paths.source.exists() && !paths.dest.exists() && EncryptedBackend::is_encrypted(&paths.source)
}

/// Count known wallet subdirectories that still need redb migration
///
/// Best-effort: returns 0 if the directory is unreadable. The actual
/// migration in `WalletMigration::run()` will surface any real I/O errors
pub fn count_redb_wallets_needing_migration(known_wallet_ids: &BTreeSet<WalletId>) -> u32 {
    count_redb_wallets_needing_migration_in(&WALLET_DATA_DIR, known_wallet_ids)
}

fn count_redb_wallets_needing_migration_in(
    dir: &Path,
    known_wallet_ids: &BTreeSet<WalletId>,
) -> u32 {
    let entries = match std::fs::read_dir(dir) {
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
        let dir = entry.path();
        if !is_known_wallet_dir(&dir, known_wallet_ids) {
            continue;
        }

        if needs_redb_migration(&dir) || needs_legacy_rename(&dir) {
            count += 1;
        }
    }
    count
}

pub fn known_wallet_ids_from_main_database() -> Result<BTreeSet<WalletId>> {
    known_wallet_ids_from_main_database_at(&main_database_paths(&ROOT_DATA_DIR).dest)
}

fn known_wallet_ids_from_main_database_at(path: &Path) -> Result<BTreeSet<WalletId>> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }

    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before reading wallet metadata"))?;

    let backend =
        EncryptedBackend::open(path, &key).context("failed to open encrypted main database")?;

    let db = redb::Database::builder()
        .create_with_backend(backend)
        .context("failed to init encrypted main database")?;

    known_wallet_ids_from_database(&db)
}

fn known_wallet_ids_from_database(db: &redb::Database) -> Result<BTreeSet<WalletId>> {
    let read_txn = db.begin_read().context("failed to begin main database wallet read")?;
    let raw_def = redb::TableDefinition::<
        RawKey<&str>,
        RawValue<Json<Vec<crate::wallet::metadata::WalletMetadata>>>,
    >::new(crate::database::wallet::TABLE.name());

    let table = match read_txn.open_table(raw_def) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeSet::new()),
        Err(error) => return Err(error).context("failed to open wallet metadata table"),
    };

    let mut ids = BTreeSet::new();
    for entry in table.iter().context("failed to iterate wallet metadata table")? {
        let (_, value) = entry.context("failed to read wallet metadata row")?;
        let wallets: Vec<crate::wallet::metadata::WalletMetadata> =
            serde_json::from_slice(value.value())
                .context("failed to decode wallet metadata row")?;

        ids.extend(wallets.into_iter().map(|wallet| wallet.id));
    }

    Ok(ids)
}

/// Migrate the main redb database from plaintext to encrypted if needed
///
/// Returns Ok(true) if migration was performed, Ok(false) if already encrypted or new
pub fn migrate_main_database_if_needed() -> Result<bool> {
    let paths = main_database_paths(&ROOT_DATA_DIR);

    if !paths.source.exists() || paths.dest.exists() {
        return Ok(false);
    }

    // already encrypted by old migration code, just relocate to new path
    if EncryptedBackend::is_encrypted(&paths.source) {
        info!("Legacy DB at cove.db is already encrypted, renaming to cove.encrypted.db");
        std::fs::rename(&paths.source, &paths.dest)
            .context("failed to rename already-encrypted legacy DB")?;
        return Ok(true);
    }

    info!("Migrating main database to encrypted format");
    migrate_main_database(&paths.source)
}

/// Create a new encrypted redb database at `tmp_path` for migration
fn create_encrypted_dst(tmp_path: &Path) -> Result<redb::Database> {
    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before migration"))?;

    let backend =
        EncryptedBackend::create(tmp_path, &key).context("failed to create encrypted database")?;

    redb::Database::builder()
        .create_with_file_format_v3(true)
        .create_with_backend(backend)
        .context("failed to init encrypted database")
}

/// Verify that an encrypted redb database at `path` can be opened and read
fn verify_encrypted_dst(path: &Path) -> Result<()> {
    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before migration"))?;

    let verify_backend = EncryptedBackend::open(path, &key)
        .context("verification: cannot reopen encrypted database")?;

    let verify_db = redb::Database::builder()
        .create_with_backend(verify_backend)
        .context("verification: cannot init encrypted database")?;

    let _read = verify_db.begin_read().context("verification: encrypted database not readable")?;

    Ok(())
}

fn migrate_database(
    paths: &DatabasePaths,
    open_context: &str,
    rename_context: &str,
    copy_tables: impl FnOnce(&redb::Database, &redb::Database) -> Result<()>,
) -> Result<()> {
    let src_db = redb::Database::open(&paths.source).with_context(|| open_context.to_string())?;
    let dst_db = create_encrypted_dst(&paths.tmp)?;

    copy_tables(&src_db, &dst_db)?;
    verify_all_source_tables_copied(&src_db, &dst_db)?;

    drop(src_db);
    drop(dst_db);

    // verify BEFORE making any irreversible changes
    verify_encrypted_dst(&paths.tmp)?;
    std::fs::rename(&paths.tmp, &paths.dest).with_context(|| rename_context.to_string())?;

    // only delete plaintext after encrypted is verified and in place
    super::log_remove_file(&paths.source);

    Ok(())
}

fn migrate_main_database(source_path: &Path) -> Result<bool> {
    let root_dir = source_path.parent().unwrap_or(source_path);
    let paths = main_database_paths(root_dir);

    migrate_database(
        &paths,
        "failed to open plaintext main database",
        "failed to rename encrypted main database into place",
        |src_db, dst_db| {
            copy_table(src_db, dst_db, crate::database::global_flag::TABLE)?;
            copy_table(src_db, dst_db, crate::database::global_config::TABLE)?;
            copy_table(src_db, dst_db, crate::database::global_cache::TABLE)?;
            copy_table(src_db, dst_db, crate::database::cloud_backup::CLOUD_BACKUP_STATE_TABLE)?;
            copy_table(src_db, dst_db, crate::database::cloud_backup::CLOUD_BLOB_SYNC_STATE_TABLE)?;
            copy_table(src_db, dst_db, crate::database::wallet::TABLE)?;
            copy_table(src_db, dst_db, crate::database::unsigned_transactions::MAIN_TABLE)?;
            copy_table(src_db, dst_db, crate::database::unsigned_transactions::BY_WALLET_TABLE)?;
            copy_table(src_db, dst_db, crate::database::historical_price::TABLE)?;
            Ok(())
        },
    )?;

    info!("Main database migration complete");
    Ok(true)
}

pub struct WalletMigration {
    dir: PathBuf,
    migration: Arc<Migration>,
    known_wallet_ids: BTreeSet<WalletId>,
}

impl WalletMigration {
    pub fn new(migration: Arc<Migration>, known_wallet_ids: BTreeSet<WalletId>) -> Self {
        Self { dir: WALLET_DATA_DIR.to_path_buf(), migration, known_wallet_ids }
    }

    pub fn run(&self) -> Result<()> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e).context("failed to read wallet data directory");
            }
        };

        let mut failures = Vec::new();

        for entry in entries {
            self.check_cancelled()?;

            let entry = entry.context("failed to read directory entry during wallet migration")?;
            let wallet_dir = entry.path();
            if !self.is_known_wallet_dir(&wallet_dir) {
                self.log_skipped_orphan(&wallet_dir);
                continue;
            }

            self.migrate_entry(&wallet_dir, &mut failures);
        }

        if !failures.is_empty() {
            return Err(WalletMigrationError::Failed { failures }.into());
        }

        Ok(())
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.migration.is_cancelled() {
            info!("Wallet database migration cancelled");
            return Err(WalletMigrationError::Cancelled.into());
        }

        Ok(())
    }

    fn is_known_wallet_dir(&self, wallet_dir: &Path) -> bool {
        is_known_wallet_dir(wallet_dir, &self.known_wallet_ids)
    }

    fn log_skipped_orphan(&self, wallet_dir: &Path) {
        if needs_redb_migration(wallet_dir) || needs_legacy_rename(wallet_dir) {
            let dir = wallet_dir.display();
            warn!("Skipping redb migration for orphan wallet data directory at {dir}");
        }
    }

    fn migrate_entry(&self, wallet_dir: &Path, failures: &mut Vec<MigrationFailure>) {
        if needs_redb_migration(wallet_dir) {
            let source_db = wallet_database_paths(wallet_dir).source;
            let db_display = source_db.display().to_string();
            info!("Migrating wallet database at {db_display}");
            self.finish_entry(migrate_wallet_database(&source_db), db_display, failures);
            return;
        }

        if needs_legacy_rename(wallet_dir) {
            let paths = wallet_database_paths(wallet_dir);
            let db_display = paths.source.display().to_string();
            info!("Legacy wallet DB at {db_display} already encrypted, renaming");
            self.finish_entry(
                std::fs::rename(&paths.source, &paths.dest).map_err(Into::into),
                db_display,
                failures,
            );
        }
    }

    fn finish_entry(
        &self,
        result: Result<()>,
        db_path: String,
        failures: &mut Vec<MigrationFailure>,
    ) {
        match result {
            Ok(()) => self.migration.tick(),
            Err(error) => {
                error!("Failed to migrate wallet database {db_path}: {error:#}");
                failures.push(MigrationFailure { db_path, error: format!("{error:#}") });
                // tick even on failure to keep progress bar advancing and prevent watchdog timeout
                self.migration.tick();
            }
        }
    }
}

fn is_known_wallet_dir(wallet_dir: &Path, known_wallet_ids: &BTreeSet<WalletId>) -> bool {
    let Some(name) = wallet_dir.file_name().and_then(std::ffi::OsStr::to_str) else {
        return false;
    };

    known_wallet_ids.contains(name)
}

fn migrate_wallet_database(source_path: &Path) -> Result<()> {
    let wallet_dir = source_path.parent().unwrap_or(source_path);
    let paths = wallet_database_paths(wallet_dir);

    migrate_database(
        &paths,
        "failed to open plaintext wallet database",
        "failed to rename encrypted wallet database into place",
        |src_db, dst_db| {
            copy_table(src_db, dst_db, crate::database::wallet_data::TABLE)?;
            copy_table(src_db, dst_db, crate::database::wallet_data::label::TXN_TABLE)?;
            copy_table(src_db, dst_db, crate::database::wallet_data::label::ADDRESS_TABLE)?;
            copy_table(src_db, dst_db, crate::database::wallet_data::label::INPUT_TABLE)?;
            copy_table(src_db, dst_db, crate::database::wallet_data::label::OUTPUT_TABLE)?;
            Ok(())
        },
    )?;

    let path = paths.dest.display();
    info!("Wallet database migration complete at {path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use redb::{ReadableTableMetadata as _, TableDefinition};

    use super::*;
    use crate::database::{
        cloud_backup, encrypted_backend, global_cache, global_config, global_flag,
        historical_price, unsigned_transactions, wallet, wallet_data,
    };
    use crate::wallet::metadata::{WalletMetadata, WalletMode};
    use tempfile::TempDir;

    impl WalletMigration {
        fn with_dir(dir: PathBuf, migration: Arc<Migration>) -> Self {
            Self { dir, migration, known_wallet_ids: BTreeSet::new() }
        }

        fn with_dir_and_known_wallet_ids(
            dir: PathBuf,
            migration: Arc<Migration>,
            known_wallet_ids: BTreeSet<WalletId>,
        ) -> Self {
            Self { dir, migration, known_wallet_ids }
        }
    }

    fn setup_test_key() {
        encrypted_backend::set_test_encryption_key();
    }

    fn create_encrypted_redb_at(path: &Path) {
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::create(path, &key).unwrap();
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

    fn create_encrypted_main_db_with_wallets(path: &Path, wallets: Vec<WalletMetadata>) {
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::create(path, &key).unwrap();
        let db = redb::Database::builder()
            .create_with_file_format_v3(true)
            .create_with_backend(backend)
            .unwrap();

        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(wallet::TABLE).unwrap();
            let key = wallet::WalletKey::from((crate::network::Network::Bitcoin, WalletMode::Main))
                .to_string();
            table.insert(&*key, wallets).unwrap();
        }
        write_txn.commit().unwrap();
    }

    fn wallet_metadata_with_id(id: &str) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = id.into();
        metadata
    }

    fn wallet_ids(ids: &[&str]) -> BTreeSet<WalletId> {
        ids.iter().copied().map(WalletId::from).collect()
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
        create_plaintext_wallet_db_named(dir, "test_wallet_id")
    }

    fn create_plaintext_wallet_db_named(dir: &TempDir, wallet_id: &str) -> PathBuf {
        let wallet_dir = dir.path().join(wallet_id);
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
        let source_path = create_plaintext_main_db(&dir);
        let dest_path = dir.path().join(ENCRYPTED_MAIN_DB);

        assert!(!EncryptedBackend::is_encrypted(&source_path));

        migrate_main_database(&source_path).unwrap();

        // source deleted, dest is encrypted
        assert!(!source_path.exists());
        assert!(dest_path.exists());
        assert!(EncryptedBackend::is_encrypted(&dest_path));

        // verify data survived migration
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&dest_path, &key).unwrap();
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
    fn migrate_main_database_copies_cloud_backup_rows() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source_path = create_plaintext_main_db(&dir);
        let dest_path = dir.path().join(ENCRYPTED_MAIN_DB);

        let backup_state = cloud_backup::PersistedCloudBackupState {
            status: cloud_backup::PersistedCloudBackupStatus::Unverified,
            last_sync: Some(42),
            wallet_count: Some(2),
            ..cloud_backup::PersistedCloudBackupState::default()
        };
        let sync_state = cloud_backup::PersistedCloudBlobSyncState {
            kind: cloud_backup::CloudUploadKind::BackupBlob,
            namespace_id: "namespace".into(),
            wallet_id: None,
            record_id: "wallet-a".into(),
            state: cloud_backup::PersistedCloudBlobState::Dirty(
                cloud_backup::CloudBlobDirtyState { changed_at: 7 },
            ),
        };

        {
            let db = redb::Database::open(&source_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table =
                    write_txn.open_table(cloud_backup::CLOUD_BACKUP_STATE_TABLE).unwrap();
                table.insert("current", backup_state.clone()).unwrap();
            }
            {
                let mut table =
                    write_txn.open_table(cloud_backup::CLOUD_BLOB_SYNC_STATE_TABLE).unwrap();
                table.insert(sync_state.record_id.as_str(), sync_state.clone()).unwrap();
            }
            write_txn.commit().unwrap();
        }

        migrate_main_database(&source_path).unwrap();

        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&dest_path, &key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();
        let read_txn = db.begin_read().unwrap();

        let backup_table = read_txn.open_table(cloud_backup::CLOUD_BACKUP_STATE_TABLE).unwrap();
        assert_eq!(backup_table.get("current").unwrap().unwrap().value(), backup_state);

        let sync_table = read_txn.open_table(cloud_backup::CLOUD_BLOB_SYNC_STATE_TABLE).unwrap();
        assert_eq!(sync_table.get("wallet-a").unwrap().unwrap().value(), sync_state);
    }

    #[test]
    fn migrate_wallet_database_roundtrip() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source_path = create_plaintext_wallet_db(&dir);
        let wallet_dir = source_path.parent().unwrap();
        let dest_path = wallet_dir.join(ENCRYPTED_WALLET_DB);

        assert!(!EncryptedBackend::is_encrypted(&source_path));

        migrate_wallet_database(&source_path).unwrap();

        // source deleted, dest is encrypted
        assert!(!source_path.exists());
        assert!(dest_path.exists());
        assert!(EncryptedBackend::is_encrypted(&dest_path));

        // verify data survived
        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&dest_path, &key).unwrap();
        let db = redb::Database::builder().create_with_backend(backend).unwrap();

        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(wallet_data::TABLE).unwrap();
        assert!(table.get("scan_state_native_segwit").unwrap().is_some());
    }

    #[test]
    fn recover_main_crash_during_copy() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source = dir.path().join(LEGACY_MAIN_DB);
        let dest = dir.path().join(ENCRYPTED_MAIN_DB);
        let tmp = dest.with_extension("db.tmp");

        // simulate crash during copy: source exists, partial tmp exists, no dest
        std::fs::write(&source, b"original").unwrap();
        std::fs::write(&tmp, b"partial_corrupt").unwrap();

        recover_main_migration(dir.path()).unwrap();

        // corrupt tmp should be cleaned up, source untouched
        assert!(source.exists());
        assert!(!tmp.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn recover_main_crash_after_copy_before_rename() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source = dir.path().join(LEGACY_MAIN_DB);
        let dest = dir.path().join(ENCRYPTED_MAIN_DB);
        let tmp = dest.with_extension("db.tmp");

        // simulate crash after successful copy but before rename: valid encrypted tmp, no dest
        std::fs::write(&source, b"old_plaintext").unwrap();
        create_encrypted_redb_at(&tmp);

        recover_main_migration(dir.path()).unwrap();

        // source still exists, so retry from source instead of trusting an unpromoted temp
        assert!(!dest.exists());
        assert!(!tmp.exists());
        assert!(source.exists());
    }

    #[test]
    fn recover_main_cleans_leftover_plaintext() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source = dir.path().join(LEGACY_MAIN_DB);
        let dest = dir.path().join(ENCRYPTED_MAIN_DB);

        // simulate: both source and dest exist (migration completed but plaintext not deleted)
        std::fs::write(&source, b"old_plaintext").unwrap();
        create_encrypted_redb_at(&dest);

        recover_main_migration(dir.path()).unwrap();

        assert!(dest.exists());
        assert!(!source.exists(), "plaintext should be cleaned after verification");
    }

    #[test]
    fn recover_main_quarantines_corrupt_encrypted_when_plaintext_exists() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source = dir.path().join(LEGACY_MAIN_DB);
        let dest = dir.path().join(ENCRYPTED_MAIN_DB);
        let corrupt = dir.path().join("cove.encrypted.db.corrupt");

        // both exist but dest is corrupt
        std::fs::write(&source, b"old_plaintext").unwrap();
        std::fs::write(&dest, b"corrupt_encrypted").unwrap();

        recover_main_migration(dir.path()).unwrap();

        assert!(source.exists());
        assert!(!dest.exists());
        assert!(corrupt.exists());
        assert_eq!(std::fs::read(&corrupt).unwrap(), b"corrupt_encrypted");
    }

    #[test]
    fn recover_main_preserves_corrupt_encrypted_when_no_plaintext_exists() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source = dir.path().join(LEGACY_MAIN_DB);
        let dest = dir.path().join(ENCRYPTED_MAIN_DB);

        std::fs::write(&dest, b"corrupt_encrypted").unwrap();

        recover_main_migration(dir.path()).unwrap();

        assert!(!source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn recover_wallet_crash_during_copy() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let wallet_dir = dir.path().join("wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();

        let source = wallet_dir.join(LEGACY_WALLET_DB);
        let dest = wallet_dir.join(ENCRYPTED_WALLET_DB);
        let tmp = dest.with_extension("redb.tmp");

        std::fs::write(&source, b"original").unwrap();
        std::fs::write(&tmp, b"partial").unwrap();

        recover_wallet_migration(&wallet_dir).unwrap();

        assert!(source.exists());
        assert!(!tmp.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn recover_wallet_crash_after_copy_before_rename() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let wallet_dir = dir.path().join("wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();

        let source = wallet_dir.join(LEGACY_WALLET_DB);
        let dest = wallet_dir.join(ENCRYPTED_WALLET_DB);
        let tmp = dest.with_extension("redb.tmp");

        std::fs::write(&source, b"old_data").unwrap();
        create_encrypted_redb_at(&tmp);

        recover_wallet_migration(&wallet_dir).unwrap();

        assert!(!dest.exists());
        assert!(!tmp.exists());
        assert!(source.exists());
    }

    #[test]
    fn recover_wallet_quarantines_corrupt_encrypted_when_plaintext_exists() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let wallet_dir = dir.path().join("wallet_id");
        std::fs::create_dir_all(&wallet_dir).unwrap();

        let source = wallet_dir.join(LEGACY_WALLET_DB);
        let dest = wallet_dir.join(ENCRYPTED_WALLET_DB);
        let corrupt = wallet_dir.join("wallet_data.encrypted.json.redb.corrupt");

        std::fs::write(&source, b"old_data").unwrap();
        std::fs::write(&dest, b"corrupt_encrypted").unwrap();

        recover_wallet_migration(&wallet_dir).unwrap();

        assert!(source.exists());
        assert!(!dest.exists());
        assert!(corrupt.exists());
    }

    #[test]
    fn recover_legacy_preserves_bak_when_main_db_corrupt() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let bak_path = dir.path().join("cove.db.bak");
        let corrupt_path = dir.path().join("cove.db.corrupt");

        // main DB exists but is corrupt, .bak is a valid encrypted DB
        std::fs::write(&db_path, b"corrupt_data").unwrap();
        create_encrypted_redb_at(&bak_path);

        recover_legacy_at_path(&db_path).unwrap();

        // should restore from .bak without deleting the corrupt DB
        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert!(corrupt_path.exists());
        assert_eq!(std::fs::read(&corrupt_path).unwrap(), b"corrupt_data");
        assert!(verify_encrypted_redb_db(&db_path).unwrap());
    }

    #[test]
    fn recover_legacy_deletes_bak_when_main_db_healthy() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let bak_path = dir.path().join("cove.db.bak");

        // main DB is healthy, .bak exists
        create_encrypted_redb_at(&db_path);
        std::fs::write(&bak_path, b"old_backup").unwrap();

        recover_legacy_at_path(&db_path).unwrap();

        // should have deleted .bak since main DB is healthy
        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert!(verify_encrypted_redb_db(&db_path).unwrap());
    }

    #[test]
    fn recover_legacy_bak_only() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let bak_path = dir.path().join("cove.db.bak");

        // only .bak exists from old migration code
        std::fs::write(&bak_path, b"backup_data").unwrap();

        recover_legacy_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"backup_data");
    }

    #[test]
    fn recover_legacy_tmp_only_promotes_valid_tmp() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let tmp_path = dir.path().join("cove.db.enc.tmp");

        create_encrypted_redb_at(&tmp_path);

        recover_legacy_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert!(verify_encrypted_redb_db(&db_path).unwrap());
    }

    #[test]
    fn recover_legacy_tmp_only_preserves_invalid_tmp() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("cove.db");
        let tmp_path = dir.path().join("cove.db.enc.tmp");

        std::fs::write(&tmp_path, b"corrupt_tmp").unwrap();

        recover_legacy_at_path(&db_path).unwrap();

        assert!(!db_path.exists());
        assert!(tmp_path.exists());
        assert_eq!(std::fs::read(&tmp_path).unwrap(), b"corrupt_tmp");
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
    fn migrate_main_database_copies_all_tables() {
        use redb::TableHandle as _;

        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source_path = create_plaintext_main_db(&dir);
        let dest_path = dir.path().join(ENCRYPTED_MAIN_DB);

        // ensure all known main tables exist in source
        {
            let db = redb::Database::open(&source_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            write_txn.open_table(global_flag::TABLE).unwrap();
            write_txn.open_table(global_config::TABLE).unwrap();
            write_txn.open_table(global_cache::TABLE).unwrap();
            write_txn.open_table(cloud_backup::CLOUD_BACKUP_STATE_TABLE).unwrap();
            write_txn.open_table(cloud_backup::CLOUD_BLOB_SYNC_STATE_TABLE).unwrap();
            write_txn.open_table(wallet::TABLE).unwrap();
            write_txn.open_table(unsigned_transactions::MAIN_TABLE).unwrap();
            write_txn.open_table(unsigned_transactions::BY_WALLET_TABLE).unwrap();
            write_txn.open_table(historical_price::TABLE).unwrap();
            write_txn.commit().unwrap();
        }

        let source_tables: std::collections::BTreeSet<String> = {
            let db = redb::Database::open(&source_path).unwrap();
            let read_txn = db.begin_read().unwrap();
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect()
        };
        assert!(!source_tables.is_empty(), "source DB should have tables");

        migrate_main_database(&source_path).unwrap();

        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&dest_path, &key).unwrap();
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
    fn migrate_main_database_fails_if_source_table_is_not_copied() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source_path = create_plaintext_main_db(&dir);
        let dest_path = dir.path().join(ENCRYPTED_MAIN_DB);
        let tmp_path = dest_path.with_extension("db.tmp");
        let future_table: TableDefinition<&str, &str> = TableDefinition::new("future_table");

        {
            let db = redb::Database::open(&source_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(future_table).unwrap();
                table.insert("key", "value").unwrap();
            }
            write_txn.commit().unwrap();
        }

        let result = migrate_main_database(&source_path);
        assert!(result.is_err(), "migration should fail instead of dropping an unknown table");
        assert!(result.unwrap_err().to_string().contains("future_table"));
        assert!(source_path.exists());
        assert!(!dest_path.exists());
        assert!(tmp_path.exists());

        recover_main_migration(dir.path()).unwrap();

        assert!(source_path.exists());
        assert!(!dest_path.exists());
        assert!(!tmp_path.exists());
    }

    #[test]
    fn migrate_wallet_database_copies_all_tables() {
        use redb::TableHandle as _;

        setup_test_key();

        let dir = TempDir::new().unwrap();
        let source_path = create_plaintext_wallet_db(&dir);
        let wallet_dir = source_path.parent().unwrap();
        let dest_path = wallet_dir.join(ENCRYPTED_WALLET_DB);

        // ensure all known wallet tables exist in source
        {
            let db = redb::Database::open(&source_path).unwrap();
            let write_txn = db.begin_write().unwrap();
            write_txn.open_table(wallet_data::TABLE).unwrap();
            write_txn.open_table(wallet_data::label::TXN_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::ADDRESS_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::INPUT_TABLE).unwrap();
            write_txn.open_table(wallet_data::label::OUTPUT_TABLE).unwrap();
            write_txn.commit().unwrap();
        }

        let source_tables: std::collections::BTreeSet<String> = {
            let db = redb::Database::open(&source_path).unwrap();
            let read_txn = db.begin_read().unwrap();
            read_txn.list_tables().unwrap().map(|handle| handle.name().to_string()).collect()
        };
        assert!(!source_tables.is_empty(), "source DB should have tables");

        migrate_wallet_database(&source_path).unwrap();

        let key = encrypted_backend::encryption_key().unwrap();
        let backend = EncryptedBackend::open(&dest_path, &key).unwrap();
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
    fn known_wallet_ids_reads_wallet_metadata_from_main_database() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join(ENCRYPTED_MAIN_DB);
        create_encrypted_main_db_with_wallets(
            &path,
            vec![wallet_metadata_with_id("wallet_good"), wallet_metadata_with_id("wallet_other")],
        );

        let ids = known_wallet_ids_from_main_database_at(&path).unwrap();

        assert!(ids.contains("wallet_good"));
        assert!(ids.contains("wallet_other"));
    }

    #[test]
    fn count_redb_wallets_needing_migration_ignores_orphan_dirs() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        create_plaintext_wallet_db_named(&dir, "wallet_good");
        create_plaintext_wallet_db_named(&dir, "wallet_orphan");

        let count =
            count_redb_wallets_needing_migration_in(dir.path(), &wallet_ids(&["wallet_good"]));

        assert_eq!(count, 1);
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
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<WalletMigrationError>()
                .is_some_and(|e| matches!(e, WalletMigrationError::Cancelled))
        );
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
        let result = WalletMigration::with_dir_and_known_wallet_ids(
            dir.path().to_path_buf(),
            Arc::clone(&migration),
            wallet_ids(&["wallet_bad", "wallet_good"]),
        )
        .run();

        // should report error for the bad wallet
        assert!(result.is_err());
        let err = result.unwrap_err();
        let migration_err =
            err.downcast_ref::<WalletMigrationError>().expect("should be WalletMigrationError");
        match migration_err {
            WalletMigrationError::Failed { failures } => {
                assert!(
                    failures.iter().any(|f| f.db_path.contains("wallet_bad")),
                    "error should mention the bad wallet"
                );
            }
            other => panic!("expected WalletMigrationError::Failed, got: {other}"),
        }

        // both databases should tick (even the failed one) to keep watchdog happy
        assert_eq!(migration.progress().current, 2, "should tick for both success and failure");

        // good wallet should still have been migrated
        let good_encrypted = good_dir.join(ENCRYPTED_WALLET_DB);
        assert!(good_encrypted.exists(), "good wallet should have encrypted DB");
        assert!(EncryptedBackend::is_encrypted(&good_encrypted), "good wallet should be encrypted");
    }

    #[test]
    fn wallet_migration_skips_orphan_wallet_dirs() {
        setup_test_key();

        let dir = TempDir::new().unwrap();

        let orphan_dir = dir.path().join("wallet_orphan");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        let orphan_db = orphan_dir.join("wallet_data.json");
        std::fs::write(&orphan_db, b"not a valid redb database").unwrap();

        let good_db = create_plaintext_wallet_db_named(&dir, "wallet_good");
        let good_dir = good_db.parent().unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let migration = Arc::new(Migration::new(1, cancelled));
        WalletMigration::with_dir_and_known_wallet_ids(
            dir.path().to_path_buf(),
            Arc::clone(&migration),
            wallet_ids(&["wallet_good"]),
        )
        .run()
        .unwrap();

        assert_eq!(migration.progress().current, 1);
        assert!(orphan_db.exists(), "orphan source should be left untouched");
        assert!(!orphan_dir.join(ENCRYPTED_WALLET_DB).exists());
        assert!(good_dir.join(ENCRYPTED_WALLET_DB).exists());
    }

    #[test]
    fn wallet_migration_fails_known_corrupt_wallet_dir() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let bad_dir = dir.path().join("wallet_bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        let bad_db = bad_dir.join("wallet_data.json");
        std::fs::write(&bad_db, b"not a valid redb database").unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let migration = Arc::new(Migration::new(1, cancelled));
        let result = WalletMigration::with_dir_and_known_wallet_ids(
            dir.path().to_path_buf(),
            migration,
            wallet_ids(&["wallet_bad"]),
        )
        .run();

        assert!(result.is_err());
        assert!(bad_db.exists(), "known corrupt source should be preserved");
    }
}
