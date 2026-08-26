use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsStr,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read as _},
    os::fd::AsRawFd as _,
    os::unix::ffi::OsStrExt as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use cove_device::keychain::Keychain;
use cove_types::WalletId;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tracing::error;
use zeroize::Zeroizing;

use crate::{database::Database, wallet::metadata::WalletMetadata};

use super::error::BackupError;

mod marker;

#[cfg(test)]
use marker::marker_directory;
use marker::{marker_path, metadata_exists, operation_id, remove_marker, write_marker};
pub(crate) use marker::{recover_restore_markers, remove_all_restore_recovery_state};

pub(crate) const RESTORE_MARKER_VERSION: u32 = 1;
const MAX_WALLET_ID_BYTES: usize = 128;
const MAX_MARKER_BYTES: u64 = 64 * 1024;
const MARKER_DIRECTORY_NAME: &str = "restore-markers";
const LOCK_DIRECTORY_NAME: &str = "restore-locks";
const QUARANTINE_DIRECTORY_NAME: &str = "quarantine";
const MARKER_EXTENSION: &str = "json";

/// A validated wallet id that is safe to use in every restore-owned path
///
/// Wallet ids are persisted in backup metadata and are therefore not trusted
/// until this type has been constructed. The wrapper keeps validation adjacent
/// to the lease and marker owners so callers cannot accidentally skip it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedRestoreWalletId(WalletId);

impl ValidatedRestoreWalletId {
    pub(crate) fn validate(id: &WalletId) -> Result<Self, BackupError> {
        let value = id.as_str();

        if value.is_empty() {
            return Err(BackupError::InvalidWalletId("wallet id is empty".to_string()));
        }

        if value.len() > MAX_WALLET_ID_BYTES {
            return Err(BackupError::InvalidWalletId(format!(
                "wallet id is longer than {MAX_WALLET_ID_BYTES} bytes"
            )));
        }

        // production wallet ids use Nanoid's URL-safe ASCII alphabet
        if !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) {
            return Err(BackupError::InvalidWalletId(
                "wallet id contains characters outside the Nanoid alphabet".to_string(),
            ));
        }

        Ok(Self(id.clone()))
    }

    pub(crate) fn as_wallet_id(&self) -> &WalletId {
        &self.0
    }

    pub(crate) fn path_key(&self) -> String {
        restore_path_key(self.0.as_str())
    }
}

/// Return the canonical key used by every restore-owned artifact root
pub(crate) fn restore_path_key(value: &str) -> String {
    value.to_ascii_lowercase()
}

struct RestoreFileLock {
    file: File,
    path: PathBuf,
}

impl RestoreFileLock {
    fn acquire(id: &ValidatedRestoreWalletId) -> Result<Self, BackupError> {
        let directory = lock_directory();
        ensure_directory(&directory, "restore lock")?;
        let path = lock_path(id);
        ensure_lock_path(&path)?;

        let file = open_lock_file(&path)?;
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            return Err(BackupError::WalletIdOccupied(id.as_wallet_id().clone()));
        }

        // the lock is released by unlinking the lock file, so a handle opened
        // before another holder unlinked it guards an inode nobody consults
        if !lock_file_is_current(&file, &path) {
            return Err(BackupError::WalletIdOccupied(id.as_wallet_id().clone()));
        }

        Ok(Self { file, path })
    }

    /// Report whether another restore currently holds the durable lock
    ///
    /// The probe never creates or removes the lock file so that it stays safe
    /// to run against a wallet id no restore has ever touched
    fn is_held(id: &ValidatedRestoreWalletId) -> bool {
        let path = lock_path(id);
        let Ok(file) = open_lock_file_for_probe(&path) else {
            return false;
        };

        let locked = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
        locked != 0
    }
}

impl Drop for RestoreFileLock {
    fn drop(&mut self) {
        if lock_file_is_current(&self.file, &self.path) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn open_lock_file(path: &Path) -> Result<File, BackupError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(LOCK_NOFOLLOW)
        .open(path)
        .map_err(|error| BackupError::Restore(format!("failed to open restore lock: {error}")))
}

fn open_lock_file_for_probe(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).custom_flags(LOCK_NOFOLLOW).open(path)
}

fn lock_file_is_current(file: &File, path: &Path) -> bool {
    let (Ok(open), Ok(current)) = (file.metadata(), fs::symlink_metadata(path)) else {
        return false;
    };

    (open.dev(), open.ino()) == (current.dev(), current.ino())
}

fn ensure_lock_path(path: &Path) -> Result<(), BackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(BackupError::Restore("restore lock path is a symbolic link".to_string()))
        }
        Ok(metadata) if !metadata.is_file() => {
            Err(BackupError::Restore("restore lock path is not a file".to_string()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(BackupError::Restore(format!("failed to inspect restore lock path: {error}")))
        }
    }
}

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
#[cfg(target_os = "linux")]
const LOCK_NOFOLLOW: i32 = 0x20000;
#[cfg(not(target_os = "linux"))]
const LOCK_NOFOLLOW: i32 = 0x100;

unsafe extern "C" {
    fn flock(fd: std::os::fd::RawFd, operation: i32) -> i32;
}

fn lock_directory() -> PathBuf {
    cove_common::consts::ROOT_DATA_DIR.join(LOCK_DIRECTORY_NAME)
}

fn lock_path(id: &ValidatedRestoreWalletId) -> PathBuf {
    let digest = Sha256::digest(id.path_key().as_bytes());
    lock_directory().join(format!("{}.lock", hex::encode(digest)))
}

fn ensure_directory(path: &Path, description: &str) -> Result<(), BackupError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(BackupError::Restore(format!("{description} directory is a symbolic link")));
    }

    fs::create_dir_all(path).map_err(|error| {
        BackupError::Restore(format!("failed to create {description} directory: {error}"))
    })?;

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        BackupError::Restore(format!("failed to inspect {description} directory: {error}"))
    })?;

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BackupError::Restore(format!("{description} path is not a directory")));
    }

    Ok(())
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn value_fingerprint(value: &[u8]) -> ArtifactFingerprint {
    ArtifactFingerprint {
        kind: ArtifactKind::Value,
        identity: "value".to_string(),
        size: value.len() as u64,
        digest: hash_bytes(value),
    }
}

/// Fingerprint a secret, keeping its plaintext in a buffer that is wiped after use
fn secret_fingerprint(prefix: &str, value: impl fmt::Display) -> ArtifactFingerprint {
    let formatted = Zeroizing::new(format!("{prefix}{value}"));
    value_fingerprint(formatted.as_bytes())
}

fn fingerprint_path(path: &Path) -> io::Result<Option<ArtifactFingerprint>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let (kind, digest) = if metadata.file_type().is_symlink() {
        (ArtifactKind::Symlink, hash_os_str(fs::read_link(path)?.as_os_str()))
    } else if metadata.is_file() {
        (ArtifactKind::File, digest_file(path)?)
    } else if metadata.is_dir() {
        (ArtifactKind::Directory, hash_bytes(&[]))
    } else {
        (ArtifactKind::Other, hash_bytes(&[]))
    };

    Ok(Some(ArtifactFingerprint {
        kind,
        identity: metadata_identity(&metadata),
        size: metadata.len(),
        digest,
    }))
}

fn digest_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }

    Ok(hex::encode(digest.finalize()))
}

fn metadata_identity(metadata: &fs::Metadata) -> String {
    format!("unix:{}:{}", metadata.dev(), metadata.ino())
}

fn hash_os_str(value: &OsStr) -> String {
    hash_bytes(value.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KeychainArtifactKind {
    Secret,
    Xpub,
    Descriptors,
    TapSignerBackup,
}

impl KeychainArtifactKind {
    const ALL: [Self; 4] = [Self::Secret, Self::Xpub, Self::Descriptors, Self::TapSignerBackup];

    fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Xpub => "xpub",
            Self::Descriptors => "descriptors",
            Self::TapSignerBackup => "tap-signer-backup",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "secret" => Some(Self::Secret),
            "xpub" => Some(Self::Xpub),
            "descriptors" => Some(Self::Descriptors),
            "tap-signer-backup" => Some(Self::TapSignerBackup),
            _ => None,
        }
    }
}

fn delete_keychain_kind(keychain: &Keychain, id: &WalletId, kind: KeychainArtifactKind) -> bool {
    match kind {
        KeychainArtifactKind::Secret => keychain.delete_wallet_secret(id),
        KeychainArtifactKind::Xpub => keychain.delete_wallet_xpub(id),
        KeychainArtifactKind::Descriptors => keychain.delete_public_descriptor(id),
        KeychainArtifactKind::TapSignerBackup => keychain.delete_tap_signer_backup(id),
    }
}

fn capture_keychain_fingerprints(
    id: &WalletId,
) -> Result<BTreeMap<String, ArtifactFingerprint>, BackupError> {
    let keychain = Keychain::global();
    let mut fingerprints = BTreeMap::new();

    if let Some(secret) = keychain
        .get_wallet_secret(id)
        .map_err(|error| BackupError::Keychain(format!("wallet secret snapshot: {error}")))?
    {
        let fingerprint = match secret {
            cove_device::keychain::WalletSecret::Mnemonic(mnemonic) => {
                secret_fingerprint("mnemonic:", mnemonic)
            }
            cove_device::keychain::WalletSecret::Xpriv(xprv) => {
                secret_fingerprint("xpriv:", xprv.expose())
            }
        };
        fingerprints.insert(KeychainArtifactKind::Secret.as_str().to_string(), fingerprint);
    }

    if let Some(xpub) = keychain
        .get_wallet_xpub(id)
        .map_err(|error| BackupError::Keychain(format!("wallet xpub snapshot: {error}")))?
    {
        fingerprints.insert(
            KeychainArtifactKind::Xpub.as_str().to_string(),
            value_fingerprint(xpub.to_string().as_bytes()),
        );
    }

    if let Some((external, internal)) = keychain
        .get_public_descriptor(id)
        .map_err(|error| BackupError::Keychain(format!("wallet descriptor snapshot: {error}")))?
    {
        let value = format!("{external}\n{internal}");
        fingerprints.insert(
            KeychainArtifactKind::Descriptors.as_str().to_string(),
            value_fingerprint(value.as_bytes()),
        );
    }

    if let Some(backup) = keychain
        .get_tap_signer_backup(id)
        .map_err(|error| BackupError::Keychain(format!("TapSigner backup snapshot: {error}")))?
    {
        fingerprints.insert(
            KeychainArtifactKind::TapSignerBackup.as_str().to_string(),
            value_fingerprint(&backup),
        );
    }

    Ok(fingerprints)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ArtifactKind {
    File,
    Directory,
    Symlink,
    Other,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtifactFingerprint {
    pub(crate) kind: ArtifactKind,
    pub(crate) identity: String,
    pub(crate) size: u64,
    pub(crate) digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RestoreArtifactSnapshot {
    pub(crate) metadata: bool,
    pub(crate) keychain_items: bool,
    /// The keychain kinds present before the restore, keyed by kind name
    ///
    /// The value is `None` when the snapshot was rebuilt from a durable marker:
    /// markers persist no secret-derived digests, so only the kinds survive a
    /// crash and rollback must treat every listed kind as pre-existing
    pub(crate) keychain_entries: BTreeMap<String, Option<ArtifactFingerprint>>,
    pub(crate) bdk_paths: HashSet<PathBuf>,
    pub(crate) bdk_fingerprints: BTreeMap<PersistedBdkArtifact, ArtifactFingerprint>,
    pub(crate) wallet_data_paths: HashSet<PathBuf>,
    pub(crate) wallet_data_fingerprints: BTreeMap<String, ArtifactFingerprint>,
    pub(crate) wallet_data_directory: Option<ArtifactFingerprint>,
    pub(crate) wallet_data_occupied: bool,
}

impl RestoreArtifactSnapshot {
    pub(crate) fn capture(id: &ValidatedRestoreWalletId) -> Result<Self, BackupError> {
        let metadata_present = metadata_exists(id)?;

        for entry in crate::database::wallet_data::wallet_data_root_entries()
            .map_err(|error| BackupError::Restore(format!("wallet-data root: {error}")))?
        {
            let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if restore_path_key(name) == id.path_key() && name != id.as_wallet_id().as_str() {
                return Err(BackupError::InvalidWalletId(format!(
                    "wallet id collides with existing path: {}",
                    id.as_wallet_id()
                )));
            }
        }

        let keychain_entries = capture_keychain_fingerprints(id.as_wallet_id())?
            .into_iter()
            .map(|(kind, fingerprint)| (kind, Some(fingerprint)))
            .collect::<BTreeMap<_, _>>();
        let keychain_items = !keychain_entries.is_empty()
            || Keychain::global().wallet_items_exist(id.as_wallet_id());

        let mut bdk_paths = HashSet::new();
        let mut bdk_fingerprints = BTreeMap::new();
        for (artifact, path) in bdk_artifacts(id.as_wallet_id()) {
            if let Some(fingerprint) = fingerprint_path(&path).map_err(|error| {
                BackupError::Restore(format!("BDK artifact snapshot {}: {error}", path.display()))
            })? {
                bdk_paths.insert(path);
                bdk_fingerprints.insert(artifact, fingerprint);
            }
        }

        let paths =
            crate::database::wallet_data::wallet_data_artifact_paths_checked(id.as_wallet_id())
                .map_err(|error| {
                    BackupError::Restore(format!("wallet-data artifact snapshot: {error}"))
                })?;

        let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
        let mut wallet_data_paths = HashSet::new();
        let mut wallet_data_fingerprints = BTreeMap::new();
        let mut wallet_data_directory = None;
        for path in paths {
            let Some(fingerprint) = fingerprint_path(&path).map_err(|error| {
                BackupError::Restore(format!(
                    "wallet-data artifact snapshot {}: {error}",
                    path.display()
                ))
            })?
            else {
                continue;
            };
            wallet_data_paths.insert(path.clone());
            if path == directory {
                wallet_data_directory = Some(fingerprint);
            } else if let Ok(relative) = path.strip_prefix(&directory)
                && !relative.as_os_str().is_empty()
            {
                wallet_data_fingerprints.insert(hash_wallet_data_entry(relative), fingerprint);
            }
        }

        let wallet_data_occupied = !wallet_data_fingerprints.is_empty()
            || wallet_data_directory
                .as_ref()
                .is_some_and(|fingerprint| fingerprint.kind != ArtifactKind::Directory);

        Ok(Self {
            metadata: metadata_present,
            keychain_items,
            keychain_entries,
            bdk_paths,
            bdk_fingerprints,
            wallet_data_paths,
            wallet_data_fingerprints,
            wallet_data_directory,
            wallet_data_occupied,
        })
    }

    pub(crate) fn is_occupied(&self) -> bool {
        self.metadata
            || self.keychain_items
            || !self.bdk_paths.is_empty()
            || self.wallet_data_occupied
    }

    pub(crate) fn has_markerless_conflict(&self) -> bool {
        !self.metadata
            && (self.keychain_items || !self.bdk_paths.is_empty() || self.wallet_data_occupied)
    }
}

/// A per-wallet lease held from preflight through marker cleanup or rollback
pub(crate) struct WalletRestoreLease {
    id: ValidatedRestoreWalletId,
    snapshot: RestoreArtifactSnapshot,
    _lock: Arc<RestoreFileLock>,
}

impl WalletRestoreLease {
    /// Take the lease for a wallet id that must be free of every local artifact
    pub(crate) fn acquire(metadata: &WalletMetadata) -> Result<Self, BackupError> {
        Self::acquire_with_snapshot(metadata, None)
    }

    /// Take the lease for a wallet id whose exact existing artifacts were approved
    pub(crate) fn acquire_for_approval(
        metadata: &WalletMetadata,
        expected: &RestoreArtifactSnapshot,
    ) -> Result<Self, BackupError> {
        Self::acquire_with_snapshot(metadata, Some(expected))
    }

    fn acquire_with_snapshot(
        metadata: &WalletMetadata,
        expected: Option<&RestoreArtifactSnapshot>,
    ) -> Result<Self, BackupError> {
        let id = ValidatedRestoreWalletId::validate(&metadata.id)?;
        let lock = Arc::new(RestoreFileLock::acquire(&id)?);
        let snapshot = RestoreArtifactSnapshot::capture(&id)?;

        match expected {
            Some(expected) if &snapshot != expected => {
                return Err(BackupError::ImportApprovalStale(metadata.id.clone()));
            }
            Some(_) => {}
            None if snapshot.is_occupied() => {
                return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
            }
            None => {}
        }

        Ok(Self { id, snapshot, _lock: lock })
    }

    pub(crate) fn snapshot(&self) -> &RestoreArtifactSnapshot {
        &self.snapshot
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRestoreMarker {
    version: u32,
    operation_id: String,
    wallet_id: String,
    initial: PersistedArtifactSnapshot,
    phase: RestoreMarkerPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RestoreMarkerPhase {
    Writing,
    CleanupInProgress,
    CleanupComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedArtifactSnapshot {
    metadata: bool,
    keychain_items: bool,
    /// Which keychain kinds existed before the restore
    ///
    /// Only the kind names are persisted: a marker is plaintext on disk, so a
    /// secret-derived digest here would be a seed verification oracle
    keychain_kinds: BTreeSet<String>,
    bdk_fingerprints: BTreeMap<PersistedBdkArtifact, ArtifactFingerprint>,
    wallet_data_directory: Option<ArtifactFingerprint>,
    wallet_data_fingerprints: BTreeMap<String, ArtifactFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum PersistedBdkArtifact {
    FileStore,
    Sqlite,
    Wal,
    Shm,
    Journal,
}

impl PersistedBdkArtifact {
    const ALL: [Self; 5] = [Self::FileStore, Self::Sqlite, Self::Wal, Self::Shm, Self::Journal];

    fn path(self, paths: &[PathBuf; 5]) -> PathBuf {
        match self {
            Self::FileStore => paths[0].clone(),
            Self::Sqlite => paths[1].clone(),
            Self::Wal => paths[2].clone(),
            Self::Shm => paths[3].clone(),
            Self::Journal => paths[4].clone(),
        }
    }
}

impl PersistedArtifactSnapshot {
    fn from_snapshot(snapshot: &RestoreArtifactSnapshot) -> Self {
        Self {
            metadata: snapshot.metadata,
            keychain_items: snapshot.keychain_items,
            keychain_kinds: snapshot.keychain_entries.keys().cloned().collect(),
            bdk_fingerprints: snapshot.bdk_fingerprints.clone(),
            wallet_data_directory: snapshot.wallet_data_directory.clone(),
            wallet_data_fingerprints: snapshot.wallet_data_fingerprints.clone(),
        }
    }

    fn to_snapshot(&self, id: &ValidatedRestoreWalletId) -> RestoreArtifactSnapshot {
        let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id.as_wallet_id());
        let bdk_paths_set =
            self.bdk_fingerprints.keys().map(|artifact| artifact.path(&bdk_paths)).collect();

        let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
        let mut wallet_data_paths = HashSet::new();
        if self.wallet_data_directory.is_some() {
            wallet_data_paths.insert(directory);
        }

        RestoreArtifactSnapshot {
            metadata: self.metadata,
            keychain_items: self.keychain_items || !self.keychain_kinds.is_empty(),
            keychain_entries: self.keychain_kinds.iter().map(|kind| (kind.clone(), None)).collect(),
            bdk_paths: bdk_paths_set,
            bdk_fingerprints: self.bdk_fingerprints.clone(),
            wallet_data_fingerprints: self.wallet_data_fingerprints.clone(),
            wallet_data_directory: self.wallet_data_directory.clone(),
            wallet_data_paths,
            wallet_data_occupied: !self.wallet_data_fingerprints.is_empty()
                || self
                    .wallet_data_directory
                    .as_ref()
                    .is_some_and(|fingerprint| fingerprint.kind != ArtifactKind::Directory),
        }
    }
}

fn bdk_artifacts(id: &WalletId) -> [(PersistedBdkArtifact, PathBuf); 5] {
    let paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id);
    PersistedBdkArtifact::ALL.map(|artifact| (artifact, artifact.path(&paths)))
}

fn hash_wallet_data_entry(relative_path: &Path) -> String {
    hash_os_str(relative_path.as_os_str())
}

/// Durable restore journal. The marker contains no secrets and no absolute paths
pub(crate) struct RestoreMarkerGuard {
    metadata: WalletMetadata,
    lease: Option<WalletRestoreLease>,
    marker: PersistedRestoreMarker,
    marker_path: PathBuf,
    cleanup_attempted: bool,
    cleanup_complete: bool,
    metadata_committed: bool,
    rolled_back: bool,
}

impl RestoreMarkerGuard {
    pub(crate) fn begin(
        metadata: &WalletMetadata,
        lease: WalletRestoreLease,
    ) -> Result<Self, BackupError> {
        let operation_id = operation_id();
        let marker = PersistedRestoreMarker {
            version: RESTORE_MARKER_VERSION,
            operation_id: operation_id.clone(),
            wallet_id: lease.id.as_wallet_id().as_str().to_string(),
            initial: PersistedArtifactSnapshot::from_snapshot(lease.snapshot()),
            phase: RestoreMarkerPhase::Writing,
        };
        let marker_path = marker_path(&operation_id)?;

        write_marker(&marker_path, &marker)?;

        Ok(Self {
            metadata: metadata.clone(),
            lease: Some(lease),
            marker,
            marker_path,
            cleanup_attempted: false,
            cleanup_complete: false,
            metadata_committed: false,
            rolled_back: false,
        })
    }

    fn lease(&self) -> &WalletRestoreLease {
        self.lease.as_ref().expect("restore lease is held until marker completion")
    }

    pub(crate) fn initial_snapshot(&self) -> &RestoreArtifactSnapshot {
        self.lease().snapshot()
    }

    /// Delete markerless artifacts only after an explicit, exact-snapshot approval
    pub(crate) fn remove_approved_conflicts(&mut self) -> Result<(), BackupError> {
        let snapshot = self.initial_snapshot().clone();
        if !snapshot.has_markerless_conflict() {
            return Ok(());
        }

        self.cleanup_attempted = true;
        self.marker.phase = RestoreMarkerPhase::CleanupInProgress;
        write_marker(&self.marker_path, &self.marker)?;
        remove_markerless_artifacts(&self.lease().id, &snapshot)?;
        self.cleanup_complete = true;
        self.marker.phase = RestoreMarkerPhase::CleanupComplete;
        write_marker(&self.marker_path, &self.marker)
    }

    /// Record the restore as committed and drop the marker, reporting any cleanup warning
    pub(crate) fn commit(mut self) -> Vec<String> {
        // metadata is the durable commit record
        // once it exists, never roll it back merely because marker cleanup is unavailable
        self.metadata_committed = true;
        let mut warnings = Vec::new();
        if let Err(error) = remove_marker(&self.marker_path) {
            warnings.push(format!("restore marker cleanup is pending: {error}"));
        }

        self.rolled_back = true;
        self.lease.take();
        warnings
    }

    pub(crate) fn rollback(&mut self) -> Vec<String> {
        if self.metadata_committed || self.rolled_back {
            return Vec::new();
        }

        self.rolled_back = true;
        let mut failures = Vec::new();
        let initial_snapshot = if self.cleanup_complete {
            RestoreArtifactSnapshot::default()
        } else {
            self.initial_snapshot().clone()
        };

        rollback_keychain(&self.metadata.id, &initial_snapshot, &mut failures);
        rollback_bdk_artifacts(
            &self.metadata.id,
            &initial_snapshot,
            &self.metadata.name,
            &mut failures,
        );
        rollback_wallet_data(
            &self.metadata.id,
            &initial_snapshot,
            &self.metadata.name,
            &mut failures,
        );
        rollback_metadata(&self.metadata, &initial_snapshot, &mut failures);

        if failures.is_empty() && (!self.cleanup_attempted || self.cleanup_complete) {
            if let Err(error) = remove_marker(&self.marker_path) {
                failures.push(error.to_string());
            }
        } else if failures.is_empty() {
            failures.push("approved artifact cleanup was incomplete".to_string());
        }

        failures
    }
}

impl Drop for RestoreMarkerGuard {
    fn drop(&mut self) {
        if self.metadata_committed || self.rolled_back {
            return;
        }

        let failures = self.rollback();
        if !failures.is_empty() {
            error!(
                wallet_id = %self.metadata.id,
                failures = ?failures,
                "restore rollback was incomplete; durable restore marker retained"
            );
        }
    }
}

/// Whether a keychain entry still matches what the initial snapshot recorded
///
/// A snapshot rebuilt from a durable marker records only that the kind existed,
/// so any content is accepted for a kind the snapshot lists
fn matches_initial_keychain_entry(
    initial: Option<&Option<ArtifactFingerprint>>,
    current: Option<&ArtifactFingerprint>,
) -> bool {
    match (initial, current) {
        (Some(None), Some(_)) => true,
        (Some(Some(expected)), Some(current)) => expected == current,
        _ => false,
    }
}

fn cleanup_keychain_after_capture_failure(
    keychain: &Keychain,
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
) -> Result<(), String> {
    if !initial.keychain_items {
        return delete_keychain_items_exact(id)
            .map_err(|error| format!("exact cleanup failed ({error})"));
    }

    let cleanup_failures = KeychainArtifactKind::ALL
        .into_iter()
        .filter(|kind| !initial.keychain_entries.contains_key(kind.as_str()))
        .filter(|kind| !delete_keychain_kind(keychain, id, *kind))
        .map(|kind| format!("failed to delete keychain {} during capture recovery", kind.as_str()))
        .collect::<Vec<_>>();

    if cleanup_failures.is_empty() {
        return Ok(());
    }

    Err(format!("selective cleanup failed: {}", cleanup_failures.join("; ")))
}

fn capture_keychain_for_rollback(
    keychain: &Keychain,
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
) -> Result<BTreeMap<String, ArtifactFingerprint>, String> {
    let error = match capture_keychain_fingerprints(id) {
        Ok(entries) => return Ok(entries),
        Err(error) => error,
    };

    cleanup_keychain_after_capture_failure(keychain, id, initial).map_err(|cleanup_error| {
        let entries_remain = keychain.wallet_items_exist(id);

        format!(
            "keychain fingerprint capture failed ({error}); {cleanup_error}; keychain entries remain={entries_remain}"
        )
    })?;

    capture_keychain_fingerprints(id).map_err(|retry_error| {
        let entries_remain = keychain.wallet_items_exist(id);

        format!(
            "keychain fingerprint capture failed ({error}); retry failed ({retry_error}); keychain entries remain={entries_remain}"
        )
    })
}

fn rollback_keychain(id: &WalletId, initial: &RestoreArtifactSnapshot, failures: &mut Vec<String>) {
    let keychain = Keychain::global();

    if initial.keychain_items && initial.keychain_entries.is_empty() {
        failures.push(
            "pre-existing keychain entries have no approved fingerprints; selective cleanup skipped"
                .to_string(),
        );
        return;
    }

    let current = match capture_keychain_for_rollback(keychain, id, initial) {
        Ok(entries) => entries,
        Err(error) => {
            failures.push(error);
            return;
        }
    };

    let mut reported_kinds = BTreeSet::new();

    for (kind, fingerprint) in &current {
        let initial_entry = initial.keychain_entries.get(kind);
        if initial_entry.is_some() {
            if !matches_initial_keychain_entry(initial_entry, Some(fingerprint)) {
                failures.push(format!("pre-existing keychain {kind} changed during restore"));
                reported_kinds.insert(kind.clone());
            }

            continue;
        }

        let deleted = KeychainArtifactKind::from_str(kind)
            .is_some_and(|kind| delete_keychain_kind(keychain, id, kind));
        if !deleted {
            failures.push(format!("failed to delete keychain {kind} added during restore"));
            reported_kinds.insert(kind.clone());
        }
    }

    let remaining = match capture_keychain_fingerprints(id) {
        Ok(entries) => entries,
        Err(error) => {
            failures.push(error.to_string());
            return;
        }
    };

    for (kind, fingerprint) in &remaining {
        if reported_kinds.contains(kind) {
            continue;
        }

        if !matches_initial_keychain_entry(initial.keychain_entries.get(kind), Some(fingerprint)) {
            failures.push(format!("keychain {kind} added during restore remains after rollback"));
            reported_kinds.insert(kind.clone());
        }
    }

    for (kind, expected) in &initial.keychain_entries {
        if reported_kinds.contains(kind) {
            continue;
        }

        if !matches_initial_keychain_entry(Some(expected), remaining.get(kind)) {
            failures
                .push(format!("pre-existing keychain {kind} was not preserved during rollback"));
        }
    }
}

fn delete_keychain_items_exact(id: &WalletId) -> Result<(), String> {
    if !Keychain::global().delete_wallet_items(id) {
        return Err("incomplete keychain deletion".to_string());
    }

    let remaining = capture_keychain_fingerprints(id).map_err(|error| error.to_string())?;
    if !remaining.is_empty() || Keychain::global().wallet_items_exist(id) {
        return Err("keychain entries remain after deletion".to_string());
    }

    Ok(())
}

fn rollback_bdk_artifacts(
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
    wallet_name: &str,
    failures: &mut Vec<String>,
) {
    for (artifact, path) in bdk_artifacts(id) {
        if initial.bdk_paths.contains(&path) {
            continue;
        }

        if fs::symlink_metadata(&path).is_err() {
            continue;
        }

        if let Err(error) = crate::bdk_store::BdkStore::remove_wallet_artifact(&path) {
            failures.push(format!(
                "{wallet_name}: failed to delete BDK artifact {artifact:?} {}: {error}",
                path.display()
            ));
        }
    }
}

fn rollback_wallet_data(
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
    wallet_name: &str,
    failures: &mut Vec<String>,
) {
    let storage_gate = crate::database::wallet_data::wallet_data_storage_gate(id);
    let _storage_guard = storage_gate.lock();
    rollback_wallet_data_while_gated(id, initial, wallet_name, failures);
}

fn rollback_wallet_data_while_gated(
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
    wallet_name: &str,
    failures: &mut Vec<String>,
) {
    crate::database::wallet_data::evict_wallet_data_connections(id);
    let paths = match crate::database::wallet_data::wallet_data_artifact_paths_checked(id) {
        Ok(paths) => paths,
        Err(error) => {
            failures.push(format!("{wallet_name}: failed to enumerate wallet data: {error}"));
            return;
        }
    };
    let mut paths = paths;
    paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    for path in paths {
        if initial.wallet_data_paths.contains(&path) {
            continue;
        }

        if let Err(error) = crate::database::wallet_data::remove_wallet_artifact(&path) {
            failures.push(format!(
                "{wallet_name}: failed to delete wallet data {}: {error}",
                path.display()
            ));
        }
    }
}

fn rollback_metadata(
    metadata: &WalletMetadata,
    initial: &RestoreArtifactSnapshot,
    failures: &mut Vec<String>,
) {
    if initial.metadata {
        return;
    }

    let database = Database::global();
    match database.wallets.get(&metadata.id, metadata.network, metadata.wallet_mode) {
        Ok(Some(current)) if current.id == metadata.id => {
            if let Err(error) = database.wallets.remove_wallet_metadata(
                metadata.network,
                metadata.wallet_mode,
                &metadata.id,
            ) {
                failures.push(format!("{}: failed to delete metadata: {error}", metadata.name));
            }
        }
        Ok(None) => {}
        Ok(Some(_)) => {}
        Err(error) => failures.push(format!("{}: failed to read metadata: {error}", metadata.name)),
    }
}

/// Delete every artifact the approval snapshot covers
///
/// The caller holds the wallet restore lease, which captured and compared this
/// exact snapshot under the same durable lock, so no snapshot recheck is needed
fn remove_markerless_artifacts(
    validated: &ValidatedRestoreWalletId,
    initial: &RestoreArtifactSnapshot,
) -> Result<(), BackupError> {
    let id = validated.as_wallet_id();
    let storage_gate = crate::database::wallet_data::wallet_data_storage_gate(id);
    let _storage_guard = storage_gate.lock();

    let mut failures = Vec::new();

    if initial.keychain_items
        && let Err(error) = delete_keychain_items_exact(id)
    {
        failures.push(error);
    }

    for (_, path) in bdk_artifacts(id) {
        if let Err(error) = crate::bdk_store::BdkStore::remove_wallet_artifact(&path) {
            failures.push(format!("failed to delete BDK artifact {}: {error}", path.display()));
        }
    }

    crate::database::wallet_data::evict_wallet_data_connections(id);
    let mut paths =
        crate::database::wallet_data::wallet_data_artifact_paths_checked(id).map_err(|error| {
            BackupError::Restore(format!("wallet-data cleanup enumeration: {error}"))
        })?;

    paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in paths {
        if let Err(error) = crate::database::wallet_data::remove_wallet_artifact(&path) {
            failures.push(format!("failed to delete wallet data {}: {error}", path.display()));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(BackupError::Restore(format!("approved cleanup failed: {}", failures.join("; "))))
    }
}

#[cfg(test)]
mod test_support {
    use std::sync::Arc;

    use super::{
        RestoreArtifactSnapshot, RestoreFileLock, RestoreMarkerGuard, ValidatedRestoreWalletId,
        WalletMetadata, WalletRestoreLease,
    };

    impl RestoreMarkerGuard {
        pub(crate) fn test_begin_with_snapshot(
            metadata: &WalletMetadata,
            snapshot: RestoreArtifactSnapshot,
        ) -> Self {
            let id = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
            let lock = Arc::new(RestoreFileLock::acquire(&id).unwrap());
            let lease = WalletRestoreLease { id, snapshot, _lock: lock };

            Self::begin(metadata, lease).unwrap()
        }
    }
}

#[cfg(test)]
mod tests;
