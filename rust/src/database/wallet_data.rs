use std::{path::PathBuf, sync::Arc};

use redb::{ReadOnlyTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    consts::ROOT_DATA_DIR,
    redb::Json,
    wallet::{metadata::WalletId, WalletAddressType},
};

fn database_location(id: &WalletId) -> PathBuf {
    ROOT_DATA_DIR.join("wallet_data").join(id.as_str())
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
}

#[derive(Debug, thiserror::Error, uniffi::Error, derive_more::Display)]
pub enum WalletDataError {
    #[display("Unable to access database for wallet {id}, error: {error}")]
    DatabaseAccessError { id: WalletId, error: String },

    #[display("Unable to access table for wallet {id}, error: {error}")]
    TableAccessError { id: WalletId, error: String },

    /// Unable to read: {0}
    ReadError(String),

    /// Unable to save: {0}
    SaveError(String),
}

pub type Error = WalletDataError;
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl WalletDataDb {
    pub fn new(id: WalletId) -> Self {
        let db = get_or_create_database(&id);
        let write_txn = db.begin_write().expect("failed to begin write transaction");
        let db = Arc::new(db);

        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { id, db }
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
        let key = WalletDataKey::ScanState(type_);
        let value = WalletData::ScanState(scan_state);

        self.set(key, value)
    }

    fn get(&self, key: WalletDataKey) -> Result<Option<WalletData>> {
        let table = self.read_table()?;

        let value = table
            .get(key.as_str())
            .map_err(|error| Error::ReadError(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn set(&self, key: WalletDataKey, value: WalletData) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        {
            let mut table =
                write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccessError {
                        id: self.id.clone(),
                        error: error.to_string(),
                    })?;

            table
                .insert(key.as_str(), value)
                .map_err(|error| Error::SaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        Ok(())
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Json<WalletData>>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccessError {
                id: self.id.clone(),
                error: error.to_string(),
            })?;

        Ok(table)
    }
}

pub fn get_or_create_database(id: &WalletId) -> redb::Database {
    let database_location = database_location(id);

    if database_location.exists() {
        let db = redb::Database::open(&database_location);
        match db {
            Ok(db) => return db,
            Err(error) => {
                error!("failed to open database for {id}, error: {error:?}, creating a new one");
            }
        }
    };

    info!(
        "Creating a new database for wallet {id}, at {}",
        database_location.display()
    );

    redb::Database::create(&database_location).expect("failed to create database")
}

pub fn delete_database(id: &WalletId) {
    if let Err(error) = std::fs::remove_file(database_location(id)) {
        error!("Unable to delete wallet data: {error:?}")
    }
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
