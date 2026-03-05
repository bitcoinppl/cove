use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use eyre::{Context as _, Result};
use tracing::{error, info, warn};

use crate::bdk_store::sqlite_auxiliary_path;
use crate::bootstrap::Migration;
use cove_common::consts::ROOT_DATA_DIR;

use super::log_remove_file;

/// Check whether an encrypted BDK database can be opened and read
///
/// Returns `Ok(true)` if verified, `Ok(false)` if corrupt, `Err` for I/O errors
fn verify_encrypted_bdk_db(path: &Path) -> Result<bool> {
    let path_display = path.display();

    let key = crate::database::encrypted_backend::encryption_key().ok_or_else(|| {
        eyre::eyre!("no encryption key available for verification of {path_display}")
    })?;

    let hex_key = format!("x'{}'", hex::encode(key));

    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Verification failed for {path_display}: could not open connection: {e}");
            return Ok(false);
        }
    };

    if let Err(e) = conn.pragma_update(None, "key", &hex_key) {
        warn!("Verification failed for {path_display}: pragma key failed: {e}");
        return Ok(false);
    };

    if let Err(e) =
        conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| {
            r.get::<_, i64>(0)
        })
    {
        warn!("Verification failed for {path_display}: schema query failed: {e}");
        return Ok(false);
    }

    // page-level integrity check catches corruption a schema read would miss
    match conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)) {
        Ok(ref result) if result == "ok" => Ok(true),
        Ok(result) => {
            warn!("Verification failed for {path_display}: integrity check returned: {result}");
            Ok(false)
        }
        Err(e) => {
            warn!("Verification failed for {path_display}: integrity check failed: {e}");
            Ok(false)
        }
    }
}

/// Check if a file is a plaintext (unencrypted) SQLite database
pub fn is_plaintext_sqlite(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => {
            warn!("Cannot check if {} is plaintext SQLite: {e}", path.display());
            return false;
        }
    };

    use std::io::Read as _;
    let mut header = [0u8; 16];
    if file.read_exact(&mut header).is_err() {
        return false;
    }

    header.starts_with(b"SQLite format 3\0")
}

/// Migrate a single BDK database from plaintext SQLite to SQLCipher
fn migrate_single_bdk_database(path: &Path) -> Result<()> {
    let key = crate::database::encrypted_backend::encryption_key()
        .ok_or_else(|| eyre::eyre!("encryption key must be set before BDK migration"))?;
    let hex_key = format!("x'{}'", hex::encode(key));

    let tmp_path = path.with_extension("db.enc.tmp");
    let bak_path = path.with_extension("db.bak");

    let conn = rusqlite::Connection::open(path).context("failed to open plaintext BDK database")?;

    // empty database (created but never used) — just delete it
    let table_count: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0))
        .context("failed to check table count in plaintext database")?;

    if table_count == 0 {
        drop(conn);
        log_remove_file(path);
        clean_auxiliary_files(path);
        return Ok(());
    }

    let path_str = tmp_path.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!("ATTACH DATABASE '{path_str}' AS encrypted KEY \"{hex_key}\";",))
        .context("failed to attach encrypted database")?;

    conn.execute_batch("SELECT sqlcipher_export('encrypted');")
        .context("sqlcipher_export failed")?;

    conn.execute_batch("DETACH DATABASE encrypted;")
        .context("failed to detach encrypted database")?;

    drop(conn);

    // verify the encrypted database is readable before swapping
    {
        let verify = rusqlite::Connection::open(&tmp_path)
            .context("verification: cannot open exported database")?;
        verify
            .pragma_update(None, "key", &hex_key)
            .context("verification: cannot set key on exported database")?;
        verify
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| {
                r.get::<_, i64>(0)
            })
            .context("verification: exported database appears corrupt")?;
    }

    std::fs::rename(path, &bak_path).context("failed to rename old BDK database to .bak")?;
    std::fs::rename(&tmp_path, path)
        .context("failed to rename encrypted BDK database into place")?;

    // keep .bak until next launch when recovery verifies the encrypted DB works
    clean_auxiliary_files(path);

    Ok(())
}

fn clean_auxiliary_files(db_path: &Path) {
    for suffix in ["wal", "shm"] {
        let aux_path = sqlite_auxiliary_path(db_path, suffix);
        log_remove_file(&aux_path);
    }
}

/// Checkpoint WAL data into the main DB file before removing auxiliary files
///
/// Prevents losing uncheckpointed writes when recovery artifacts trigger cleanup
fn checkpoint_and_clean_auxiliary_files(db_path: &Path) {
    if !db_path.exists() {
        // no main DB file — auxiliaries are definitely stale
        clean_auxiliary_files(db_path);
        return;
    }

    let Some(key) = crate::database::encrypted_backend::encryption_key() else {
        let path = db_path.display();
        warn!("No encryption key — preserving WAL/SHM at {path}");
        return;
    };

    let hex_key = format!("x'{}'", hex::encode(key));
    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        let path = db_path.display();
        warn!("Cannot open DB for checkpoint at {path} — preserving WAL/SHM");
        return;
    };
    if conn.pragma_update(None, "key", &hex_key).is_err() {
        let path = db_path.display();
        warn!("Cannot set key for checkpoint at {path} — preserving WAL/SHM");
        return;
    }

    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get::<_, i32>(0)) {
        Ok(0) => clean_auxiliary_files(db_path),
        Ok(busy) => {
            let path = db_path.display();
            warn!("WAL checkpoint busy={busy} at {path} — preserving WAL/SHM")
        }
        Err(e) => {
            let path = db_path.display();
            warn!("WAL checkpoint failed at {path}: {e} — preserving WAL/SHM")
        }
    }
}

/// Recover from interrupted BDK migrations
pub fn recover_interrupted_bdk_migrations() -> Result<()> {
    recover_interrupted_bdk_migrations_in_dir(&ROOT_DATA_DIR)
}

fn recover_interrupted_bdk_migrations_in_dir(dir: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            eyre::bail!("Failed to read data directory for BDK recovery: {e}");
        }
    };

    let mut recovery_targets = HashSet::new();

    for entry in entries {
        let entry = entry.context("failed to read directory entry during BDK recovery")?;
        if let Some(path) = recovery_target_path(&entry.path()) {
            recovery_targets.insert(path);
        }
    }

    for target in recovery_targets {
        recover_at_path(&target)?;
    }

    Ok(())
}

fn recovery_target_path(path: &Path) -> Option<std::path::PathBuf> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    if !name.starts_with("bdk_wallet_sqlite_") {
        return None;
    }

    // check longer suffixes first so `.db.enc.tmp` and `.db.bak` don't match `.db`
    if let Some(base_name) = name.strip_suffix(".db.enc.tmp") {
        return Some(path.with_file_name(format!("{base_name}.db")));
    }

    if let Some(base_name) = name.strip_suffix(".db.bak") {
        return Some(path.with_file_name(format!("{base_name}.db")));
    }

    if name.ends_with(".db") {
        return Some(path.to_path_buf());
    }

    None
}

fn recover_at_path(db_path: &Path) -> Result<()> {
    let bak_path = db_path.with_extension("db.bak");
    let tmp_path = db_path.with_extension("db.enc.tmp");

    // only clean WAL/SHM when recovery artifacts (.bak/.enc.tmp) are present,
    // otherwise a normal DB's uncommitted WAL data would be lost
    let had_recovery_artifacts = bak_path.exists() || tmp_path.exists();

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
        info!("Recovering interrupted BDK migration at {path}");
        std::fs::rename(&tmp_path, db_path)
            .context(format!("failed to finish interrupted BDK migration at {path}"))?;
    }

    if tmp_path.exists() {
        log_remove_file(&tmp_path);
    }

    if bak_path.exists() && db_path.exists() {
        match verify_encrypted_bdk_db(db_path) {
            Ok(true) => {
                // encrypted DB confirmed working — safe to delete backup
                log_remove_file(&bak_path);
                clean_auxiliary_files(&bak_path);
            }
            Ok(false) => {
                // encrypted DB is corrupt, restore from backup
                let path = db_path.display();
                warn!("Encrypted DB at {path} appears corrupt, restoring from backup");
                log_remove_file(db_path);
                clean_auxiliary_files(db_path);
                std::fs::rename(&bak_path, db_path).context("failed to restore from backup")?;
                // restored a plaintext backup — skip SQLCipher checkpoint
                return Ok(());
            }
            Err(e) => {
                // I/O error — preserve both files so nothing is lost
                let path = db_path.display();
                warn!("Cannot verify encrypted DB at {path}: {e:#} — preserving both files");
                return Ok(());
            }
        }
    }

    if had_recovery_artifacts {
        checkpoint_and_clean_auxiliary_files(db_path);
    }

    Ok(())
}

/// Check whether a file is a BDK wallet database that needs migration
fn needs_bdk_migration(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name.starts_with("bdk_wallet_sqlite_") && name.ends_with(".db") && is_plaintext_sqlite(path)
}

/// Count BDK databases in ROOT_DATA_DIR that are plaintext SQLite
/// Best-effort: returns 0 if the directory is unreadable. The actual
/// migration in `BdkMigration::run()` will surface any real I/O errors
pub fn count_bdk_databases_needing_migration() -> u32 {
    count_bdk_databases_in_dir(&ROOT_DATA_DIR)
}

fn count_bdk_databases_in_dir(dir: &Path) -> u32 {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            error!("Failed to read directory for BDK migration count: {e}");
            return 0;
        }
    };

    let mut count = 0u32;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry during BDK migration count: {e}");
                continue;
            }
        };
        let path = entry.path();
        if needs_bdk_migration(&path) {
            count += 1;
        }
    }
    count
}

pub struct BdkMigration {
    dir: PathBuf,
    migration: Arc<Migration>,
}

impl BdkMigration {
    pub fn new(migration: Arc<Migration>) -> Self {
        Self { dir: ROOT_DATA_DIR.to_path_buf(), migration }
    }

    #[cfg(test)]
    fn with_dir(dir: PathBuf, migration: Arc<Migration>) -> Self {
        Self { dir, migration }
    }

    pub fn run(&self) -> Result<()> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                eyre::bail!("Failed to read data directory for BDK migration: {e}");
            }
        };

        let mut errors: Vec<String> = Vec::new();

        for entry in entries {
            if self.migration.is_cancelled() {
                info!("BDK migration cancelled after partial progress");
                eyre::bail!("BDK migration cancelled");
            }

            let entry = entry.context("failed to read directory entry during BDK migration")?;
            let path = entry.path();

            if needs_bdk_migration(&path) {
                let db_display = path.display();
                info!("Migrating BDK database at {db_display}");
                match migrate_single_bdk_database(&path) {
                    Ok(()) => self.migration.tick(),
                    Err(e) => {
                        error!("Failed to migrate BDK database {db_display}: {e:#}");
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
            eyre::bail!("failed to migrate {count} BDK database(s): {details}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::encrypted_backend;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn setup_test_key() {
        encrypted_backend::set_test_encryption_key();
    }

    fn test_migration(total: u32) -> Arc<Migration> {
        let cancelled = Arc::new(AtomicBool::new(false));
        Arc::new(Migration::new(total, cancelled))
    }

    fn create_plaintext_bdk_db(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        create_plaintext_bdk_db_at(&path);
        path
    }

    fn create_plaintext_bdk_db_at(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE test_data (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test_data (id, value) VALUES (1, 'hello');
             INSERT INTO test_data (id, value) VALUES (2, 'world');",
        )
        .unwrap();
    }

    /// Create a real encrypted SQLCipher database at the given path
    fn create_encrypted_db_at(path: &Path) {
        setup_test_key();
        let key = encrypted_backend::encryption_key().unwrap();
        let hex_key = format!("x'{}'", hex::encode(key));
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.pragma_update(None, "key", &hex_key).unwrap();
        conn.execute_batch(
            "CREATE TABLE test_data (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test_data (id, value) VALUES (1, 'hello');",
        )
        .unwrap();
    }

    #[test]
    fn is_plaintext_sqlite_detection() {
        let dir = TempDir::new().unwrap();

        let plain_path = create_plaintext_bdk_db(&dir, "plain.db");
        assert!(is_plaintext_sqlite(&plain_path));

        // encrypted file should not be detected as plaintext
        let enc_path = dir.path().join("encrypted.db");
        std::fs::write(&enc_path, b"not a sqlite header at all").unwrap();
        assert!(!is_plaintext_sqlite(&enc_path));

        // nonexistent file
        assert!(!is_plaintext_sqlite(&dir.path().join("nope.db")));
    }

    #[test]
    fn migrate_single_bdk_database_roundtrip() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_bdk_db(&dir, "bdk_wallet_sqlite_test.db");

        assert!(is_plaintext_sqlite(&db_path));

        migrate_single_bdk_database(&db_path).unwrap();

        // should no longer be plaintext
        assert!(!is_plaintext_sqlite(&db_path));

        // verify data survived: open with the encryption key
        let key = encrypted_backend::encryption_key().unwrap();
        let hex_key = format!("x'{}'", hex::encode(key));

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "key", &hex_key).unwrap();

        let value: String = conn
            .query_row("SELECT value FROM test_data WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "hello");

        let value: String = conn
            .query_row("SELECT value FROM test_data WHERE id = 2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "world");
    }

    #[test]
    fn recover_bdk_crash_during_copy() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let tmp_path = dir.path().join("bdk_wallet_sqlite_test.db.enc.tmp");

        std::fs::write(&db_path, b"original").unwrap();
        std::fs::write(&tmp_path, b"partial").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"original");
    }

    #[test]
    fn recover_bdk_crash_after_bak_rename() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let bak_path = dir.path().join("bdk_wallet_sqlite_test.db.bak");
        let tmp_path = dir.path().join("bdk_wallet_sqlite_test.db.enc.tmp");

        // simulate: old→.bak done, encrypted tmp exists, final rename didn't happen
        create_plaintext_bdk_db_at(&bak_path);
        create_encrypted_db_at(&tmp_path);

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        // .bak cleaned because encrypted DB verified successfully
        assert!(!bak_path.exists());
        assert!(verify_encrypted_bdk_db(&db_path).unwrap());
    }

    #[test]
    fn recovery_target_path_matches_db_and_bak() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let bak_path = dir.path().join("bdk_wallet_sqlite_test.db.bak");
        let other_path = dir.path().join("other.db.bak");

        assert_eq!(recovery_target_path(&db_path), Some(db_path.clone()));
        assert_eq!(recovery_target_path(&bak_path), Some(db_path));
        assert_eq!(recovery_target_path(&other_path), None);
    }

    #[test]
    fn recovery_target_path_matches_enc_tmp() {
        let dir = TempDir::new().unwrap();
        let tmp_path = dir.path().join("bdk_wallet_sqlite_test.db.enc.tmp");
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        assert_eq!(recovery_target_path(&tmp_path), Some(db_path));

        // non-BDK enc.tmp should not match
        let other_tmp = dir.path().join("other.db.enc.tmp");
        assert_eq!(recovery_target_path(&other_tmp), None);
    }

    #[test]
    fn recover_bdk_cleans_auxiliary_files() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let bak_path = dir.path().join("bdk_wallet_sqlite_test.db.bak");

        // use a real encrypted DB so verification passes
        create_encrypted_db_at(&db_path);
        create_plaintext_bdk_db_at(&bak_path);
        std::fs::write(sqlite_auxiliary_path(&db_path, "wal"), b"wal").unwrap();
        std::fs::write(sqlite_auxiliary_path(&db_path, "shm"), b"shm").unwrap();
        std::fs::write(sqlite_auxiliary_path(&bak_path, "wal"), b"wal").unwrap();
        std::fs::write(sqlite_auxiliary_path(&bak_path, "shm"), b"shm").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(!sqlite_auxiliary_path(&db_path, "wal").exists());
        assert!(!sqlite_auxiliary_path(&db_path, "shm").exists());
        assert!(!sqlite_auxiliary_path(&bak_path, "wal").exists());
        assert!(!sqlite_auxiliary_path(&bak_path, "shm").exists());
    }

    #[test]
    fn recover_bak_only() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let bak_path = dir.path().join("bdk_wallet_sqlite_test.db.bak");

        // only .bak exists, no db or tmp
        std::fs::write(&bak_path, b"backup_data").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!bak_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), b"backup_data");
    }

    #[test]
    fn migrate_path_with_single_quote() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let quoted_dir = dir.path().join("bob's wallet");
        std::fs::create_dir_all(&quoted_dir).unwrap();

        let db_path = quoted_dir.join("bdk.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE test_data (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test_data (id, value) VALUES (1, 'hello');",
        )
        .unwrap();
        drop(conn);

        assert!(is_plaintext_sqlite(&db_path));
        migrate_single_bdk_database(&db_path).unwrap();
        assert!(!is_plaintext_sqlite(&db_path));

        // verify data survived
        let key = encrypted_backend::encryption_key().unwrap();
        let hex_key = format!("x'{}'", hex::encode(key));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "key", &hex_key).unwrap();

        let value: String = conn
            .query_row("SELECT value FROM test_data WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "hello");
    }

    #[test]
    fn migrate_empty_bdk_database_succeeds() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_empty.db");

        // create a valid SQLite file with header but no user tables
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "wal").unwrap();
        drop(conn);

        assert!(is_plaintext_sqlite(&db_path));
        migrate_single_bdk_database(&db_path).unwrap();

        // empty db should just be removed
        assert!(!db_path.exists());
    }

    #[test]
    fn orphan_enc_tmp_cleaned_by_recovery() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let tmp_path = dir.path().join("bdk_wallet_sqlite_test.db.enc.tmp");

        // only orphan .enc.tmp exists
        std::fs::write(&tmp_path, b"orphan").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(!tmp_path.exists());
        assert!(!db_path.exists());
    }

    #[test]
    fn bak_retained_after_migration() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_bdk_db(&dir, "bdk_wallet_sqlite_test.db");
        let bak_path = db_path.with_extension("db.bak");

        migrate_single_bdk_database(&db_path).unwrap();

        assert!(db_path.exists(), "encrypted DB should exist");
        assert!(bak_path.exists(), ".bak should be retained after migration");
        assert!(!is_plaintext_sqlite(&db_path), "DB should be encrypted");
        assert!(is_plaintext_sqlite(&bak_path), ".bak should be plaintext");
    }

    #[test]
    fn bak_cleaned_on_recovery_when_encrypted_db_valid() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = create_plaintext_bdk_db(&dir, "bdk_wallet_sqlite_test.db");
        let bak_path = db_path.with_extension("db.bak");

        // migrate (leaves .bak)
        migrate_single_bdk_database(&db_path).unwrap();
        assert!(bak_path.exists());

        // simulate next launch recovery
        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists(), "encrypted DB should still exist");
        assert!(!bak_path.exists(), ".bak should be cleaned after verification");
    }

    #[test]
    fn bak_restored_when_encrypted_db_invalid() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");
        let bak_path = db_path.with_extension("db.bak");

        // write garbage to db_path (simulating corruption)
        std::fs::write(&db_path, b"corrupt_encrypted_data").unwrap();
        create_plaintext_bdk_db_at(&bak_path);

        recover_at_path(&db_path).unwrap();

        assert!(db_path.exists());
        assert!(!bak_path.exists());
        // should have restored the plaintext backup
        assert!(is_plaintext_sqlite(&db_path));
    }

    #[test]
    fn mixed_directory_only_migrates_matching_plaintext() {
        setup_test_key();
        let dir = TempDir::new().unwrap();

        // matching plaintext BDK databases — should be migrated
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_aaa.db"));
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_bbb.db"));

        // already encrypted — should be skipped
        create_encrypted_db_at(&dir.path().join("bdk_wallet_sqlite_ccc.db"));

        // non-BDK files — should be ignored
        create_plaintext_bdk_db_at(&dir.path().join("other.db"));
        std::fs::write(dir.path().join("readme.txt"), b"hello").unwrap();

        let migration = test_migration(2);
        BdkMigration::with_dir(dir.path().to_path_buf(), Arc::clone(&migration)).run().unwrap();

        assert_eq!(migration.progress().current, 2, "should tick once per migrated DB");

        // plaintext BDK DBs should now be encrypted
        assert!(!is_plaintext_sqlite(&dir.path().join("bdk_wallet_sqlite_aaa.db")));
        assert!(!is_plaintext_sqlite(&dir.path().join("bdk_wallet_sqlite_bbb.db")));

        // already-encrypted should still verify
        assert!(verify_encrypted_bdk_db(&dir.path().join("bdk_wallet_sqlite_ccc.db")).unwrap());

        // non-BDK should be untouched
        assert!(is_plaintext_sqlite(&dir.path().join("other.db")));
        assert!(dir.path().join("readme.txt").exists());
    }

    #[test]
    fn migrate_errors_on_unreadable_dir() {
        // point at a file instead of a directory — read_dir will fail
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("not_a_dir");
        std::fs::write(&file_path, b"nope").unwrap();

        let migration = test_migration(0);
        let result = BdkMigration::with_dir(file_path, migration).run();
        assert!(result.is_err());
    }

    #[test]
    fn recover_errors_on_unreadable_dir() {
        // point at a file instead of a directory — read_dir will fail
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("not_a_dir");
        std::fs::write(&file_path, b"nope").unwrap();

        let result = recover_interrupted_bdk_migrations_in_dir(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn recover_preserves_wal_for_non_migration_db() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        // normal encrypted DB with no recovery artifacts
        create_encrypted_db_at(&db_path);
        let wal_path = sqlite_auxiliary_path(&db_path, "wal");
        let shm_path = sqlite_auxiliary_path(&db_path, "shm");
        std::fs::write(&wal_path, b"wal data").unwrap();
        std::fs::write(&shm_path, b"shm data").unwrap();

        recover_at_path(&db_path).unwrap();

        assert!(wal_path.exists(), "WAL should be preserved for non-migration DB");
        assert!(shm_path.exists(), "SHM should be preserved for non-migration DB");
    }

    #[test]
    fn migrate_continues_past_bad_db() {
        setup_test_key();
        let dir = TempDir::new().unwrap();

        // corrupt plaintext DB (has SQLite header but invalid content)
        let bad_path = dir.path().join("bdk_wallet_sqlite_bad.db");
        let conn = rusqlite::Connection::open(&bad_path).unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        drop(conn);
        // corrupt the file after the header so sqlcipher_export fails
        let mut data = std::fs::read(&bad_path).unwrap();
        if data.len() > 200 {
            for byte in data[100..200].iter_mut() {
                *byte = 0xFF;
            }
        }
        std::fs::write(&bad_path, &data).unwrap();

        // valid plaintext DB
        let good_path = dir.path().join("bdk_wallet_sqlite_good.db");
        create_plaintext_bdk_db_at(&good_path);

        let migration = test_migration(2);
        let result = BdkMigration::with_dir(dir.path().to_path_buf(), Arc::clone(&migration)).run();

        // should report error
        assert!(result.is_err());

        // both databases should tick (even the failed one) to keep watchdog happy
        assert_eq!(migration.progress().current, 2, "should tick for both success and failure");
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("bdk_wallet_sqlite_bad.db"), "error should mention the bad DB");

        // good DB should still have been migrated
        assert!(!is_plaintext_sqlite(&good_path), "good DB should be encrypted");
        assert!(verify_encrypted_bdk_db(&good_path).unwrap(), "good DB should be valid");
    }

    #[test]
    fn wal_data_survives_checkpoint_then_cleanup() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        // create encrypted DB in WAL mode with data left in WAL (not checkpointed)
        let key = encrypted_backend::encryption_key().unwrap();
        let hex_key = format!("x'{}'", hex::encode(key));
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "key", &hex_key).unwrap();
            conn.pragma_update(None, "journal_mode", "wal").unwrap();
            conn.execute_batch(
                "CREATE TABLE test_data (id INTEGER PRIMARY KEY, value TEXT);
                 INSERT INTO test_data (id, value) VALUES (1, 'wal_survives');",
            )
            .unwrap();
            // simulate crash — leak connection to prevent clean checkpoint on Drop
            std::mem::forget(conn);
        }

        assert!(
            sqlite_auxiliary_path(&db_path, "wal").exists(),
            "WAL file should exist before recovery"
        );

        // create a .bak to trigger had_recovery_artifacts
        let bak_path = db_path.with_extension("db.bak");
        std::fs::write(&bak_path, b"stale_backup").unwrap();

        recover_at_path(&db_path).unwrap();

        // wal/shm should be removed after checkpoint
        assert!(!sqlite_auxiliary_path(&db_path, "wal").exists());
        assert!(!sqlite_auxiliary_path(&db_path, "shm").exists());

        // data from WAL should have been checkpointed into the main file
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "key", &hex_key).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM test_data WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "wal_survives");
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_failure_preserves_wal() {
        use std::os::unix::fs::PermissionsExt as _;

        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        // unreadable file so Connection::open fails without SQLite touching WAL/SHM
        std::fs::write(&db_path, b"x").unwrap();

        let wal_path = sqlite_auxiliary_path(&db_path, "wal");
        let shm_path = sqlite_auxiliary_path(&db_path, "shm");
        std::fs::write(&wal_path, b"wal data").unwrap();
        std::fs::write(&shm_path, b"shm data").unwrap();

        std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o000)).unwrap();

        checkpoint_and_clean_auxiliary_files(&db_path);

        // restore permissions so TempDir cleanup works
        std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(wal_path.exists(), "WAL should be preserved when checkpoint fails");
        assert!(shm_path.exists(), "SHM should be preserved when checkpoint fails");
    }

    #[cfg(unix)]
    #[test]
    fn recover_at_path_propagates_rename_error() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("restricted");
        std::fs::create_dir(&sub).unwrap();

        let db = sub.join("bdk_wallet_sqlite_test.db");
        let bak = db.with_extension("db.bak");
        std::fs::write(&bak, b"backup_data").unwrap();

        // make directory read-only so rename fails
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = recover_at_path(&db);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err(), "recover_at_path should propagate rename errors");
    }

    #[test]
    fn bdk_migration_stops_on_cancellation() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_aaa.db"));
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_bbb.db"));
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_ccc.db"));

        let cancelled = Arc::new(AtomicBool::new(true));
        let migration = Arc::new(Migration::new(3, cancelled));
        let result = BdkMigration::with_dir(dir.path().to_path_buf(), migration).run();

        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("cancelled"), "error should mention cancellation: {err_msg}");
    }

    #[test]
    fn bdk_migration_mid_flight_cancellation() {
        setup_test_key();

        let dir = TempDir::new().unwrap();
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_aaa.db"));
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_bbb.db"));
        create_plaintext_bdk_db_at(&dir.path().join("bdk_wallet_sqlite_ccc.db"));

        let cancelled = Arc::new(AtomicBool::new(false));
        let migration = Arc::new(Migration::new(3, Arc::clone(&cancelled)));

        // cancel from a background thread after the first database is migrated
        let progress_handle = Arc::clone(&migration);
        let cancel_flag = Arc::clone(&cancelled);
        let watcher = std::thread::spawn(move || {
            loop {
                if progress_handle.progress().current >= 1 {
                    cancel_flag.store(true, std::sync::atomic::Ordering::Release);
                    return;
                }
                std::thread::yield_now();
            }
        });

        let result = BdkMigration::with_dir(dir.path().to_path_buf(), Arc::clone(&migration)).run();
        watcher.join().unwrap();

        let progress = migration.progress();
        assert!(progress.current >= 1, "should have migrated at least one DB");

        // migration may or may not have finished all 3 before cancellation took effect
        // (cancellation is cooperative — checked between database operations)
        if progress.current < 3 {
            assert!(result.is_err());
            let err_msg = format!("{:#}", result.unwrap_err());
            assert!(err_msg.contains("cancelled"), "error should mention cancellation: {err_msg}");
        }
    }

    #[test]
    fn integrity_check_catches_corruption() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        create_encrypted_db_at(&db_path);
        assert!(verify_encrypted_bdk_db(&db_path).unwrap(), "valid DB should pass verification");

        // corrupt middle bytes of the encrypted DB
        let mut data = std::fs::read(&db_path).unwrap();
        let mid = data.len() / 2;
        for byte in data[mid..mid + 64].iter_mut() {
            *byte ^= 0xFF;
        }
        std::fs::write(&db_path, &data).unwrap();

        assert!(
            !verify_encrypted_bdk_db(&db_path).unwrap(),
            "corrupted DB should fail verification"
        );
    }
}
