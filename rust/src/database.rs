//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod global_bool;
pub mod wallet;

use std::{path::PathBuf, sync::Arc};

use global_bool::GlobalBoolTable;
use wallet::WalletTable;

use eyre::Context;
use log::{debug, error, info};
use once_cell::sync::OnceCell;
use redb::TableDefinition;

use crate::view_model::wallet::WalletId;

pub static DATABASE: OnceCell<Database> = OnceCell::new();

pub const GLOBAL_BOOL_CONFIG: TableDefinition<&'static str, bool> =
    TableDefinition::new("global_bool_config");

pub const WALLETS: TableDefinition<&'static str, Vec<WalletId>> = TableDefinition::new("wallets");

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    pub db: Arc<redb::Database>,
    pub global_bool: GlobalBoolTable,
    pub wallets: WalletTable,
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
}

impl Database {
    pub fn global() -> &'static Database {
        DATABASE.get_or_init(|| {
            let db = get_or_create_database();
            create_all_tables(&db);

            let db = Arc::new(db);
            let wallets = WalletTable::new(db.clone());
            let global_bool = GlobalBoolTable::new(db.clone());

            Database {
                db,
                wallets,
                global_bool,
            }
        })
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
