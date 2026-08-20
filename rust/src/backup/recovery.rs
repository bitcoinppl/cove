use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsStr,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
    sync::LazyLock,
};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;

use cove_device::keychain::Keychain;
use cove_types::WalletId;
use parking_lot::Mutex;
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use strum::IntoEnumIterator as _;
use tracing::{error, info, warn};

use crate::{
    database::Database,
    wallet::metadata::{WalletMetadata, WalletMode},
};

use super::error::BackupError;

pub(crate) const RESTORE_MARKER_VERSION: u32 = 1;
const MAX_WALLET_ID_BYTES: usize = 128;
const MAX_MARKER_BYTES: u64 = 64 * 1024;
const MARKER_DIRECTORY_NAME: &str = "restore-markers";
const LOCK_DIRECTORY_NAME: &str = "restore-locks";
const QUARANTINE_DIRECTORY_NAME: &str = "quarantine";
const MARKER_EXTENSION: &str = "json";

static ACTIVE_WALLET_RESTORE_LEASES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// A validated wallet id that is safe to use in every restore-owned path
///
/// Wallet ids are persisted in backup metadata and are therefore not trusted
/// until this type has been constructed. The wrapper keeps validation adjacent
/// to the lease and marker owners so callers cannot accidentally skip it.
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

        if is_windows_reserved_name(value) {
            return Err(BackupError::InvalidWalletId(
                "wallet id is reserved by the filesystem".to_string(),
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

fn is_windows_reserved_name(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

struct RestoreFileLock {
    _file: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl RestoreFileLock {
    fn acquire(id: &ValidatedRestoreWalletId) -> Result<Self, BackupError> {
        let directory = lock_directory();
        ensure_directory(&directory, "restore lock")?;
        let path = lock_path(id);
        ensure_lock_path(&path)?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd as _;
            use std::os::unix::fs::OpenOptionsExt as _;

            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .custom_flags(LOCK_NOFOLLOW)
                .open(&path)
                .map_err(|error| {
                    BackupError::Restore(format!("failed to open restore lock: {error}"))
                })?;
            if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
                return Err(BackupError::WalletIdOccupied(id.as_wallet_id().clone()));
            }

            Ok(Self { _file: file })
        }

        #[cfg(not(unix))]
        {
            let file =
                OpenOptions::new().create_new(true).read(true).write(true).open(&path).map_err(
                    |error| {
                        if error.kind() == io::ErrorKind::AlreadyExists {
                            BackupError::WalletIdOccupied(id.as_wallet_id().clone())
                        } else {
                            BackupError::Restore(format!("failed to create restore lock: {error}"))
                        }
                    },
                )?;
            Ok(Self { _file: file, path })
        }
    }
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

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_NB: i32 = 4;
#[cfg(target_os = "linux")]
const LOCK_NOFOLLOW: i32 = 0x20000;
#[cfg(all(unix, not(target_os = "linux")))]
const LOCK_NOFOLLOW: i32 = 0x100;

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: std::os::fd::RawFd, operation: i32) -> i32;
}

#[cfg(not(unix))]
impl Drop for RestoreFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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

struct FingerprintWriter {
    digest: Sha256,
    size: u64,
}

impl FingerprintWriter {
    fn new() -> Self {
        Self { digest: Sha256::new(), size: 0 }
    }

    fn write_fragment(&mut self, value: &str) {
        self.digest.update(value.as_bytes());
        self.size += value.len() as u64;
    }

    fn finish(self) -> ArtifactFingerprint {
        ArtifactFingerprint {
            kind: ArtifactKind::Value,
            identity: "value".to_string(),
            size: self.size,
            digest: hex::encode(self.digest.finalize()),
        }
    }
}

impl fmt::Write for FingerprintWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_fragment(value);
        Ok(())
    }
}

fn display_fingerprint(prefix: &str, value: impl fmt::Display) -> ArtifactFingerprint {
    let mut writer = FingerprintWriter::new();
    writer.write_fragment(prefix);
    fmt::write(&mut writer, format_args!("{value}")).expect("fingerprint writer cannot fail");
    writer.finish()
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
    #[cfg(unix)]
    {
        format!("unix:{}:{}", metadata.dev(), metadata.ino())
    }

    #[cfg(windows)]
    {
        format!(
            "windows:{}:{}:{}",
            metadata.volume_serial_number().unwrap_or_default(),
            metadata.file_index_high(),
            metadata.file_index_low()
        );
    }

    #[cfg(not(any(unix, windows)))]
    {
        String::new()
    }
}

fn hash_os_str(value: &OsStr) -> String {
    #[cfg(unix)]
    {
        hex::encode(Sha256::digest(value.as_bytes()))
    }

    #[cfg(windows)]
    {
        let bytes = value.encode_wide().flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        hex::encode(Sha256::digest(bytes))
    }

    #[cfg(not(any(unix, windows)))]
    hash_bytes(value.to_string_lossy().as_bytes())
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
                display_fingerprint("mnemonic:", mnemonic)
            }
            cove_device::keychain::WalletSecret::Xpriv(xprv) => {
                display_fingerprint("xpriv:", xprv.expose())
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
    pub(crate) metadata_digest: Option<String>,
    pub(crate) keychain_items: bool,
    pub(crate) keychain_fingerprints: BTreeMap<String, ArtifactFingerprint>,
    pub(crate) bdk_paths: HashSet<PathBuf>,
    pub(crate) bdk_fingerprints: BTreeMap<PersistedBdkArtifact, ArtifactFingerprint>,
    pub(crate) wallet_data_paths: HashSet<PathBuf>,
    pub(crate) wallet_data_fingerprints: BTreeMap<String, ArtifactFingerprint>,
    pub(crate) wallet_data_directory: Option<ArtifactFingerprint>,
    pub(crate) wallet_data_occupied: bool,
}

impl RestoreArtifactSnapshot {
    pub(crate) fn capture(id: &ValidatedRestoreWalletId) -> Result<Self, BackupError> {
        let database = Database::global();
        let mut metadata_present = false;
        let mut metadata_digests = BTreeSet::new();

        for network in cove_types::network::Network::iter() {
            for mode in WalletMode::iter() {
                let wallets = database
                    .wallets
                    .get_all(network, mode)
                    .map_err(|error| BackupError::Database(error.to_string()))?;

                for wallet in wallets {
                    if wallet.id == *id.as_wallet_id() {
                        metadata_present = true;
                        let bytes = serde_json::to_vec(&wallet).map_err(|error| {
                            BackupError::Serialization(format!("wallet metadata snapshot: {error}"))
                        })?;
                        metadata_digests.insert(hash_bytes(&bytes));
                    } else if restore_path_key(wallet.id.as_str()) == id.path_key() {
                        return Err(BackupError::InvalidWalletId(format!(
                            "wallet id collides with existing path: {}",
                            id.as_wallet_id()
                        )));
                    }
                }
            }
        }

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

        let keychain_fingerprints = capture_keychain_fingerprints(id.as_wallet_id())?;
        let keychain_items = !keychain_fingerprints.is_empty()
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
            metadata_digest: (!metadata_digests.is_empty()).then(|| {
                hash_bytes(
                    metadata_digests
                        .iter()
                        .flat_map(String::as_bytes)
                        .copied()
                        .collect::<Vec<_>>()
                        .as_slice(),
                )
            }),
            keychain_items,
            bdk_paths,
            bdk_fingerprints,
            wallet_data_paths,
            wallet_data_fingerprints,
            wallet_data_directory,
            wallet_data_occupied,
            keychain_fingerprints,
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
    key: String,
    _lock: Arc<RestoreFileLock>,
}

impl WalletRestoreLease {
    pub(crate) fn acquire(metadata: &WalletMetadata) -> Result<Self, BackupError> {
        Self::acquire_with_snapshot(metadata, None, false)
    }

    pub(crate) fn acquire_for_approval(
        metadata: &WalletMetadata,
        expected: &RestoreArtifactSnapshot,
    ) -> Result<Self, BackupError> {
        Self::acquire_with_snapshot(metadata, Some(expected), true)
    }

    pub(crate) fn acquire_preserving(
        metadata: &WalletMetadata,
        expected: &RestoreArtifactSnapshot,
    ) -> Result<Self, BackupError> {
        Self::acquire_with_snapshot(metadata, Some(expected), true)
    }

    fn acquire_with_snapshot(
        metadata: &WalletMetadata,
        expected: Option<&RestoreArtifactSnapshot>,
        allow_occupied: bool,
    ) -> Result<Self, BackupError> {
        let id = ValidatedRestoreWalletId::validate(&metadata.id)?;
        let key = id.path_key();
        {
            let mut active = ACTIVE_WALLET_RESTORE_LEASES.lock();
            if !active.insert(key.clone()) {
                return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
            }
        }

        let lock = match RestoreFileLock::acquire(&id) {
            Ok(lock) => Arc::new(lock),
            Err(error) => {
                ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&key);
                return Err(error);
            }
        };

        let snapshot = match RestoreArtifactSnapshot::capture(&id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&key);
                return Err(error);
            }
        };

        if let Some(expected) = expected
            && &snapshot != expected
        {
            ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&key);
            return Err(BackupError::ImportApprovalStale(metadata.id.clone()));
        }

        if !allow_occupied && snapshot.is_occupied() {
            ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&key);
            return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
        }

        Ok(Self { id, snapshot, key, _lock: lock })
    }

    pub(crate) fn snapshot(&self) -> &RestoreArtifactSnapshot {
        &self.snapshot
    }
}

impl Drop for WalletRestoreLease {
    fn drop(&mut self) {
        ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&self.key);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRestoreMarker {
    version: u32,
    operation_id: String,
    payload_digest: String,
    wallet_id: String,
    initial: PersistedArtifactSnapshot,
    phase: RestoreMarkerPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum RestoreMarkerPhase {
    Writing,
    CleanupInProgress,
    CleanupComplete,
    MetadataCommitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedArtifactSnapshot {
    metadata: bool,
    metadata_digest: Option<String>,
    keychain_items: bool,
    keychain_fingerprints: BTreeMap<String, ArtifactFingerprint>,
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

impl PersistedArtifactSnapshot {
    fn from_snapshot(id: &ValidatedRestoreWalletId, snapshot: &RestoreArtifactSnapshot) -> Self {
        let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id.as_wallet_id());
        let mut bdk_fingerprints = snapshot.bdk_fingerprints.clone();
        for (index, path) in bdk_paths.iter().enumerate() {
            if !snapshot.bdk_paths.contains(path) {
                continue;
            }
            let artifact = persisted_bdk_artifact(index);
            if let Some(fingerprint) = fingerprint_path(path).ok().flatten() {
                bdk_fingerprints.entry(artifact).or_insert(fingerprint);
            }
        }

        let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
        let mut wallet_data_fingerprints = snapshot.wallet_data_fingerprints.clone();
        let wallet_data_directory = snapshot.wallet_data_directory.clone().or_else(|| {
            snapshot.wallet_data_paths.contains(&directory).then(|| {
                fingerprint_path(&directory).ok().flatten().unwrap_or_else(|| ArtifactFingerprint {
                    kind: ArtifactKind::Directory,
                    identity: "missing-directory".to_string(),
                    size: 0,
                    digest: hash_bytes(&[]),
                })
            })
        });
        for path in &snapshot.wallet_data_paths {
            let Some(relative) = path
                .strip_prefix(&directory)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
            else {
                continue;
            };
            if let Some(fingerprint) = fingerprint_path(path).ok().flatten() {
                wallet_data_fingerprints
                    .entry(hash_wallet_data_entry(relative))
                    .or_insert(fingerprint);
            }
        }

        Self {
            metadata: snapshot.metadata,
            metadata_digest: snapshot.metadata_digest.clone(),
            keychain_items: snapshot.keychain_items,
            keychain_fingerprints: snapshot.keychain_fingerprints.clone(),
            bdk_fingerprints,
            wallet_data_directory,
            wallet_data_fingerprints,
        }
    }

    fn to_snapshot(&self, id: &ValidatedRestoreWalletId) -> RestoreArtifactSnapshot {
        let bdk_paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id.as_wallet_id());
        let mut bdk_paths_set = HashSet::new();
        for artifact in self.bdk_fingerprints.keys() {
            bdk_paths_set.insert(bdk_path_for_artifact(&bdk_paths, *artifact));
        }

        let directory = crate::database::wallet_data::wallet_data_directory_path(id.as_wallet_id());
        let mut wallet_data_paths = HashSet::new();
        if self.wallet_data_directory.is_some() {
            wallet_data_paths.insert(directory);
        }

        RestoreArtifactSnapshot {
            metadata: self.metadata,
            metadata_digest: self.metadata_digest.clone(),
            keychain_items: self.keychain_items || !self.keychain_fingerprints.is_empty(),
            keychain_fingerprints: self.keychain_fingerprints.clone(),
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

fn persisted_bdk_artifact(index: usize) -> PersistedBdkArtifact {
    match index {
        0 => PersistedBdkArtifact::FileStore,
        1 => PersistedBdkArtifact::Sqlite,
        2 => PersistedBdkArtifact::Wal,
        3 => PersistedBdkArtifact::Shm,
        _ => PersistedBdkArtifact::Journal,
    }
}

fn bdk_artifacts(id: &WalletId) -> [(PersistedBdkArtifact, PathBuf); 5] {
    let paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(id);
    [
        (PersistedBdkArtifact::FileStore, paths[0].clone()),
        (PersistedBdkArtifact::Sqlite, paths[1].clone()),
        (PersistedBdkArtifact::Wal, paths[2].clone()),
        (PersistedBdkArtifact::Shm, paths[3].clone()),
        (PersistedBdkArtifact::Journal, paths[4].clone()),
    ]
}

fn bdk_path_for_artifact(paths: &[PathBuf; 5], artifact: PersistedBdkArtifact) -> PathBuf {
    match artifact {
        PersistedBdkArtifact::FileStore => paths[0].clone(),
        PersistedBdkArtifact::Sqlite => paths[1].clone(),
        PersistedBdkArtifact::Wal => paths[2].clone(),
        PersistedBdkArtifact::Shm => paths[3].clone(),
        PersistedBdkArtifact::Journal => paths[4].clone(),
    }
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
        payload_digest: &str,
        lease: WalletRestoreLease,
    ) -> Result<Self, BackupError> {
        let operation_id = operation_id();
        let marker = PersistedRestoreMarker {
            version: RESTORE_MARKER_VERSION,
            operation_id: operation_id.clone(),
            payload_digest: payload_digest.to_string(),
            wallet_id: lease.id.as_wallet_id().as_str().to_string(),
            initial: PersistedArtifactSnapshot::from_snapshot(&lease.id, lease.snapshot()),
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

    pub(crate) fn initial_snapshot(&self) -> &RestoreArtifactSnapshot {
        self.lease.as_ref().expect("restore lease is held until marker completion").snapshot()
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
        remove_markerless_artifacts(&self.metadata.id, &snapshot)?;
        self.cleanup_complete = true;
        self.marker.phase = RestoreMarkerPhase::CleanupComplete;
        write_marker(&self.marker_path, &self.marker)
    }

    pub(crate) fn commit(mut self) -> Result<(), BackupError> {
        // metadata is the durable commit record
        // once it exists, never roll it back merely because marker cleanup is unavailable
        self.metadata_committed = true;
        self.marker.phase = RestoreMarkerPhase::MetadataCommitted;
        write_marker(&self.marker_path, &self.marker)?;

        match remove_marker(&self.marker_path) {
            Ok(()) => {
                self.rolled_back = true;
                self.lease.take();
                Ok(())
            }
            Err(error) => Err(error),
        }
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

    #[cfg(test)]
    pub(crate) fn test_begin_with_snapshot(
        metadata: &WalletMetadata,
        snapshot: RestoreArtifactSnapshot,
    ) -> Self {
        let id = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let key = id.path_key();
        ACTIVE_WALLET_RESTORE_LEASES.lock().insert(key.clone());
        let lock = Arc::new(RestoreFileLock::acquire(&id).unwrap());
        let lease = WalletRestoreLease { id, snapshot, key, _lock: lock };

        Self::begin(metadata, "test", lease).unwrap()
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

fn rollback_keychain(id: &WalletId, initial: &RestoreArtifactSnapshot, failures: &mut Vec<String>) {
    let keychain = Keychain::global();

    if initial.keychain_items && initial.keychain_fingerprints.is_empty() {
        failures.push(
            "pre-existing keychain entries have no approved fingerprints; selective cleanup skipped"
                .to_string(),
        );
        return;
    }

    let current = match capture_keychain_fingerprints(id) {
        Ok(entries) => entries,
        Err(error) => {
            if !initial.keychain_items {
                if let Err(cleanup_error) = delete_keychain_items_exact(id) {
                    let entries_remain = keychain.wallet_items_exist(id);
                    failures.push(format!(
                        "keychain fingerprint capture failed ({error}); exact cleanup failed ({cleanup_error}); keychain entries remain={entries_remain}"
                    ));
                    return;
                }
            } else if initial.keychain_fingerprints.is_empty() {
                let entries_remain = keychain.wallet_items_exist(id);
                failures.push(format!(
                    "keychain fingerprint capture failed with pre-existing entries and no approved fingerprints; selective cleanup skipped: {error}; keychain entries remain={entries_remain}"
                ));
                return;
            } else {
                let mut cleanup_failures = Vec::new();
                for kind in KeychainArtifactKind::ALL {
                    if initial.keychain_fingerprints.contains_key(kind.as_str()) {
                        continue;
                    }

                    if !delete_keychain_kind(keychain, id, kind) {
                        cleanup_failures.push(format!(
                            "failed to delete keychain {} during capture recovery",
                            kind.as_str()
                        ));
                    }
                }

                if !cleanup_failures.is_empty() {
                    let entries_remain = keychain.wallet_items_exist(id);
                    failures.push(format!(
                        "keychain fingerprint capture failed ({error}); selective cleanup failed: {}; keychain entries remain={entries_remain}",
                        cleanup_failures.join("; ")
                    ));
                    return;
                }
            }

            match capture_keychain_fingerprints(id) {
                Ok(entries) => entries,
                Err(retry_error) => {
                    let entries_remain = keychain.wallet_items_exist(id);
                    failures.push(format!(
                        "keychain fingerprint capture failed ({error}); retry failed ({retry_error}); keychain entries remain={entries_remain}"
                    ));
                    return;
                }
            }
        }
    };

    let mut reported_kinds = BTreeSet::new();

    for (kind, fingerprint) in &current {
        match initial.keychain_fingerprints.get(kind) {
            Some(expected) if expected == fingerprint => continue,
            Some(_) => {
                failures.push(format!("pre-existing keychain {kind} changed during restore"));
                reported_kinds.insert(kind.clone());
                continue;
            }
            None => {}
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

        if initial.keychain_fingerprints.get(kind) != Some(fingerprint) {
            failures.push(format!("keychain {kind} added during restore remains after rollback"));
            reported_kinds.insert(kind.clone());
        }
    }
    for (kind, fingerprint) in &initial.keychain_fingerprints {
        if reported_kinds.contains(kind) {
            continue;
        }

        if remaining.get(kind) != Some(fingerprint) {
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

fn remove_markerless_artifacts(
    id: &WalletId,
    initial: &RestoreArtifactSnapshot,
) -> Result<(), BackupError> {
    let validated = ValidatedRestoreWalletId::validate(id)?;
    let current = RestoreArtifactSnapshot::capture(&validated)?;
    if &current != initial {
        return Err(BackupError::ImportApprovalStale(id.clone()));
    }

    let mut failures = Vec::new();

    if current.keychain_items
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

fn operation_id() -> String {
    let bytes = rand::rng().random::<[u8; 16]>();
    hex::encode(bytes)
}

fn marker_directory() -> PathBuf {
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

fn marker_path(operation_id: &str) -> Result<PathBuf, BackupError> {
    if operation_id.is_empty()
        || operation_id.len() > 64
        || !operation_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(BackupError::Restore("invalid restore operation id".to_string()));
    }

    Ok(marker_directory().join(format!("{operation_id}.{MARKER_EXTENSION}")))
}

fn write_marker(path: &Path, marker: &PersistedRestoreMarker) -> Result<(), BackupError> {
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

fn remove_marker(path: &Path) -> Result<(), BackupError> {
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
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()
    }

    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

fn read_marker(path: &Path) -> Result<PersistedRestoreMarker, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("restore marker is a symbolic link".to_string());
    }

    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;

        OpenOptions::new()
            .read(true)
            .custom_flags(LOCK_NOFOLLOW)
            .open(path)
            .map_err(|error| error.to_string())?
    };
    #[cfg(not(unix))]
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut limited = file.take(MAX_MARKER_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err("restore marker is too large".to_string());
    }

    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn metadata_exists(id: &WalletId) -> Result<bool, String> {
    let database = Database::global();
    let mut exact_match = false;

    for network in cove_types::network::Network::iter() {
        for mode in WalletMode::iter() {
            let wallets =
                database.wallets.get_all(network, mode).map_err(|error| error.to_string())?;
            for wallet in wallets {
                if wallet.id == *id {
                    exact_match = true;
                } else if restore_path_key(wallet.id.as_str()) == restore_path_key(id.as_str()) {
                    return Err(format!("wallet id collides with existing path: {id}"));
                }
            }
        }
    }

    Ok(exact_match)
}

fn restore_lease_is_active(id: &ValidatedRestoreWalletId) -> bool {
    ACTIVE_WALLET_RESTORE_LEASES.lock().contains(&id.path_key())
}

fn cleanup_owned_marker_artifacts(
    id: &ValidatedRestoreWalletId,
    initial: &PersistedArtifactSnapshot,
    cleanup_complete: bool,
) -> Result<(), String> {
    let mut initial_snapshot =
        if cleanup_complete { RestoreArtifactSnapshot::default() } else { initial.to_snapshot(id) };

    let current = match RestoreArtifactSnapshot::capture(id) {
        Ok(current) => current,
        Err(error) => {
            let mut keychain_failures = Vec::new();
            rollback_keychain(id.as_wallet_id(), &initial_snapshot, &mut keychain_failures);
            if !keychain_failures.is_empty() {
                return Err(format!(
                    "failed to capture restore artifacts: {error}; {}",
                    keychain_failures.join("; ")
                ));
            }

            RestoreArtifactSnapshot::capture(id).map_err(|retry_error| {
                format!(
                    "failed to capture restore artifacts after keychain rollback: {retry_error}"
                )
            })?
        }
    };

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
    rollback_wallet_data(
        id.as_wallet_id(),
        &initial_snapshot,
        id.as_wallet_id().as_str(),
        &mut failures,
    );

    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

fn resume_interrupted_cleanup(
    id: &ValidatedRestoreWalletId,
    initial: &PersistedArtifactSnapshot,
) -> Result<(), String> {
    let expected = initial.to_snapshot(id);
    let current = match RestoreArtifactSnapshot::capture(id) {
        Ok(current) => current,
        Err(error) => {
            let mut keychain_failures = Vec::new();
            rollback_keychain(id.as_wallet_id(), &expected, &mut keychain_failures);
            if !keychain_failures.is_empty() {
                return Err(format!(
                    "failed to capture interrupted cleanup artifacts: {error}; {}",
                    keychain_failures.join("; ")
                ));
            }

            RestoreArtifactSnapshot::capture(id).map_err(|retry_error| {
                format!(
                    "failed to capture interrupted cleanup artifacts after keychain rollback: {retry_error}"
                )
            })?
        }
    };

    let mut paths_to_remove = Vec::new();

    if current.keychain_items && !expected.keychain_items {
        return Err("new keychain entries appeared while cleanup was interrupted".to_string());
    }
    for (kind, fingerprint) in &current.keychain_fingerprints {
        if expected.keychain_fingerprints.get(kind) != Some(fingerprint) {
            return Err("keychain entries changed while cleanup was interrupted".to_string());
        }
    }

    for (artifact, fingerprint) in &current.bdk_fingerprints {
        if expected.bdk_fingerprints.get(artifact) != Some(fingerprint) {
            return Err("BDK artifacts changed while cleanup was interrupted".to_string());
        }
        paths_to_remove.push(bdk_path_for_artifact(
            &crate::bdk_store::BdkStore::wallet_store_artifact_paths(id.as_wallet_id()),
            *artifact,
        ));
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

    if current.keychain_items {
        delete_keychain_items_exact(id.as_wallet_id())?;
    }

    paths_to_remove.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in paths_to_remove {
        let result = if path.starts_with(&*cove_common::consts::ROOT_DATA_DIR) {
            if bdk_artifacts(id.as_wallet_id()).iter().any(|(_, candidate)| candidate == &path) {
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

    if restore_lease_is_active(&validated) {
        info!(wallet_id = %id, "Deferring restore marker recovery for an active restore lease");
        return Ok(());
    }

    let _lock = RestoreFileLock::acquire(&validated)
        .map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;

    if metadata_exists(&id).map_err(MarkerRecoveryError::Retryable)? {
        remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
        info!(wallet_id = %id, "Removed committed restore marker during bootstrap recovery");
        return Ok(());
    }

    if matches!(marker.phase, RestoreMarkerPhase::CleanupInProgress) {
        crate::database::wallet_data::evict_wallet_data_connections(&id);
        resume_interrupted_cleanup(&validated, &marker.initial)
            .map_err(MarkerRecoveryError::Retryable)?;
        remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
        info!(wallet_id = %id, "Resumed interrupted restore cleanup during bootstrap recovery");
        return Ok(());
    }

    cleanup_owned_marker_artifacts(
        &validated,
        &marker.initial,
        matches!(
            marker.phase,
            RestoreMarkerPhase::CleanupComplete | RestoreMarkerPhase::MetadataCommitted
        ),
    )
    .map_err(MarkerRecoveryError::Retryable)?;
    remove_marker(path).map_err(|error| MarkerRecoveryError::Retryable(error.to_string()))?;
    info!(wallet_id = %id, "Recovered interrupted wallet restore");
    Ok(())
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
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!("failed to read restore marker entry: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        if file_name.starts_with('.') && file_name.ends_with(".tmp") {
            if let Err(error) = recover_temporary_marker(&path) {
                warn!(path = %path.display(), %error, "Quarantining invalid restore marker temp");
                if let Err(quarantine_error) = quarantine_marker(&path) {
                    failures.push(format!(
                        "{}: {error}; quarantine failed: {quarantine_error}",
                        path.display()
                    ));
                }
            }
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some(MARKER_EXTENSION) {
            continue;
        }

        if let Err(error) = recover_marker(&path) {
            match error {
                MarkerRecoveryError::Invalid(reason) => {
                    warn!(path = %path.display(), %reason, "Quarantining malformed restore marker");
                    if let Err(quarantine_error) = quarantine_marker(&path) {
                        failures.push(format!(
                            "{}: {reason}; quarantine failed: {quarantine_error}",
                            path.display()
                        ));
                    }
                }
                MarkerRecoveryError::Retryable(reason) => {
                    warn!(path = %path.display(), %reason, "Restore marker requires recovery");
                    failures.push(format!("{}: {reason}", path.display()));
                }
            }
        }
    }

    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::str::FromStr as _;

    use super::*;

    fn id(value: &str) -> WalletId {
        WalletId::from(value.to_string())
    }

    fn write_test_marker(
        wallet_id: &WalletId,
        snapshot: &RestoreArtifactSnapshot,
        phase: RestoreMarkerPhase,
    ) -> PathBuf {
        let validated = ValidatedRestoreWalletId::validate(wallet_id).unwrap();
        let operation_id = operation_id();
        let path = marker_path(&operation_id).unwrap();
        let marker = PersistedRestoreMarker {
            version: RESTORE_MARKER_VERSION,
            operation_id,
            payload_digest: "test-digest".to_string(),
            wallet_id: wallet_id.as_str().to_string(),
            initial: PersistedArtifactSnapshot::from_snapshot(&validated, snapshot),
            phase,
        };
        write_marker(&path, &marker).unwrap();
        path
    }

    #[test]
    fn restore_ids_reject_path_and_control_input() {
        for value in ["", ".", "..", "a/b", "a\\b", "a\0b", "a\n"] {
            assert!(ValidatedRestoreWalletId::validate(&id(value)).is_err(), "{value:?}");
        }
    }

    #[test]
    fn restore_ids_reject_unsafe_length_and_suffixes() {
        assert!(
            ValidatedRestoreWalletId::validate(&id(&"a".repeat(MAX_WALLET_ID_BYTES + 1))).is_err()
        );
        assert!(ValidatedRestoreWalletId::validate(&id("wallet.")).is_err());
        assert!(ValidatedRestoreWalletId::validate(&id("wallet ")).is_err());
        assert!(ValidatedRestoreWalletId::validate(&id("CON")).is_err());
        assert!(ValidatedRestoreWalletId::validate(&id("wallet:name")).is_err());
        assert!(ValidatedRestoreWalletId::validate(&id("wallet")).is_ok());
    }

    #[test]
    fn restore_ids_reject_non_nanoid_unicode() {
        for value in ["walleté", "wallete\u{301}", "钱包"] {
            assert!(ValidatedRestoreWalletId::validate(&id(value)).is_err(), "{value:?}");
        }

        assert!(ValidatedRestoreWalletId::validate(&id("Wallet_01-test")).is_ok());
    }

    #[test]
    fn persisted_snapshot_does_not_store_wallet_data_filename() {
        let wallet_id = ValidatedRestoreWalletId::validate(&id("wallet")).unwrap();
        let directory =
            crate::database::wallet_data::wallet_data_directory_path(wallet_id.as_wallet_id());
        let secret_name = directory.join("secret-labels.redb");
        let snapshot = RestoreArtifactSnapshot {
            wallet_data_paths: HashSet::from([secret_name]),
            ..RestoreArtifactSnapshot::default()
        };

        let marker = PersistedArtifactSnapshot::from_snapshot(&wallet_id, &snapshot);
        let json = serde_json::to_string(&marker).unwrap();

        assert!(!json.contains("secret-labels.redb"));
        assert!(json.contains("wallet_data_fingerprints"));
    }

    #[test]
    fn secret_fingerprint_matches_formatted_value_without_combining_secret_parts() {
        let mnemonic = bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();

        assert_eq!(
            display_fingerprint("mnemonic:", &mnemonic),
            value_fingerprint(format!("mnemonic:{mnemonic}").as_bytes())
        );
    }

    #[test]
    fn marker_round_trip_uses_the_atomic_file_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("operation.json");
        let marker = PersistedRestoreMarker {
            version: RESTORE_MARKER_VERSION,
            operation_id: "0123456789abcdef".to_string(),
            payload_digest: "payload-digest".to_string(),
            wallet_id: "wallet".to_string(),
            initial: PersistedArtifactSnapshot {
                metadata: false,
                metadata_digest: None,
                keychain_items: false,
                keychain_fingerprints: BTreeMap::new(),
                bdk_fingerprints: BTreeMap::new(),
                wallet_data_directory: None,
                wallet_data_fingerprints: BTreeMap::new(),
            },
            phase: RestoreMarkerPhase::Writing,
        };

        write_marker(&path, &marker).unwrap();
        assert_eq!(read_marker(&path).unwrap().operation_id, marker.operation_id);
        remove_marker(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn marker_drop_rolls_back_new_artifacts_during_panic_unwind() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.name = "panic restore".to_string();
        let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
            .into_iter()
            .find(|path| path.to_string_lossy().ends_with("-wal"))
            .unwrap();
        if let Some(parent) = artifact.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let _ = fs::remove_file(&artifact);

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _journal = RestoreMarkerGuard::test_begin_with_snapshot(
                &metadata,
                RestoreArtifactSnapshot { metadata: true, ..RestoreArtifactSnapshot::default() },
            );
            fs::write(&artifact, b"created during restore").unwrap();
            panic!("simulate restore panic");
        }));

        assert!(result.is_err());
        assert!(!artifact.exists());
    }

    #[test]
    fn approved_cleanup_marks_keychain_artifacts_as_owned() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::app::reconcile::test_support::init_noop_updater();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let xpub = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let snapshot = RestoreArtifactSnapshot::capture(&validated).unwrap();

        let mut journal = RestoreMarkerGuard::test_begin_with_snapshot(&metadata, snapshot);
        journal.remove_approved_conflicts().unwrap();
        assert!(!Keychain::global().wallet_items_exist(&metadata.id));

        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        assert!(journal.rollback().is_empty());
        assert!(!Keychain::global().wallet_items_exist(&metadata.id));
    }

    #[test]
    fn rollback_preserves_existing_xpub_and_removes_added_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::app::reconcile::test_support::init_noop_updater();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let xpub = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let snapshot = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let mut journal = RestoreMarkerGuard::test_begin_with_snapshot(&metadata, snapshot);
        let mnemonic = bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        Keychain::global().save_wallet_key(&metadata.id, mnemonic).unwrap();

        assert!(journal.rollback().is_empty());
        assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));
        assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());

        assert!(Keychain::global().delete_wallet_xpub(&metadata.id));
    }

    #[test]
    fn approval_lease_rejects_changed_artifact_snapshot() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::app::reconcile::test_support::init_noop_updater();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let expected = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let artifact =
            crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
        fs::write(&artifact, b"changed after approval").unwrap();

        assert!(matches!(
            WalletRestoreLease::acquire_for_approval(&metadata, &expected),
            Err(BackupError::ImportApprovalStale(id)) if id == metadata.id
        ));
        fs::remove_file(artifact).unwrap();
    }

    #[test]
    fn recovery_preserves_preexisting_artifacts_and_is_repeatable() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id);
        let preexisting = paths[0].clone();
        let created = paths[1].clone();
        fs::write(&preexisting, b"pre-existing").unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        fs::write(&created, b"created by interrupted restore").unwrap();

        recover_restore_markers().unwrap();
        assert!(preexisting.exists());
        assert!(!created.exists());
        assert!(!marker_path.exists());
        recover_restore_markers().unwrap();

        fs::remove_file(preexisting).unwrap();
    }

    #[test]
    fn recovery_removes_marker_and_unreadable_restore_created_keychain_entry() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "unreadable encrypted secret")]);

        assert!(Keychain::global().get_wallet_secret(&metadata.id).is_err());
        recover_restore_markers().unwrap();

        assert!(!marker_path.exists());
        assert!(!Keychain::global().wallet_items_exist(&metadata.id));
    }

    #[test]
    fn recovery_removes_unreadable_restore_created_entry_and_preserves_xpub() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let xpub = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        let xpub_key = format!("{}::wallet_xpub", metadata.id);
        let xpub_value = xpub.to_string();
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        crate::test_support::shared_mock_keychain().set_entries(vec![
            (xpub_key.as_str(), xpub_value.as_str()),
            (secret_key.as_str(), "unreadable encrypted secret"),
        ]);

        recover_restore_markers().unwrap();
        assert!(!marker_path.exists());
        assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));
        assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());

        assert!(Keychain::global().delete_wallet_xpub(&metadata.id));
    }

    #[test]
    fn recovery_does_not_broad_delete_inconsistent_keychain_snapshot() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let initial =
            RestoreArtifactSnapshot { keychain_items: true, ..RestoreArtifactSnapshot::default() };
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "unreadable encrypted secret")]);

        assert!(recover_restore_markers().is_err());
        assert!(marker_path.exists());
        assert!(Keychain::global().get_wallet_secret(&metadata.id).is_err());

        crate::test_support::shared_mock_keychain().reset();
        remove_marker(&marker_path).unwrap();
    }

    #[test]
    fn recovery_does_not_selectively_delete_without_approved_keychain_fingerprints() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let initial =
            RestoreArtifactSnapshot { keychain_items: true, ..RestoreArtifactSnapshot::default() };
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        let xpub = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        assert!(recover_restore_markers().is_err());
        assert!(marker_path.exists());
        assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));

        crate::test_support::shared_mock_keychain().reset();
        remove_marker(&marker_path).unwrap();
    }

    #[test]
    fn rollback_reports_a_changed_keychain_kind_once() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let original = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        let changed = bitcoin::bip32::Xpub::from_str(
            "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, original).unwrap();

        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let key = format!("{}::wallet_xpub", metadata.id);
        let changed_value = changed.to_string();
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(key.as_str(), changed_value.as_str())]);

        let mut failures = Vec::new();
        rollback_keychain(&metadata.id, &initial, &mut failures);

        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("pre-existing keychain xpub changed"));

        crate::test_support::shared_mock_keychain().reset();
    }

    #[test]
    fn recovery_retains_marker_when_owned_cleanup_fails() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
        fs::create_dir(&path).unwrap();
        let marker_path = write_test_marker(
            &metadata.id,
            &RestoreArtifactSnapshot::default(),
            RestoreMarkerPhase::Writing,
        );

        assert!(recover_restore_markers().is_err());
        assert!(marker_path.exists());

        fs::remove_dir(&path).unwrap();
        recover_restore_markers().unwrap();
        assert!(!marker_path.exists());
    }

    #[test]
    fn recovery_removes_marker_after_metadata_commit_without_cleaning_artifacts() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::app::reconcile::test_support::init_noop_updater();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let metadata = WalletMetadata::preview_new();
        let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
        fs::write(&path, b"committed artifact").unwrap();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path =
            write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::MetadataCommitted);
        Database::global().wallets.save_restored_wallet_metadata(metadata.clone()).unwrap();

        recover_restore_markers().unwrap();
        assert!(!marker_path.exists());
        assert!(path.exists());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn durable_wallet_lock_blocks_when_process_lease_state_is_missing() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let lease = WalletRestoreLease::acquire(&metadata).unwrap();
        ACTIVE_WALLET_RESTORE_LEASES.lock().remove(&validated.path_key());

        assert!(matches!(
            WalletRestoreLease::acquire(&metadata),
            Err(BackupError::WalletIdOccupied(id)) if id == metadata.id
        ));

        drop(lease);
        assert!(WalletRestoreLease::acquire(&metadata).is_ok());
    }

    #[test]
    fn interrupted_cleanup_resumes_from_durable_approval() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
        fs::write(&path, b"approved artifact").unwrap();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path =
            write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::CleanupInProgress);

        recover_restore_markers().unwrap();

        assert!(!path.exists());
        assert!(!marker_path.exists());
    }

    #[test]
    fn interrupted_cleanup_accepts_remaining_keychain_subset() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let mnemonic = bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let xpub = bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_key(&metadata.id, mnemonic).unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path =
            write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::CleanupInProgress);
        crate::test_support::shared_mock_keychain().fail_delete_at(3);

        assert!(recover_restore_markers().is_err());
        assert!(marker_path.exists());
        assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());
        assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));

        recover_restore_markers().unwrap();
        assert!(!marker_path.exists());
        assert!(!Keychain::global().wallet_items_exist(&metadata.id));
    }

    #[test]
    fn empty_wallet_data_directory_is_not_an_occupied_restore_artifact() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let directory = crate::database::wallet_data::wallet_data_directory_path(&metadata.id);
        fs::create_dir_all(&directory).unwrap();

        let id = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let snapshot = RestoreArtifactSnapshot::capture(&id).unwrap();

        assert!(snapshot.wallet_data_paths.contains(&directory));
        assert!(!snapshot.wallet_data_occupied);
        assert!(!snapshot.is_occupied());

        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn nested_wallet_data_cleanup_preserves_existing_entries() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let directory = crate::database::wallet_data::wallet_data_directory_path(&metadata.id);
        let existing = directory.join("nested/deep/existing.redb");
        let created = directory.join("nested/deep/created.redb");
        #[cfg(unix)]
        let external = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let external_sentinel = external.path().join("sentinel");
        #[cfg(unix)]
        fs::write(&external_sentinel, b"outside wallet data").unwrap();
        if let Some(parent) = existing.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&existing, b"existing").unwrap();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
        fs::write(&created, b"created").unwrap();
        #[cfg(unix)]
        {
            let link = directory.join("nested/external");
            std::os::unix::fs::symlink(external.path(), &link).unwrap();
        }

        recover_restore_markers().unwrap();

        assert!(existing.exists());
        assert!(!created.exists());
        assert!(!marker_path.exists());
        #[cfg(unix)]
        {
            assert!(external_sentinel.exists());
            assert!(fs::symlink_metadata(directory.join("nested/external")).is_err());
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_marker_is_quarantined_without_blocking_recovery() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let directory = marker_directory();
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("bad-marker.json");
        fs::write(&path, b"not-json").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            directory.join("missing-target"),
            directory.join(".bad-marker.json.tmp"),
        )
        .unwrap();

        recover_restore_markers().unwrap();

        assert!(!path.exists());
        let quarantine = directory.join(QUARANTINE_DIRECTORY_NAME);
        assert!(fs::read_dir(&quarantine).unwrap().next().is_some());
        fs::remove_dir_all(quarantine).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn approval_rejects_symlink_replacement() {
        use std::os::unix::fs::symlink;

        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let artifact =
            crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
        fs::write(&artifact, b"original").unwrap();
        let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
        let expected = RestoreArtifactSnapshot::capture(&validated).unwrap();
        let target = artifact.with_extension("replacement");
        fs::write(&target, b"replacement").unwrap();
        fs::remove_file(&artifact).unwrap();
        symlink(&target, &artifact).unwrap();

        assert!(matches!(
            WalletRestoreLease::acquire_for_approval(&metadata, &expected),
            Err(BackupError::ImportApprovalStale(id)) if id == metadata.id
        ));

        fs::remove_file(&artifact).unwrap();
        fs::remove_file(target).unwrap();
    }
}
