pub mod label;

use std::{path::PathBuf, sync::Arc};

use label::LabelsTable;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use redb::{ReadOnlyTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::{
    consts::WALLET_DATA_DIR,
    redb::Json,
    wallet::{metadata::WalletId, WalletAddressType},
};

use ahash::AHashMap as HashMap;

pub static DATABASE_CONNECTIONS: Lazy<RwLock<HashMap<WalletId, Arc<redb::Database>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn database_location(id: &WalletId) -> PathBuf {
    let dir = WALLET_DATA_DIR.join(id.as_str());

    if !dir.exists() {
        std::fs::create_dir_all(&dir).expect("always work to create dir");
    }

    dir.join("wallet_data.json")
}

const TABLE: TableDefinition<&'static str, Json<WalletData>> =
    TableDefinition::new("wallet_data.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalletData {
    /// number of addresses scanned
    ScanState(ScanState),
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum WalletDataKey {
    ScanState(WalletAddressType),
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

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletDataDb {
    pub id: WalletId,
    pub db: Arc<redb::Database>,
    pub labels: LabelsTable,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WalletDataError {
    #[error("Unable to access database for wallet {id}, error: {error}")]
    DatabaseAccess { id: WalletId, error: String },

    #[error("Unable to access table for wallet {id}, error: {error}")]
    TableAccess { id: WalletId, error: String },

    #[error("Unable to read: {0}")]
    Read(String),

    #[error("Unable to save: {0}")]
    Save(String),
}

pub type Error = WalletDataError;
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl WalletDataDb {
    pub fn new(id: WalletId) -> Self {
        let db = get_or_create_database(&id);
        let write_txn = db.begin_write().expect("failed to begin write transaction");

        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        let labels = LabelsTable::new(db.clone(), &write_txn);

        Self { id, db, labels }
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

    fn get(&self, key: WalletDataKey) -> Result<Option<WalletData>> {
        let table = self.read_table()?;

        let value = table
            .get(key.as_str())
            .map_err(|error| Error::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn set(&self, key: WalletDataKey, value: WalletData) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess {
                    id: self.id.clone(),
                    error: error.to_string(),
                })?;

            table
                .insert(key.as_str(), value)
                .map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|error| Error::DatabaseAccess {
            id: self.id.clone(),
            error: error.to_string(),
        })?;

        Ok(())
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Json<WalletData>>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        Ok(table)
    }
}

pub fn get_or_create_database(id: &WalletId) -> Arc<redb::Database> {
    let database_location = database_location(id);

    // check if we already have a database connection for this id and return it
    {
        let db_connections = DATABASE_CONNECTIONS.read();
        if let Some(db) = db_connections.get(id) {
            return db.clone();
        }
    }

    if database_location.exists() {
        let db = redb::Database::open(&database_location);
        match db {
            Ok(db) => {
                let mut db_connections = DATABASE_CONNECTIONS.write();
                let db = Arc::new(db);
                db_connections.insert(id.clone(), db.clone());

                return db;
            }
            Err(error) => {
                error!("failed to open database for {id}, error: {error:?}, creating a new one");
            }
        }
    };

    info!(
        "Creating a new database for wallet {id}, at {}",
        database_location.display()
    );

    let db = redb::Database::create(&database_location).expect("failed to create database");
    let mut db_connections = DATABASE_CONNECTIONS.write();
    let db = Arc::new(db);
    db_connections.insert(id.clone(), db.clone());

    db
}

pub fn delete_database(id: &WalletId) -> Result<(), std::io::Error> {
    {
        let mut db_connections = DATABASE_CONNECTIONS.write();
        db_connections.remove(id);
    }

    std::fs::remove_file(database_location(id))
}

impl WalletDataKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            WalletDataKey::ScanState(WalletAddressType::NativeSegwit) => "scan_state_native_segwit",
            WalletDataKey::ScanState(WalletAddressType::WrappedSegwit) => {
                "scan_state_wrapped_segwit"
            }
            WalletDataKey::ScanState(WalletAddressType::Legacy) => "scan_state_legacy",
        }
    }
}

impl ScanningInfo {
    pub fn new(address_type: WalletAddressType) -> Self {
        Self {
            address_type,
            count: 0,
        }
    }
}

impl From<ScanningInfo> for ScanState {
    fn from(info: ScanningInfo) -> Self {
        Self::Scanning(info)
    }
}
