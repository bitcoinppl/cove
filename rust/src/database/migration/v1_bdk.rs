use std::{collections::HashSet, path::Path};

use eyre::{Context as _, Result};
use tracing::{info, warn};

use crate::bdk_store::sqlite_auxiliary_path;
use cove_common::consts::ROOT_DATA_DIR;

/// Check whether an encrypted BDK database can be opened and read
fn verify_encrypted_bdk_db(path: &Path) -> bool {
    let Some(key) = crate::database::encrypted_backend::encryption_key() else {
        return false;
    };

    let hex_key = format!("x'{}'", hex::encode(key));

    let Ok(conn) = rusqlite::Connection::open(path) else {
        return false;
    };

    if conn.pragma_update(None, "key", &hex_key).is_err() {
        return false;
    };

    if conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_err()
    {
        return false;
    }

    // page-level integrity check catches corruption a schema read would miss
    matches!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)),
        Ok(ref result) if result == "ok"
    )
}

fn log_remove_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!("Failed to remove {}: {e}", path.display()),
    }
}

/// Check if a file is a plaintext (unencrypted) SQLite database
pub fn is_plaintext_sqlite(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
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
        .expect("encryption key must be set before BDK migration");
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
        warn!("No encryption key — preserving WAL/SHM at {}", db_path.display());
        return;
    };

    let hex_key = format!("x'{}'", hex::encode(key));
    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        warn!("Cannot open DB for checkpoint at {} — preserving WAL/SHM", db_path.display());
        return;
    };
    if conn.pragma_update(None, "key", &hex_key).is_err() {
        warn!("Cannot set key for checkpoint at {} — preserving WAL/SHM", db_path.display());
        return;
    }

    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get::<_, i32>(0)) {
        Ok(0) => clean_auxiliary_files(db_path),
        Ok(busy) => {
            warn!("WAL checkpoint busy={busy} at {} — preserving WAL/SHM", db_path.display())
        }
        Err(e) => {
            warn!("WAL checkpoint failed at {}: {e} — preserving WAL/SHM", db_path.display())
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
        recover_at_path(&target);
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

fn recover_at_path(db_path: &Path) {
    let bak_path = db_path.with_extension("db.bak");
    let tmp_path = db_path.with_extension("db.enc.tmp");

    // only clean WAL/SHM when recovery artifacts (.bak/.enc.tmp) are present,
    // otherwise a normal DB's uncommitted WAL data would be lost
    let had_recovery_artifacts = bak_path.exists() || tmp_path.exists();

    // only backup exists: migration completed but final rename didn't happen
    if bak_path.exists() && !db_path.exists() && !tmp_path.exists() {
        warn!("Only backup exists at {} -- restoring from backup", bak_path.display());
        if let Err(e) = std::fs::rename(&bak_path, db_path) {
            warn!("Failed to restore from backup at {}: {e}", bak_path.display());
        }
        return;
    }

    if tmp_path.exists() && bak_path.exists() && !db_path.exists() {
        // crash after old→.bak, before tmp→final: finish the rename
        info!("Recovering interrupted BDK migration at {}", db_path.display());
        if let Err(e) = std::fs::rename(&tmp_path, db_path) {
            warn!("Failed to finish interrupted BDK migration at {}: {e}", db_path.display());
            return;
        }
    }

    if tmp_path.exists() {
        log_remove_file(&tmp_path);
    }

    if bak_path.exists() && db_path.exists() {
        if verify_encrypted_bdk_db(db_path) {
            // encrypted DB confirmed working — safe to delete backup
            log_remove_file(&bak_path);
            clean_auxiliary_files(&bak_path);
        } else {
            // encrypted DB is invalid, restore from backup
            warn!("Encrypted DB at {} appears invalid, restoring from backup", db_path.display());
            log_remove_file(db_path);
            clean_auxiliary_files(db_path);
            if let Err(e) = std::fs::rename(&bak_path, db_path) {
                warn!("Failed to restore from backup: {e}");
            }
        }
    }

    if had_recovery_artifacts {
        checkpoint_and_clean_auxiliary_files(db_path);
    }
}

/// Migrate all plaintext BDK SQLite databases to SQLCipher
pub fn migrate_bdk_databases_if_needed() -> Result<()> {
    migrate_bdk_databases_in_dir(&ROOT_DATA_DIR)
}

fn migrate_bdk_databases_in_dir(dir: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            eyre::bail!("Failed to read data directory for BDK migration: {e}");
        }
    };

    let mut errors: Vec<String> = Vec::new();

    for entry in entries {
        let entry = entry.context("failed to read directory entry during BDK migration")?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();

        if name.starts_with("bdk_wallet_sqlite_")
            && name.ends_with(".db")
            && is_plaintext_sqlite(&path)
        {
            info!("Migrating BDK database at {}", path.display());
            if let Err(e) = migrate_single_bdk_database(&path) {
                warn!("Failed to migrate BDK database {}: {e:#}", path.display());
                errors.push(format!("{}: {e:#}", path.display()));
            }
        }
    }

    if !errors.is_empty() {
        eyre::bail!("failed to migrate {} BDK database(s): {}", errors.len(), errors.join("; "));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::encrypted_backend;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_test_key() {
        encrypted_backend::set_test_encryption_key();
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

        recover_at_path(&db_path);

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

        recover_at_path(&db_path);

        assert!(db_path.exists());
        assert!(!tmp_path.exists());
        // .bak cleaned because encrypted DB verified successfully
        assert!(!bak_path.exists());
        assert!(verify_encrypted_bdk_db(&db_path));
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

        recover_at_path(&db_path);

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

        recover_at_path(&db_path);

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

        recover_at_path(&db_path);

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
        recover_at_path(&db_path);

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

        recover_at_path(&db_path);

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

        migrate_bdk_databases_in_dir(dir.path()).unwrap();

        // plaintext BDK DBs should now be encrypted
        assert!(!is_plaintext_sqlite(&dir.path().join("bdk_wallet_sqlite_aaa.db")));
        assert!(!is_plaintext_sqlite(&dir.path().join("bdk_wallet_sqlite_bbb.db")));

        // already-encrypted should still verify
        assert!(verify_encrypted_bdk_db(&dir.path().join("bdk_wallet_sqlite_ccc.db")));

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

        let result = migrate_bdk_databases_in_dir(&file_path);
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

        recover_at_path(&db_path);

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

        let result = migrate_bdk_databases_in_dir(dir.path());

        // should report error
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("bdk_wallet_sqlite_bad.db"), "error should mention the bad DB");

        // good DB should still have been migrated
        assert!(!is_plaintext_sqlite(&good_path), "good DB should be encrypted");
        assert!(verify_encrypted_bdk_db(&good_path), "good DB should be valid");
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

        recover_at_path(&db_path);

        // WAL/SHM should be removed after checkpoint
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

    #[test]
    fn integrity_check_catches_corruption() {
        setup_test_key();
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bdk_wallet_sqlite_test.db");

        create_encrypted_db_at(&db_path);
        assert!(verify_encrypted_bdk_db(&db_path), "valid DB should pass verification");

        // corrupt middle bytes of the encrypted DB
        let mut data = std::fs::read(&db_path).unwrap();
        let mid = data.len() / 2;
        for byte in data[mid..mid + 64].iter_mut() {
            *byte ^= 0xFF;
        }
        std::fs::write(&db_path, &data).unwrap();

        assert!(!verify_encrypted_bdk_db(&db_path), "corrupted DB should fail verification");
    }
}
