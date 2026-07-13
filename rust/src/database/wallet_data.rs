pub mod label;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use label::LabelsTable;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use redb::{ReadOnlyTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    network::Network,
    wallet::{WalletAddressType, metadata::WalletId},
};
use cove_common::consts::WALLET_DATA_DIR;
use cove_types::redb::Json;

use ahash::AHashMap as HashMap;

pub static DATABASE_CONNECTIONS: Lazy<RwLock<HashMap<WalletId, Arc<redb::Database>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

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
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum WalletDataKey {
    ScanState(WalletAddressType),
    ReceiveAddressCache,
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

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletDataDb {
    pub id: WalletId,
    pub db: Arc<redb::Database>,
    pub labels: LabelsTable,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum WalletDataError {
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

    fn new_with_db_location(id: WalletId, db_location: &Path) -> Result<Self> {
        let db = get_or_create_database(&id, db_location)?;
        let write_txn = db.begin_write().map_err(|e| WalletDataError::DatabaseAccess {
            id: id.clone(),
            error: e.to_string(),
        })?;

        // create table if it doesn't exist
        write_txn
            .open_table(TABLE)
            .map_err(|e| WalletDataError::TableAccess { id: id.clone(), error: e.to_string() })?;
        let labels = LabelsTable::new(db.clone(), &write_txn);

        // commit the write transaction
        write_txn.commit().map_err(|e| WalletDataError::DatabaseAccess {
            id: id.clone(),
            error: e.to_string(),
        })?;

        Ok(Self { id, db, labels })
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

    fn get(&self, key: WalletDataKey) -> Result<Option<WalletData>> {
        let table = self.read_table()?;

        let value = table
            .get(key.as_str())
            .map_err(|error| Error::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn set(&self, key: WalletDataKey, value: WalletData) -> Result<()> {
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

/// Get an existing database or create a new one
pub fn get_or_create_database(id: &WalletId, location: &Path) -> Result<Arc<redb::Database>> {
    let path = database_location(id, location)
        .map_err(|e| WalletDataError::DatabaseAccess { id: id.clone(), error: e.to_string() })?;

    let mut db_connections = DATABASE_CONNECTIONS.write();
    if let Some(db) = db_connections.get(id) {
        return Ok(db.clone());
    }

    let db = super::encrypted_backend::open_or_create_database(&path).map_err(|e| match e {
        super::error::DatabaseError::UnsupportedVersion(version) => {
            WalletDataError::UnsupportedVersion { id: id.clone(), version }
        }
        other => WalletDataError::DatabaseAccess { id: id.clone(), error: other.to_string() },
    })?;

    let db = Arc::new(db);
    db_connections.insert(id.clone(), db.clone());

    Ok(db)
}

pub fn delete_database(id: &WalletId) -> Result<(), std::io::Error> {
    delete_database_at_location(id, &WALLET_DATA_DIR)
}

fn delete_database_at_location(id: &WalletId, location: &Path) -> Result<(), std::io::Error> {
    {
        let mut db_connections = DATABASE_CONNECTIONS.write();
        db_connections.remove(id);
    }

    std::fs::remove_file(database_location(id, location)?)
}

impl WalletDataKey {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ScanState(WalletAddressType::NativeSegwit) => "scan_state_native_segwit",
            Self::ScanState(WalletAddressType::WrappedSegwit) => "scan_state_wrapped_segwit",
            Self::ScanState(WalletAddressType::Legacy) => "scan_state_legacy",
            Self::ReceiveAddressCache => "receive_address_cache",
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
        DATABASE_CONNECTIONS.write().remove(&id);
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db =
            WalletDataDb::new_with_db_location(id, tmp.path()).expect("failed to create test db");
        (db, tmp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn concurrent_new_or_existing_calls_share_one_database_handle() {
        crate::database::encrypted_backend::tests::set_test_encryption_key();
        let wallet_id = WalletId::preview_new_random();
        DATABASE_CONNECTIONS.write().remove(&wallet_id);
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

        DATABASE_CONNECTIONS.write().remove(&wallet_id);
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
}
