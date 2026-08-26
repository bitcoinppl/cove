pub mod label;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use label::LabelsTable;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use redb::{ReadOnlyTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    network::Network,
    wallet::{WalletAddressType, metadata::WalletId},
};
use cove_common::consts::{WALLET_DATA_DIR, wallet_data_dir_path};
use cove_types::redb::Json;

use ahash::AHashMap as HashMap;

pub static DATABASE_CONNECTIONS: Lazy<RwLock<HashMap<WalletId, Arc<redb::Database>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Per-wallet locks so concurrent opens of the same id serialize without blocking other wallets
static DATABASE_OPEN_LOCKS: Lazy<Mutex<HashMap<WalletId, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Per-wallet gates held while restore cleanup evicts and removes wallet data
static DATABASE_STORAGE_GATES: Lazy<Mutex<HashMap<WalletId, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn database_location(id: &WalletId, location: &Path) -> Result<PathBuf, std::io::Error> {
    let dir = location.join(id.as_str());

    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }

    Ok(dir.join("wallet_data.encrypted.json.redb"))
}

pub(crate) const TABLE: TableDefinition<&'static str, Json<WalletData>> =
    TableDefinition::new("wallet_data.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalletData {
    /// number of addresses scanned
    ScanState(ScanState),
    ReceiveAddressCache(ReceiveAddressCache),
    PayjoinSenderSession(PayjoinSenderSession),
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum WalletDataKey {
    ScanState(WalletAddressType),
    ReceiveAddressCache,
    PayjoinSenderSession,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
pub enum ScanState {
    NotStarted,
    Scanning(ScanningInfo),
    Completed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Record)]
pub struct ScanningInfo {
    pub address_type: WalletAddressType,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReceiveAddressCache {
    pub derivation_index: u32,
    pub first_shown_at_secs: u64,
    pub wallet_id: WalletId,
    pub network: Network,
    pub address_type: WalletAddressType,
}

impl ReceiveAddressCache {
    pub fn with_visible_window_start(mut self, now_secs: u64) -> Self {
        self.first_shown_at_secs = now_secs;
        self
    }
}

/// Consensus-encoded bytes of a bitcoin transaction stored in the database.
///
/// `bitcoin::Transaction` does not implement serde natively; storing the consensus-encoded
/// bytes avoids an orphan-impl problem while keeping the format identical to what is broadcast.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Eq,
    PartialEq,
    derive_more::From,
    derive_more::Into,
    derive_more::AsRef,
)]
pub struct TransactionBytes(Vec<u8>);

/// Terminal action decided for a payjoin session before the broadcast completes
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum PendingAction {
    BroadcastFallback,
    BroadcastProposal { transaction: TransactionBytes },
}

/// Event log for an in-flight payjoin sender session, allowing it to survive app restarts
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PayjoinSenderSession {
    pub events: Vec<String>,
    pub fallback_tx: TransactionBytes,
    #[serde(default)]
    pub created_at_secs: Option<u64>,
    #[serde(default)]
    pub pending_action: Option<PendingAction>,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletDataDb {
    pub id: WalletId,
    pub db: Arc<redb::Database>,
    pub labels: LabelsTable,
    storage: WalletDataStorage,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WalletDataStorage {
    Persistent,
    InMemory,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum WalletDataError {
    #[error(transparent)]
    WalletLifecycle(#[from] crate::wallet_lifecycle::WalletLifecycleFailure),

    #[error("Unable to access database for wallet {id}, error: {error}")]
    DatabaseAccess { id: WalletId, error: String },

    #[error("Unable to access table for wallet {id}, error: {error}")]
    TableAccess { id: WalletId, error: String },

    #[error("Unable to read: {0}")]
    Read(String),

    #[error("Unable to save: {0}")]
    Save(String),

    #[error("Unsupported database version for wallet {id}: {version}")]
    UnsupportedVersion { id: WalletId, version: super::error::UnsupportedDbVersion },
}

pub type Error = WalletDataError;
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl WalletDataDb {
    /// Gets an existing database or creates a new one
    pub fn new_or_existing(id: WalletId) -> Result<Self> {
        Self::new_with_db_location(id, &WALLET_DATA_DIR)
    }

    /// Creates an ephemeral wallet-data database that never touches the wallet-data directory
    pub(crate) fn new_in_memory(id: WalletId) -> Result<Self> {
        let db = redb::Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(|error| WalletDataError::DatabaseAccess {
                id: id.clone(),
                error: error.to_string(),
            })?;

        Self::new_with_db(id, Arc::new(db), WalletDataStorage::InMemory)
    }

    fn new_with_db_location(id: WalletId, db_location: &Path) -> Result<Self> {
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())?;
        let storage_gate = wallet_data_storage_gate(&id);
        let _storage_guard = storage_gate.lock();
        let db = get_or_create_database_locked(&id, db_location)?;
        Self::new_with_db(id, db, WalletDataStorage::Persistent)
    }

    fn new_with_db(
        id: WalletId,
        db: Arc<redb::Database>,
        storage: WalletDataStorage,
    ) -> Result<Self> {
        let write_txn = db.begin_write().map_err(|e| WalletDataError::DatabaseAccess {
            id: id.clone(),
            error: e.to_string(),
        })?;

        // create table if it doesn't exist
        write_txn
            .open_table(TABLE)
            .map_err(|e| WalletDataError::TableAccess { id: id.clone(), error: e.to_string() })?;
        let labels = LabelsTable::new(id.clone(), db.clone(), &write_txn);

        // commit the write transaction
        write_txn.commit().map_err(|e| WalletDataError::DatabaseAccess {
            id: id.clone(),
            error: e.to_string(),
        })?;

        Ok(Self { id, db, labels, storage })
    }

    pub(crate) const fn is_in_memory(&self) -> bool {
        matches!(self.storage, WalletDataStorage::InMemory)
    }

    pub fn get_scan_state(&self, address_type: WalletAddressType) -> Result<Option<ScanState>> {
        let key = WalletDataKey::ScanState(address_type);
        let value = self.get(key)?;

        let Some(WalletData::ScanState(scan_state)) = value else {
            return Ok(None);
        };

        Ok(Some(scan_state))
    }

    pub fn set_scan_state(
        &self,
        type_: WalletAddressType,
        scan_state: impl Into<ScanState>,
    ) -> Result<()> {
        let scan_state = scan_state.into();
        debug!("setting scan state for {type_:?}, scan_state: {scan_state:?}");

        let key = WalletDataKey::ScanState(type_);
        let value = WalletData::ScanState(scan_state);

        self.set(key, value)
    }

    pub fn get_receive_address_cache(&self) -> Result<Option<ReceiveAddressCache>> {
        let value = self.get(WalletDataKey::ReceiveAddressCache)?;

        let Some(WalletData::ReceiveAddressCache(cache)) = value else {
            return Ok(None);
        };

        Ok(Some(cache))
    }

    pub fn set_receive_address_cache(&self, cache: ReceiveAddressCache) -> Result<()> {
        self.set(WalletDataKey::ReceiveAddressCache, WalletData::ReceiveAddressCache(cache))
    }

    pub fn delete_receive_address_cache(&self) -> Result<()> {
        self.delete(WalletDataKey::ReceiveAddressCache)
    }

    pub fn get_payjoin_sender_session(&self) -> Result<Option<PayjoinSenderSession>> {
        let value = self.get(WalletDataKey::PayjoinSenderSession)?;

        let Some(WalletData::PayjoinSenderSession(session)) = value else {
            return Ok(None);
        };

        Ok(Some(session))
    }

    pub fn set_payjoin_sender_session(&self, session: PayjoinSenderSession) -> Result<()> {
        self.set(WalletDataKey::PayjoinSenderSession, WalletData::PayjoinSenderSession(session))
    }

    /// Persist the terminal fallback marker authorized by wallet quiescence
    pub(crate) fn set_terminal_payjoin_fallback(
        &self,
        session: PayjoinSenderSession,
        authority: &crate::wallet_lifecycle::TerminalPayjoinPersistenceAuthority,
    ) -> Result<()> {
        let _persistence = (!self.is_in_memory()).then(|| authority.begin(&self.id)).transpose()?;

        self.set_without_lifecycle(
            WalletDataKey::PayjoinSenderSession,
            WalletData::PayjoinSenderSession(session),
        )
    }

    pub fn delete_payjoin_sender_session(&self) -> Result<()> {
        self.delete(WalletDataKey::PayjoinSenderSession)
    }

    fn get(&self, key: WalletDataKey) -> Result<Option<WalletData>> {
        let table = self.read_table()?;

        let value = table
            .get(key.as_str())
            .map_err(|error| Error::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn set(&self, key: WalletDataKey, value: WalletData) -> Result<()> {
        let _persistence = (!self.is_in_memory())
            .then(|| {
                crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
                    .begin_persistence_operation(self.id.clone())
            })
            .transpose()?;

        self.set_without_lifecycle(key, value)
    }

    fn set_without_lifecycle(&self, key: WalletDataKey, value: WalletData) -> Result<()> {
        let write_txn = self.db.begin_write().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        {
            let mut table = write_txn.open_table(TABLE).map_err(|error| Error::TableAccess {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

            table.insert(key.as_str(), value).map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        Ok(())
    }

    fn delete(&self, key: WalletDataKey) -> Result<()> {
        let _persistence = (!self.is_in_memory())
            .then(|| {
                crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
                    .begin_persistence_operation(self.id.clone())
            })
            .transpose()?;
        let write_txn = self.db.begin_write().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        {
            let mut table = write_txn.open_table(TABLE).map_err(|error| Error::TableAccess {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

            table.remove(key.as_str()).map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        Ok(())
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Json<WalletData>>, Error> {
        let read_txn = self.db.begin_read().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        let table = read_txn.open_table(TABLE).map_err(|error| Error::TableAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        Ok(table)
    }
}

fn get_or_create_database_locked(id: &WalletId, location: &Path) -> Result<Arc<redb::Database>> {
    let path = database_location(id, location)
        .map_err(|e| WalletDataError::DatabaseAccess { id: id.clone(), error: e.to_string() })?;

    // fast path when the connection is already registered
    if let Some(db) = DATABASE_CONNECTIONS.read().get(id).cloned() {
        return Ok(db);
    }

    // only serialize opens for this wallet id; other wallets keep opening in parallel
    let open_lock = {
        let mut locks = DATABASE_OPEN_LOCKS.lock();
        locks.entry(id.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
    };
    let _open_guard = open_lock.lock();

    // another caller may have finished opening while we waited
    if let Some(db) = DATABASE_CONNECTIONS.read().get(id).cloned() {
        return Ok(db);
    }

    let db = super::encrypted_backend::open_or_create_database(&path).map_err(|e| match e {
        super::error::DatabaseError::UnsupportedVersion(version) => {
            WalletDataError::UnsupportedVersion { id: id.clone(), version }
        }
        other => WalletDataError::DatabaseAccess { id: id.clone(), error: other.to_string() },
    })?;

    let db = Arc::new(db);
    DATABASE_CONNECTIONS.write().insert(id.clone(), db.clone());

    Ok(db)
}

pub fn delete_database(id: &WalletId) -> Result<(), std::io::Error> {
    delete_database_at_location(id, &WALLET_DATA_DIR)
}

/// Remove the complete wallet-data namespace without creating a missing path
pub(crate) fn delete_wallet_data_directory(id: &WalletId) -> Result<(), std::io::Error> {
    delete_wallet_data_directory_at_location(id, &WALLET_DATA_DIR)
}

pub(crate) fn delete_wallet_data_directory_at_location(
    id: &WalletId,
    location: &Path,
) -> Result<(), std::io::Error> {
    delete_wallet_data_directory_with_sync(id, location, sync_immediate_parent)
}

fn delete_wallet_data_directory_with_sync(
    id: &WalletId,
    location: &Path,
    sync_parent: impl FnOnce(&Path) -> Result<(), std::io::Error>,
) -> Result<(), std::io::Error> {
    let directory = location.join(id.as_str());
    let metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    evict_wallet_data_connections(id);
    if let Some(metadata) = metadata {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            std::fs::remove_dir_all(&directory)?;
        } else {
            std::fs::remove_file(&directory)?;
        }
    }

    sync_parent(&directory)
}

fn sync_immediate_parent(directory: &Path) -> Result<(), std::io::Error> {
    let Some(parent) = directory.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }

    std::fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
pub(crate) fn wallet_data_artifact_paths(id: &WalletId) -> Vec<PathBuf> {
    let directory = wallet_data_directory_path(id);
    let mut paths = vec![directory.clone()];

    let Ok(metadata) = std::fs::symlink_metadata(&directory) else {
        return paths;
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return paths;
    }

    let Ok(entries) = std::fs::read_dir(&directory) else {
        return paths;
    };

    paths.extend(entries.filter_map(std::result::Result::ok).map(|entry| entry.path()));
    paths
}

/// Enumerate wallet-data artifacts without following symbolic links
///
/// Restore recovery uses this checked owner API so nested files and directories are cleaned
/// without hiding permission or I/O failures
pub(crate) fn wallet_data_artifact_paths_checked(
    id: &WalletId,
) -> Result<Vec<PathBuf>, std::io::Error> {
    let root_metadata = match std::fs::symlink_metadata(&*WALLET_DATA_DIR) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![wallet_data_directory_path(id)]);
        }
        Err(error) => return Err(error),
    };
    if root_metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "wallet-data root is a symbolic link",
        ));
    }
    if !root_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "wallet-data root is not a directory",
        ));
    }

    let directory = wallet_data_directory_path(id);
    let metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![directory]),
        Err(error) => return Err(error),
    };

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(vec![directory]);
    }

    let mut paths = vec![directory.clone()];
    enumerate_wallet_data_children(&directory, &mut paths)?;
    Ok(paths)
}

fn enumerate_wallet_data_children(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        let is_directory = metadata.is_dir() && !metadata.file_type().is_symlink();
        paths.push(path.clone());

        if is_directory {
            enumerate_wallet_data_children(&path, paths)?;
        }
    }

    Ok(())
}

/// Return the wallet-data directory without opening or creating it
///
/// Restore recovery uses this owner-provided path only after the wallet id has
/// passed restore validation, so inspecting a marker never creates storage
pub(crate) fn wallet_data_directory_path(id: &WalletId) -> PathBuf {
    wallet_data_dir_path().join(id.as_str())
}

/// List wallet-data root entries so restore validation can reject case-folded aliases
pub(crate) fn wallet_data_root_entries() -> Result<Vec<PathBuf>, std::io::Error> {
    let metadata = match std::fs::symlink_metadata(&*WALLET_DATA_DIR) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "wallet-data root is a symbolic link",
        ));
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "wallet-data root is not a directory",
        ));
    }

    let entries = std::fs::read_dir(&*WALLET_DATA_DIR)?;
    entries.map(|entry| entry.map(|entry| entry.path())).collect()
}

/// Remove one restore-owned wallet-data artifact without opening the database
pub(crate) fn remove_wallet_artifact(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let result = if metadata.file_type().is_symlink() || !metadata.is_dir() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir(path)
    };

    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn directory_contains_wallet_data(directory: &Path) -> bool {
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) => return error.kind() != std::io::ErrorKind::NotFound,
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return true;
    }

    std::fs::read_dir(directory).map(|mut entries| entries.next().is_some()).unwrap_or(true)
}

/// Drop all cached wallet data connections and open locks
pub fn clear_database_connections() {
    DATABASE_CONNECTIONS.write().clear();
    DATABASE_OPEN_LOCKS.lock().clear();
}

/// Return the shared per-wallet gate used by database opens and restore cleanup
pub(crate) fn wallet_data_storage_gate(id: &WalletId) -> Arc<Mutex<()>> {
    let mut gates = DATABASE_STORAGE_GATES.lock();
    gates.entry(id.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
}

/// Evict cached wallet-data connections before restore cleanup
pub(crate) fn evict_wallet_data_connections(id: &WalletId) {
    DATABASE_CONNECTIONS.write().remove(id);
    DATABASE_OPEN_LOCKS.lock().remove(id);
}

fn delete_database_at_location(id: &WalletId, location: &Path) -> Result<(), std::io::Error> {
    let storage_gate = wallet_data_storage_gate(id);
    let _storage_guard = storage_gate.lock();
    DATABASE_CONNECTIONS.write().remove(id);
    DATABASE_OPEN_LOCKS.lock().remove(id);

    // a wallet that never opened its wallet-data database has nothing to delete,
    // and deletion must still converge when the file is already gone
    match std::fs::remove_file(database_location(id, location)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl WalletDataKey {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ScanState(WalletAddressType::NativeSegwit) => "scan_state_native_segwit",
            Self::ScanState(WalletAddressType::WrappedSegwit) => "scan_state_wrapped_segwit",
            Self::ScanState(WalletAddressType::Legacy) => "scan_state_legacy",
            Self::ReceiveAddressCache => "receive_address_cache",
            Self::PayjoinSenderSession => "payjoin_sender_session",
        }
    }
}

impl ScanningInfo {
    pub const fn new(address_type: WalletAddressType) -> Self {
        Self { address_type, count: 0 }
    }
}

impl From<ScanningInfo> for ScanState {
    fn from(info: ScanningInfo) -> Self {
        Self::Scanning(info)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn new_test_wallet_data_db(id: WalletId) -> (WalletDataDb, tempfile::TempDir) {
        crate::database::encrypted_backend::tests::set_test_encryption_key();
        clear_wallet_registry_entry(&id);
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db =
            WalletDataDb::new_with_db_location(id, tmp.path()).expect("failed to create test db");
        (db, tmp)
    }

    pub(crate) fn clear_wallet_registry_entry(id: &WalletId) {
        DATABASE_CONNECTIONS.write().remove(id);
        DATABASE_OPEN_LOCKS.lock().remove(id);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::{Barrier, mpsc},
        time::Duration,
    };

    use super::*;

    #[test]
    fn unreadable_wallet_data_path_is_treated_as_occupied() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let parent_file = tmp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"occupied").expect("failed to create parent file");

        assert!(directory_contains_wallet_data(&parent_file.join("wallet")));
    }

    #[test]
    fn deleting_missing_wallet_data_does_not_create_its_directory() {
        let wallet_id = WalletId::preview_new_random();
        let directory = wallet_data_directory_path(&wallet_id);
        assert!(!directory.exists());

        delete_wallet_data_directory(&wallet_id).expect("missing wallet data is idempotent");

        assert!(!directory.exists());
    }

    #[test]
    fn retry_after_parent_sync_failure_syncs_parent_when_wallet_path_is_absent() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let wallet_id = WalletId::preview_new_random();
        let directory = tmp.path().join(wallet_id.as_str());
        std::fs::create_dir(&directory).expect("wallet directory is created");

        let first = delete_wallet_data_directory_with_sync(&wallet_id, tmp.path(), |_| {
            Err(std::io::Error::other("injected parent sync failure"))
        });
        assert!(first.is_err());
        assert!(!directory.exists(), "the first attempt removes the wallet directory");

        let sync_calls = Cell::new(0);
        delete_wallet_data_directory_with_sync(&wallet_id, tmp.path(), |requested_directory| {
            assert_eq!(requested_directory, directory);
            sync_calls.set(sync_calls.get() + 1);
            Ok(())
        })
        .expect("retry syncs the deletion even though the wallet path is absent");

        assert_eq!(sync_calls.get(), 1);
    }

    #[test]
    fn concurrent_new_or_existing_calls_share_one_database_handle() {
        crate::database::encrypted_backend::tests::set_test_encryption_key();
        let wallet_id = WalletId::preview_new_random();
        test_support::clear_wallet_registry_entry(&wallet_id);
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let location = Arc::new(tmp.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(16));

        let handles = (0..16)
            .map(|_| {
                let wallet_id = wallet_id.clone();
                let location = Arc::clone(&location);
                let barrier = Arc::clone(&barrier);

                std::thread::spawn(move || {
                    barrier.wait();
                    WalletDataDb::new_with_db_location(wallet_id, location.as_ref().as_path())
                })
            })
            .collect::<Vec<_>>();

        let databases = handles
            .into_iter()
            .map(|handle| handle.join().expect("wallet data open thread should not panic"))
            .collect::<Result<Vec<_>>>()
            .expect("all concurrent wallet data opens should succeed");

        assert!(databases.windows(2).all(|pair| Arc::ptr_eq(&pair[0].db, &pair[1].db)));

        test_support::clear_wallet_registry_entry(&wallet_id);
    }

    #[test]
    fn concurrent_opens_for_different_wallets_succeed_independently() {
        crate::database::encrypted_backend::tests::set_test_encryption_key();
        let wallet_ids = (0..8).map(|_| WalletId::preview_new_random()).collect::<Vec<_>>();
        for wallet_id in &wallet_ids {
            test_support::clear_wallet_registry_entry(wallet_id);
        }

        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let location = Arc::new(tmp.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(wallet_ids.len()));

        let handles = wallet_ids
            .iter()
            .cloned()
            .map(|wallet_id| {
                let location = Arc::clone(&location);
                let barrier = Arc::clone(&barrier);

                std::thread::spawn(move || {
                    barrier.wait();
                    WalletDataDb::new_with_db_location(wallet_id, location.as_ref().as_path())
                })
            })
            .collect::<Vec<_>>();

        let databases = handles
            .into_iter()
            .map(|handle| handle.join().expect("wallet data open thread should not panic"))
            .collect::<Result<Vec<_>>>()
            .expect("all concurrent independent wallet data opens should succeed");

        for left in 0..databases.len() {
            for right in (left + 1)..databases.len() {
                assert!(!Arc::ptr_eq(&databases[left].db, &databases[right].db));
            }
        }

        for wallet_id in &wallet_ids {
            test_support::clear_wallet_registry_entry(wallet_id);
        }
    }

    #[test]
    fn storage_gate_blocks_database_open_until_cleanup_releases_it() {
        crate::database::encrypted_backend::tests::set_test_encryption_key();
        let wallet_id = WalletId::preview_new_random();
        test_support::clear_wallet_registry_entry(&wallet_id);
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let gate = wallet_data_storage_gate(&wallet_id);
        let gate_guard = gate.lock();
        let barrier = Arc::new(Barrier::new(2));
        let (sender, receiver) = mpsc::channel();
        let location = tmp.path().to_path_buf();
        let thread_wallet_id = wallet_id.clone();
        let thread_barrier = Arc::clone(&barrier);

        let handle = std::thread::spawn(move || {
            thread_barrier.wait();
            let result = WalletDataDb::new_with_db_location(thread_wallet_id, &location);
            sender.send(result.is_ok()).expect("open result receiver should exist");
        });

        barrier.wait();
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        drop(gate_guard);

        assert!(receiver.recv_timeout(Duration::from_secs(2)).unwrap());
        handle.join().expect("wallet data open thread should not panic");
        test_support::clear_wallet_registry_entry(&wallet_id);
    }

    #[test]
    fn receive_address_cache_round_trips() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id.clone());
        let cache = ReceiveAddressCache {
            derivation_index: 7,
            first_shown_at_secs: 1_700_000_000,
            wallet_id,
            network: Network::Signet,
            address_type: WalletAddressType::NativeSegwit,
        };

        db.set_receive_address_cache(cache.clone()).unwrap();

        assert_eq!(db.get_receive_address_cache().unwrap(), Some(cache));
    }

    #[test]
    fn receive_address_cache_visible_window_start_updates_timer_only() {
        let wallet_id = WalletId::preview_new_random();
        let cache = ReceiveAddressCache {
            derivation_index: 7,
            first_shown_at_secs: 1_700_000_000,
            wallet_id: wallet_id.clone(),
            network: Network::Signet,
            address_type: WalletAddressType::NativeSegwit,
        };

        let reset = cache.with_visible_window_start(1_700_000_300);

        assert_eq!(reset.derivation_index, 7);
        assert_eq!(reset.first_shown_at_secs, 1_700_000_300);
        assert_eq!(reset.wallet_id, wallet_id);
        assert_eq!(reset.network, Network::Signet);
        assert_eq!(reset.address_type, WalletAddressType::NativeSegwit);
    }

    #[test]
    fn delete_receive_address_cache_clears_cache() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id.clone());
        let cache = ReceiveAddressCache {
            derivation_index: 7,
            first_shown_at_secs: 1_700_000_000,
            wallet_id,
            network: Network::Signet,
            address_type: WalletAddressType::NativeSegwit,
        };

        db.set_receive_address_cache(cache).unwrap();
        db.delete_receive_address_cache().unwrap();

        assert_eq!(db.get_receive_address_cache().unwrap(), None);
    }

    #[test]
    fn delete_missing_receive_address_cache_succeeds() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id);

        db.delete_receive_address_cache().unwrap();

        assert_eq!(db.get_receive_address_cache().unwrap(), None);
    }

    #[test]
    fn payjoin_sender_session_round_trips() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id);
        let session = PayjoinSenderSession {
            events: vec![r#"{"PostedOriginalPsbt":[]}"#.to_string()],
            fallback_tx: vec![0x02, 0x00, 0x00, 0x00].into(),
            created_at_secs: None,
            pending_action: None,
        };

        db.set_payjoin_sender_session(session.clone()).unwrap();

        assert_eq!(db.get_payjoin_sender_session().unwrap(), Some(session));
    }

    #[test]
    fn delete_payjoin_sender_session_clears_session() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id);
        let session = PayjoinSenderSession {
            events: vec![r#"{"Closed":"Failure"}"#.to_string()],
            fallback_tx: vec![0x02, 0x00, 0x00, 0x00].into(),
            created_at_secs: None,
            pending_action: None,
        };

        db.set_payjoin_sender_session(session).unwrap();
        db.delete_payjoin_sender_session().unwrap();

        assert_eq!(db.get_payjoin_sender_session().unwrap(), None);
    }

    #[test]
    fn delete_missing_payjoin_sender_session_succeeds() {
        let wallet_id = WalletId::preview_new_random();
        let (db, _tmp) = test_support::new_test_wallet_data_db(wallet_id);

        db.delete_payjoin_sender_session().unwrap();

        assert_eq!(db.get_payjoin_sender_session().unwrap(), None);
    }
}
