use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use eyre::{Context as _, Result, bail};
use tracing::{info, warn};

use crate::bdk_store::sqlite_auxiliary_path;

/// Checkpoint WAL data into the main DB file before removing auxiliary files
///
/// Prevents losing uncheckpointed writes when recovery artifacts trigger cleanup
pub(super) fn checkpoint_and_clean_auxiliary_files(db_path: &Path) {
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

pub(super) fn rename_auxiliary_files(source_path: &Path, destination_path: &Path) -> Result<()> {
    for suffix in ["wal", "shm"] {
        let source_aux_path = sqlite_auxiliary_path(source_path, suffix);
        if !source_aux_path.exists() {
            continue;
        }

        let destination_aux_path = sqlite_auxiliary_path(destination_path, suffix);
        crate::database::migration::log_remove_file(&destination_aux_path);
        std::fs::rename(&source_aux_path, &destination_aux_path).context(format!(
            "failed to rename {} to {}",
            source_aux_path.display(),
            destination_aux_path.display()
        ))?;
    }

    Ok(())
}

pub(super) fn finalize_sqlite_bundle_move(
    source_path: &Path,
    destination_path: &Path,
) -> Result<()> {
    clean_auxiliary_files(destination_path);
    std::fs::rename(source_path, destination_path).context(format!(
        "failed to rename {} to {}",
        source_path.display(),
        destination_path.display()
    ))?;
    rename_auxiliary_files(source_path, destination_path)
}

pub(super) fn recover_interrupted_bdk_migrations_in_dir(dir: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            bail!("Failed to read data directory for BDK recovery: {e}");
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

pub(super) fn recovery_target_path(path: &Path) -> Option<PathBuf> {
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

pub(super) fn recover_at_path(db_path: &Path) -> Result<()> {
    let bak_path = db_path.with_extension("db.bak");
    let tmp_path = db_path.with_extension("db.enc.tmp");

    // only clean WAL/SHM when recovery artifacts (.bak/.enc.tmp) are present,
    // otherwise a normal DB's uncommitted WAL data would be lost
    let had_recovery_artifacts = bak_path.exists() || tmp_path.exists();

    // only backup exists: migration completed but final rename didn't happen
    if bak_path.exists() && !db_path.exists() && !tmp_path.exists() {
        let bak = bak_path.display();
        warn!("Only backup exists at {bak} -- restoring from backup");
        finalize_sqlite_bundle_move(&bak_path, db_path)
            .context(format!("failed to restore from backup at {bak}"))?;
        return Ok(());
    }

    if tmp_path.exists() && bak_path.exists() && !db_path.exists() {
        // crash after old→.bak, before tmp→final: finish the rename
        let path = db_path.display();
        info!("Recovering interrupted BDK migration at {path}");
        checkpoint_and_clean_auxiliary_files(&tmp_path);
        finalize_sqlite_bundle_move(&tmp_path, db_path)
            .context(format!("failed to finish interrupted BDK migration at {path}"))?;
    }

    if tmp_path.exists() {
        crate::database::migration::log_remove_file(&tmp_path);
        clean_auxiliary_files(&tmp_path);
    }

    if bak_path.exists() && db_path.exists() {
        match super::verify_encrypted_bdk_db(db_path) {
            Ok(true) => {
                // encrypted DB confirmed working — safe to delete backup
                crate::database::migration::log_remove_file(&bak_path);
                clean_auxiliary_files(&bak_path);
            }
            Ok(false) => {
                // encrypted DB is corrupt, restore from backup
                let path = db_path.display();
                warn!("Encrypted DB at {path} appears corrupt, restoring from backup");
                crate::database::migration::log_remove_file(db_path);
                clean_auxiliary_files(db_path);
                finalize_sqlite_bundle_move(&bak_path, db_path)
                    .context("failed to restore from backup")?;
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

pub(super) fn clean_auxiliary_files(db_path: &Path) {
    for suffix in ["wal", "shm"] {
        let aux_path = sqlite_auxiliary_path(db_path, suffix);
        crate::database::migration::log_remove_file(&aux_path);
    }
}
