//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

use std::{path::PathBuf, sync::Arc};

use bdk_wallet::bitcoin::Network;
use eyre::{Context, Result};
use log::{debug, error, info};
use once_cell::sync::OnceCell;
use redb::TableDefinition;

use crate::{
    update::{Update, Updater},
    view_model::wallet::WalletId,
};

pub static DATABASE: OnceCell<Database> = OnceCell::new();

const GLOBAL_BOOL_CONFIG: TableDefinition<&'static str, bool> =
    TableDefinition::new("global_bool_config");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalBoolConfigKey {
    CompletedOnboarding,
}

const WALLETS: TableDefinition<&'static str, Vec<WalletId>> = TableDefinition::new("wallets");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum WalletKey {
    Bitcoin,
    Testnet,
}

impl From<Network> for WalletKey {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => WalletKey::Bitcoin,
            Network::Testnet => WalletKey::Testnet,
            other => panic!("unsupported network: {other:?}"),
        }
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    pub db: Arc<redb::Database>,
}

#[uniffi::export]
pub fn global() {
    Database::global();
}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

type Error = DatabaseError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum DatabaseError {
    #[error("failed to open database: {0}")]
    DatabaseAccessError(String),

    #[error("failed to open table: {0}")]
    TableAccessError(String),

    #[error("failed to get bool config value: {0}")]
    ConfigReadError(String),

    #[error("failed to get wallets: {0}")]
    WalletsReadError(String),

    #[error("failed to save bool config value: {0}")]
    ConfigSaveError(String),

    #[error("failed to save wallets: {0}")]
    WalletsSaveError(String),
}

#[uniffi::export]
impl Database {
    #[uniffi::constructor(name = "new")]
    pub fn new() -> Self {
        Self::global().clone()
    }

    pub fn get_bool_config(&self, key: GlobalBoolConfigKey) -> Result<bool, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(GLOBAL_BOOL_CONFIG)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| Error::ConfigReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or(false);

        Ok(value)
    }

    pub fn set_bool_config(&self, key: GlobalBoolConfigKey, value: bool) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(GLOBAL_BOOL_CONFIG)
                .map_err(|error| Error::TableAccessError(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| Error::ConfigSaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdate);

        Ok(())
    }

    pub fn toggle_bool_config(&self, key: GlobalBoolConfigKey) -> Result<(), Error> {
        let value = self.get_bool_config(key)?;

        let new_value = !value;
        self.set_bool_config(key, new_value)?;

        Ok(())
    }
}

impl Database {
    pub fn global() -> &'static Database {
        DATABASE.get_or_init(|| {
            let db = get_or_create_database();
            create_all_tables(&db);

            Database { db: Arc::new(db) }
        })
    }

    pub fn get_wallets(&self, network: Network) -> Result<Vec<WalletId>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(WALLETS)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let key: WalletKey = network.into();
        let key: &'static str = key.into();

        let value = table
            .get(key)
            .map_err(|error| Error::WalletsReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(value)
    }

    pub fn save_wallets(&self, network: Network, wallets: Vec<WalletId>) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(WALLETS)
                .map_err(|error| Error::TableAccessError(error.to_string()))?;

            let key: WalletKey = network.into();
            let key: &'static str = key.into();

            table
                .insert(key, wallets)
                .map_err(|error| Error::WalletsSaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdate);

        Ok(())
    }
}

fn get_or_create_database() -> redb::Database {
    let database_location = database_location();

    if database_location.exists() {
        let db = redb::Database::open(&database_location);
        match db {
            Ok(db) => return db,
            Err(error) => {
                error!("failed to open database, error: {error:?}, creating a new one");
            }
        }
    };

    info!(
        "Creating a new database, at {}",
        database_location.display()
    );

    redb::Database::create(&database_location).expect("failed to create database")
}

fn create_all_tables(db: &redb::Database) {
    debug!("creating all tables");

    let write_txn = db.begin_write().expect("failed to begin write transaction");

    // create table if it doesn't exist
    write_txn
        .open_table(GLOBAL_BOOL_CONFIG)
        .expect("failed to create table");

    write_txn
        .open_table(WALLETS)
        .expect("failed to create table");

    write_txn
        .commit()
        .expect("failed to commit write transaction");
}

fn database_location() -> PathBuf {
    let parent = dirs::home_dir()
        .expect("failed to get home document directory")
        .join("Library/Application Support/.data");

    if !parent.exists() {
        std::fs::create_dir_all(&parent)
            .wrap_err_with(|| {
                format!(
                    "failed to create data directory at {}",
                    parent.to_string_lossy()
                )
            })
            .unwrap();
    }

    parent.join("cove.db")
}
