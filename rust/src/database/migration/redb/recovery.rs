use std::path::Path;

use eyre::{Context as _, Result};
use tracing::{info, warn};

use cove_common::consts::{ROOT_DATA_DIR, WALLET_DATA_DIR};

use super::{
    DatabasePaths, LEGACY_MAIN_DB, LEGACY_WALLET_DB, main_database_paths, wallet_database_paths,
};

pub(super) fn recover_interrupted_main_migration() -> Result<()> {
    recover_main_migration(&ROOT_DATA_DIR)?;
    recover_legacy_at_path(&ROOT_DATA_DIR.join(LEGACY_MAIN_DB))
}

pub(super) fn recover_interrupted_wallet_migrations() -> Result<()> {
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
        recover_wallet_migration(&entry.path())?;
        recover_legacy_at_path(&entry.path().join(LEGACY_WALLET_DB))?;
    }

    Ok(())
}

pub(super) fn recover_main_migration(root_dir: &Path) -> Result<()> {
    let paths = main_database_paths(root_dir);
    recover_promoted_database(
        &paths,
        "failed to finish interrupted main DB migration",
        "Encrypted DB failed verification, preserving plaintext",
    )
}

pub(super) fn recover_wallet_migration(wallet_dir: &Path) -> Result<()> {
    let paths = wallet_database_paths(wallet_dir);
    recover_promoted_database(
        &paths,
        "failed to finish interrupted wallet DB migration",
        "Encrypted wallet DB failed verification, preserving plaintext",
    )
}

fn recover_promoted_database(
    paths: &DatabasePaths,
    rename_context: &str,
    verification_warning: &str,
) -> Result<()> {
    if paths.tmp.exists() && !paths.dest.exists() {
        if paths.source.exists() {
            let tmp = paths.tmp.display();
            info!("Removing unpromoted migration temp at {tmp}; source still exists");
            super::super::log_remove_file(&paths.tmp);
        } else {
            match super::verify_encrypted_redb_db(&paths.tmp) {
                Ok(true) => {
                    std::fs::rename(&paths.tmp, &paths.dest)
                        .with_context(|| rename_context.to_string())?;
                }
                _ => {
                    let tmp = paths.tmp.display();
                    warn!("Migration temp at {tmp} failed verification and no source exists");
                }
            }
        }
    } else if paths.tmp.exists() {
        if paths.source.exists() || matches!(super::verify_encrypted_redb_db(&paths.dest), Ok(true))
        {
            super::super::log_remove_file(&paths.tmp);
        } else {
            let tmp = paths.tmp.display();
            warn!("Preserving migration temp at {tmp} because destination failed verification");
        }
    }

    // clean up leftover plaintext only after verifying encrypted version works
    if paths.source.exists() && paths.dest.exists() {
        match super::verify_encrypted_redb_db(&paths.dest) {
            Ok(true) => super::super::log_remove_file(&paths.source),
            Ok(false) => {
                warn!("{verification_warning}");

                let preserved = preserve_corrupt_file(&paths.dest)?;
                let preserved = preserved.display();
                warn!("Preserved corrupt encrypted DB at {preserved}");
            }
            Err(error) => warn!("{verification_warning}: {error:#}"),
        }
    }

    Ok(())
}

/// Legacy recovery for old-style .bak/.enc.tmp files from the previous migration code
pub(super) fn recover_legacy_at_path(db_path: &Path) -> Result<()> {
    let extension = db_path.extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default();
    let bak_path = db_path.with_extension(format!("{extension}.bak"));
    let tmp_path = db_path.with_extension(format!("{extension}.enc.tmp"));

    // only .bak exists: old migration completed but final rename didn't happen
    if bak_path.exists() && !db_path.exists() && !tmp_path.exists() {
        let bak = bak_path.display();
        warn!("Only legacy backup exists at {bak} -- restoring from backup");
        std::fs::rename(&bak_path, db_path)
            .context(format!("failed to restore from legacy backup at {bak}"))?;
        return Ok(());
    }

    // both .enc.tmp and .bak exist but db missing: crashed during old two-phase swap
    // after old→.bak rename but before tmp→final rename
    if tmp_path.exists() && bak_path.exists() && !db_path.exists() {
        let path = db_path.display();
        info!("Recovering interrupted legacy migration at {path}");
        match super::verify_encrypted_redb_db(&tmp_path) {
            Ok(true) => {
                std::fs::rename(&tmp_path, db_path)
                    .context(format!("failed to finish interrupted legacy migration at {path}"))?;
                super::super::log_remove_file(&bak_path);
            }
            _ => {
                warn!("Legacy .enc.tmp at {path} is corrupt, restoring from backup");
                super::super::log_remove_file(&tmp_path);
                std::fs::rename(&bak_path, db_path)
                    .context(format!("failed to restore from legacy backup at {path}"))?;
            }
        }
        return Ok(());
    }

    if tmp_path.exists() && !db_path.exists() {
        let path = db_path.display();
        match super::verify_encrypted_redb_db(&tmp_path) {
            Ok(true) => {
                info!("Promoting verified legacy migration temp at {path}");
                std::fs::rename(&tmp_path, db_path)
                    .context(format!("failed to promote legacy migration temp at {path}"))?;
            }
            _ => {
                let tmp = tmp_path.display();
                warn!("Legacy migration temp at {tmp} failed verification and no backup exists");
            }
        }
        return Ok(());
    }

    if tmp_path.exists() {
        super::super::log_remove_file(&tmp_path);
    }

    if bak_path.exists() && db_path.exists() {
        match super::verify_encrypted_redb_db(db_path) {
            Ok(true) => super::super::log_remove_file(&bak_path),
            Ok(false) => {
                let path = db_path.display();
                warn!("Encrypted DB at {path} appears corrupt, restoring from legacy backup");

                let preserved = preserve_corrupt_file(db_path)?;
                let preserved = preserved.display();
                warn!("Preserved corrupt encrypted DB at {preserved}");
                std::fs::rename(&bak_path, db_path)
                    .context(format!("failed to restore from legacy backup at {path}"))?;
            }
            Err(e) => {
                let path = db_path.display();
                warn!("Cannot verify DB at {path}: {e:#} — preserving both files");
            }
        }
    }

    if bak_path.exists() && !db_path.exists() {
        let bak = bak_path.display();
        warn!("Restoring legacy backup at {bak}");
        std::fs::rename(&bak_path, db_path)
            .context(format!("failed to restore from legacy backup at {bak}"))?;
    }

    Ok(())
}

fn preserve_corrupt_file(path: &Path) -> Result<std::path::PathBuf> {
    let extension = path.extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default();

    for index in 0..1000 {
        let suffix = if index == 0 {
            format!("{extension}.corrupt")
        } else {
            format!("{extension}.corrupt.{index}")
        };
        let candidate = path.with_extension(suffix);
        if candidate.exists() {
            continue;
        }

        std::fs::rename(path, &candidate).with_context(|| {
            format!("failed to preserve corrupt database at {}", candidate.to_string_lossy())
        })?;
        return Ok(candidate);
    }

    eyre::bail!("failed to find a path for preserving corrupt database {}", path.display())
}
