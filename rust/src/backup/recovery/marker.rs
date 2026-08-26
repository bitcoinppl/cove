use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

use cove_types::WalletId;
use cove_util::ResultExt as _;
use rand::RngExt as _;
use tracing::{info, warn};

use crate::database::Database;

use super::{
    BackupError, LOCK_NOFOLLOW, MARKER_DIRECTORY_NAME, MARKER_EXTENSION, MAX_MARKER_BYTES,
    PersistedArtifactSnapshot, PersistedRestoreMarker, QUARANTINE_DIRECTORY_NAME,
    RESTORE_MARKER_VERSION, RestoreArtifactSnapshot, RestoreFileLock, RestoreMarkerPhase,
    ValidatedRestoreWalletId, bdk_artifacts, delete_keychain_items_exact, ensure_directory,
    hash_bytes, hash_wallet_data_entry, lock_directory, restore_path_key, rollback_bdk_artifacts,
    rollback_keychain, rollback_wallet_data_while_gated,
};

pub(crate) fn operation_id() -> String {
    let bytes = rand::rng().random::<[u8; 16]>();
    hex::encode(bytes)
}

/// Remove every durable restore-recovery artifact during a catastrophic reset
///
/// A reset that wipes wallet data must also wipe markers and locks, or the next
/// bootstrap replays recovery against data that no longer exists
pub(crate) fn remove_all_restore_recovery_state() -> std::io::Result<()> {
    for directory in [marker_directory(), lock_directory()] {
        match fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

pub(crate) fn marker_directory() -> PathBuf {
    cove_common::consts::ROOT_DATA_DIR.join(MARKER_DIRECTORY_NAME)
}

fn quarantine_marker(path: &Path) -> Result<(), String> {
    let directory = marker_directory().join(QUARANTINE_DIRECTORY_NAME);
    ensure_directory(&directory, "restore marker quarantine").map_err(|error| error.to_string())?;
    let file_name = path.file_name().ok_or_else(|| "restore marker has no filename".to_string())?;
    let mut target = directory.join(file_name);
    if target.exists() {
        let suffix = hash_bytes(path.to_string_lossy().as_bytes());
        target.set_file_name(format!("{}.{}", file_name.to_string_lossy(), &suffix[..12]));
    }
    match fs::rename(path, &target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    }
    sync_parent(path.parent().unwrap_or(&directory)).map_err(|error| error.to_string())?;
    sync_parent(&directory).map_err(|error| error.to_string())
}

fn recover_temporary_marker(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        return Err("restore marker temp is a symbolic link".to_string());
    }
    if !metadata.is_file() {
        return Err("restore marker temp is not a regular file".to_string());
    }

    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    }
    sync_parent(path.parent().ok_or_else(|| "restore marker temp has no parent".to_string())?)
        .map_err(|error| error.to_string())
}

pub(crate) fn marker_path(operation_id: &str) -> Result<PathBuf, BackupError> {
    if operation_id.is_empty()
        || operation_id.len() > 64
        || !operation_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(BackupError::Restore("invalid restore operation id".to_string()));
    }

    Ok(marker_directory().join(format!("{operation_id}.{MARKER_EXTENSION}")))
}

pub(crate) fn write_marker(
    path: &Path,
    marker: &PersistedRestoreMarker,
) -> Result<(), BackupError> {
    let parent = path.parent().ok_or_else(|| {
        BackupError::Restore("restore marker has no parent directory".to_string())
    })?;

    ensure_directory(parent, "restore marker")?;

    let bytes = serde_json::to_vec(marker)
        .map_err(|error| BackupError::Serialization(format!("restore marker: {error}")))?;

    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(BackupError::Restore("restore marker is too large".to_string()));
    }

    let temp_path =
        path.with_file_name(format!(".{}.tmp", path.file_name().unwrap().to_string_lossy()));

    let mut file = match OpenOptions::new().create_new(true).write(true).open(&temp_path) {
        Ok(file) => file,

        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&temp_path).map_err(|error| {
                BackupError::Restore(format!("failed to inspect restore marker temp: {error}"))
            })?;

            if metadata.file_type().is_symlink() {
                return Err(BackupError::Restore(
                    "restore marker temp is a symbolic link".to_string(),
                ));
            }

            fs::remove_file(&temp_path).map_err(|error| {
                BackupError::Restore(format!("failed to remove stale restore marker temp: {error}"))
            })?;

            sync_parent(parent).map_err(|error| {
                BackupError::Restore(format!("failed to sync restore marker directory: {error}"))
            })?;

            OpenOptions::new().create_new(true).write(true).open(&temp_path).map_err(|error| {
                BackupError::Restore(format!("failed to create restore marker temp: {error}"))
            })?
        }

        Err(error) => {
            return Err(BackupError::Restore(format!(
                "failed to create restore marker temp: {error}"
            )));
        }
    };

    let write_result = (|| -> io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        sync_parent(parent)
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(BackupError::Restore(format!("failed to persist restore marker: {error}")));
    }

    Ok(())
}

pub(crate) fn remove_marker(path: &Path) -> Result<(), BackupError> {
    let parent = path.parent().ok_or_else(|| {
        BackupError::Restore("restore marker has no parent directory".to_string())
    })?;

    match fs::remove_file(path) {
        Ok(()) => sync_parent(parent).map_err(|error| {
            BackupError::Restore(format!("failed to sync restore marker directory: {error}"))
        }),

        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            sync_parent(parent).map_err(|error| {
                BackupError::Restore(format!("failed to sync restore marker directory: {error}"))
            })
        }

        Err(error) => {
            Err(BackupError::Restore(format!("failed to remove restore marker: {error}")))
        }
    }
}

fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

fn read_marker(path: &Path) -> Result<PersistedRestoreMarker, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("restore marker is a symbolic link".to_string());
    }

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(LOCK_NOFOLLOW)
        .open(path)
        .map_err(|error| error.to_string())?;

    let mut limited = file.take(MAX_MARKER_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|error| error.to_string())?;

    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err("restore marker is too large".to_string());
    }

    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

/// Report whether metadata for this exact wallet id exists on the device
///
/// Rejects any stored wallet whose id only differs by case: it would share the
/// restore-owned paths of `id` without being the same wallet
pub(crate) fn metadata_exists(id: &ValidatedRestoreWalletId) -> Result<bool, BackupError> {
    let database = Database::global();
    let mut exact_match = false;

    let inventory = database.wallets.complete_inventory().map_err_str(BackupError::Database)?;

    for wallet in inventory {
        if wallet.id == *id.as_wallet_id() {
            exact_match = true;
        } else if restore_path_key(wallet.id.as_str()) == id.path_key() {
            return Err(BackupError::InvalidWalletId(format!(
                "wallet id collides with existing path: {}",
                id.as_wallet_id()
            )));
        }
    }

    Ok(exact_match)
}

/// Capture the current artifacts, recovering once from an unreadable keychain
fn capture_after_keychain_recovery(
    id: &ValidatedRestoreWalletId,
    initial: &RestoreArtifactSnapshot,
    context: &str,
) -> Result<RestoreArtifactSnapshot, String> {
    let error = match RestoreArtifactSnapshot::capture(id) {
        Ok(current) => return Ok(current),
        Err(error) => error,
    };

    let mut keychain_failures = Vec::new();
    rollback_keychain(id.as_wallet_id(), initial, &mut keychain_failures);
    if !keychain_failures.is_empty() {
        return Err(format!(
            "failed to capture {context} artifacts: {error}; {}",
            keychain_failures.join("; ")
        ));
    }

    RestoreArtifactSnapshot::capture(id).map_err(|retry_error| {
        format!("failed to capture {context} artifacts after keychain rollback: {retry_error}")
    })
}

fn cleanup_owned_marker_artifacts(
    id: &ValidatedRestoreWalletId,
    initial: &PersistedArtifactSnapshot,
    cleanup_complete: bool,
) -> Result<(), String> {
    let storage_gate = crate::database::wallet_data::wallet_data_storage_gate(id.as_wallet_id());
    let _storage_guard = storage_gate.lock();
    let mut initial_snapshot =
        if cleanup_complete { RestoreArtifactSnapshot::default() } else { initial.to_snapshot(id) };

    let current = capture_after_keychain_recovery(id, &initial_snapshot, "restore")?;

    if !cleanup_complete {
        let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
        for path in &current.wallet_data_paths {
            if path == &directory && initial.wallet_data_directory.is_some() {
                initial_snapshot.wallet_data_paths.insert(path.clone());
                continue;
            }
            let Some(relative) = path
                .strip_prefix(&directory)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
            else {
                continue;
            };
            if initial.wallet_data_fingerprints.contains_key(&hash_wallet_data_entry(relative)) {
                initial_snapshot.wallet_data_paths.insert(path.clone());
            }
        }
    }
    let mut failures = Vec::new();

    rollback_keychain(id.as_wallet_id(), &initial_snapshot, &mut failures);
    rollback_bdk_artifacts(
        id.as_wallet_id(),
        &initial_snapshot,
        id.as_wallet_id().as_str(),
        &mut failures,
    );
    rollback_wallet_data_while_gated(
        id.as_wallet_id(),
        &initial_snapshot,
        id.as_wallet_id().as_str(),
        &mut failures,
    );

    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

fn interrupted_cleanup_paths(
    id: &ValidatedRestoreWalletId,
    expected: &RestoreArtifactSnapshot,
    current: &RestoreArtifactSnapshot,
) -> Result<Vec<PathBuf>, String> {
    let mut paths_to_remove = Vec::new();

    if current.keychain_items && !expected.keychain_items {
        return Err("new keychain entries appeared while cleanup was interrupted".to_string());
    }
    for kind in current.keychain_entries.keys() {
        if !expected.keychain_entries.contains_key(kind) {
            return Err("keychain entries changed while cleanup was interrupted".to_string());
        }
    }

    let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id.as_wallet_id());
    for (artifact, fingerprint) in &current.bdk_fingerprints {
        if expected.bdk_fingerprints.get(artifact) != Some(fingerprint) {
            return Err("BDK artifacts changed while cleanup was interrupted".to_string());
        }
        paths_to_remove.push(artifact.path(&bdk_paths));
    }

    let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
    for path in &current.wallet_data_paths {
        if path == &directory {
            continue;
        }
        let Some(relative) = path.strip_prefix(&directory).ok() else {
            return Err("wallet-data path escaped its root".to_string());
        };
        let key = hash_wallet_data_entry(relative);
        let Some(expected_fingerprint) = expected.wallet_data_fingerprints.get(&key) else {
            return Err(
                "new wallet-data artifact appeared while cleanup was interrupted".to_string()
            );
        };
        let Some(current_fingerprint) = current.wallet_data_fingerprints.get(&key) else {
            return Err("wallet-data artifact fingerprint is missing".to_string());
        };
        if current_fingerprint != expected_fingerprint {
            return Err("wallet-data artifact changed while cleanup was interrupted".to_string());
        }
        paths_to_remove.push(path.clone());
    }

    if current.wallet_data_directory.is_some() && expected.wallet_data_directory.is_none() {
        return Err("new wallet-data root appeared while cleanup was interrupted".to_string());
    }

    if let Some(expected_directory) = &expected.wallet_data_directory
        && let Some(current_directory) = &current.wallet_data_directory
        && (current_directory.kind != expected_directory.kind
            || current_directory.identity != expected_directory.identity)
    {
        return Err("wallet-data root changed while cleanup was interrupted".to_string());
    }
    if expected.wallet_data_directory.is_some() && current.wallet_data_directory.is_some() {
        paths_to_remove.push(directory);
    }

    Ok(paths_to_remove)
}

fn remove_interrupted_cleanup_paths(
    id: &ValidatedRestoreWalletId,
    mut paths_to_remove: Vec<PathBuf>,
) -> Result<(), String> {
    let bdk_artifacts = bdk_artifacts(id.as_wallet_id());
    paths_to_remove.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    for path in paths_to_remove {
        let result = if path.starts_with(&*cove_common::consts::ROOT_DATA_DIR) {
            if bdk_artifacts.iter().any(|(_, candidate)| candidate == &path) {
                crate::bdk_store::BdkStore::remove_wallet_artifact(&path)
            } else {
                crate::database::wallet_data::remove_wallet_artifact(&path)
            }
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "artifact path outside data root"))
        };
        if let Err(error) = result {
            return Err(format!("failed to resume cleanup for {}: {error}", path.display()));
        }
    }

    Ok(())
}

fn resume_interrupted_cleanup(
    id: &ValidatedRestoreWalletId,
    initial: &PersistedArtifactSnapshot,
) -> Result<(), String> {
    let storage_gate = crate::database::wallet_data::wallet_data_storage_gate(id.as_wallet_id());
    let _storage_guard = storage_gate.lock();
    crate::database::wallet_data::evict_wallet_data_connections(id.as_wallet_id());
    let expected = initial.to_snapshot(id);
    let current = capture_after_keychain_recovery(id, &expected, "interrupted cleanup")?;

    let paths_to_remove = interrupted_cleanup_paths(id, &expected, &current)?;

    if current.keychain_items {
        delete_keychain_items_exact(id.as_wallet_id())?;
    }

    remove_interrupted_cleanup_paths(id, paths_to_remove)
}

enum MarkerRecoveryError {
    Invalid(String),
    Retryable(String),
}

impl std::fmt::Display for MarkerRecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(error) | Self::Retryable(error) => formatter.write_str(error),
        }
    }
}

fn recover_marker(path: &Path) -> Result<(), MarkerRecoveryError> {
    let marker = match read_marker(path) {
        Ok(marker) => marker,
        Err(reason) => match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            _ => return Err(MarkerRecoveryError::Invalid(reason)),
        },
    };
    if marker.version != RESTORE_MARKER_VERSION {
        return Err(MarkerRecoveryError::Invalid(format!(
            "unsupported restore marker version {}",
            marker.version
        )));
    }

    let id = WalletId::from(marker.wallet_id.clone());
    let validated = ValidatedRestoreWalletId::validate(&id)
        .map_err(|error| MarkerRecoveryError::Invalid(error.to_string()))?;

    let expected_path = marker_path(&marker.operation_id)
        .map_err(|error| MarkerRecoveryError::Invalid(error.to_string()))?;

    if expected_path != path {
        return Err(MarkerRecoveryError::Invalid(
            "restore marker operation id does not match its filename".to_string(),
        ));
    }

    if RestoreFileLock::is_held(&validated) {
        info!(wallet_id = %id, "deferring restore marker recovery for an active restore lease");
        return Ok(());
    }

    let _lock = RestoreFileLock::acquire(&validated)
        .map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;

    if metadata_exists(&validated)
        .map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?
    {
        remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
        info!(wallet_id = %id, "removed committed restore marker during bootstrap recovery");
        return Ok(());
    }

    if marker.phase == RestoreMarkerPhase::CleanupInProgress {
        resume_interrupted_cleanup(&validated, &marker.initial)
            .map_err(MarkerRecoveryError::Retryable)?;
        remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
        info!(wallet_id = %id, "resumed interrupted restore cleanup during bootstrap recovery");
        return Ok(());
    }

    cleanup_owned_marker_artifacts(
        &validated,
        &marker.initial,
        marker.phase == RestoreMarkerPhase::CleanupComplete,
    )
    .map_err(MarkerRecoveryError::Retryable)?;
    remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
    info!(wallet_id = %id, "recovered interrupted wallet restore");
    Ok(())
}

fn quarantine_marker_after_recovery_failure(
    path: &Path,
    reason: &str,
    message: &str,
    failures: &mut Vec<String>,
) {
    warn!(path = %path.display(), %reason, "{message}");
    if let Err(quarantine_error) = quarantine_marker(path) {
        failures
            .push(format!("{}: {reason}; quarantine failed: {quarantine_error}", path.display()));
    }
}

fn handle_marker_recovery_error(
    path: &Path,
    error: MarkerRecoveryError,
    failures: &mut Vec<String>,
) {
    match error {
        MarkerRecoveryError::Invalid(reason) => quarantine_marker_after_recovery_failure(
            path,
            &reason,
            "Quarantining malformed restore marker",
            failures,
        ),
        MarkerRecoveryError::Retryable(reason) => {
            warn!(path = %path.display(), %reason, "restore marker requires recovery");
            failures.push(format!("{}: {reason}", path.display()));
        }
    }
}

fn recover_temporary_marker_entry(path: &Path, failures: &mut Vec<String>) {
    if let Err(error) = recover_temporary_marker(path) {
        quarantine_marker_after_recovery_failure(
            path,
            &error,
            "Quarantining invalid restore marker temp",
            failures,
        );
    }
}

fn recover_marker_directory_entry(
    entry: Result<fs::DirEntry, io::Error>,
    failures: &mut Vec<String>,
) {
    let entry = match entry {
        Ok(entry) => entry,
        Err(error) => {
            failures.push(format!("failed to read restore marker entry: {error}"));
            return;
        }
    };
    let path = entry.path();
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if file_name.starts_with('.') && file_name.ends_with(".tmp") {
        recover_temporary_marker_entry(&path, failures);
        return;
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some(MARKER_EXTENSION) {
        return;
    }

    if let Err(error) = recover_marker(&path) {
        handle_marker_recovery_error(&path, error, failures);
    }
}

/// Recover all durable restore markers after the main database is readable
///
/// A marker is removed automatically only when its metadata commit record is
/// present or every artifact owned by the marker can be cleaned. Any other
/// state remains on disk and blocks bootstrap with a typed recovery error
pub(crate) fn recover_restore_markers() -> Result<(), String> {
    let directory = marker_directory();
    if let Ok(metadata) = fs::symlink_metadata(&directory)
        && metadata.file_type().is_symlink()
    {
        return Err("restore marker directory is a symbolic link".to_string());
    }

    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to read restore marker directory: {error}")),
    };

    let mut failures = Vec::new();
    for entry in entries {
        recover_marker_directory_entry(entry, &mut failures);
    }

    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}
