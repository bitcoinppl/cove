//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod error;
pub mod global_config;
pub mod global_flag;
pub mod wallet;

use std::{path::PathBuf, sync::Arc};

use global_config::GlobalConfigTable;
use global_flag::GlobalFlagTable;
use wallet::WalletTable;

use once_cell::sync::OnceCell;
use tracing::{error, info};

use crate::consts::ROOT_DATA_DIR;

pub static DATABASE: OnceCell<Database> = OnceCell::new();

pub type Error = error::DatabaseError;

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    pub db: Arc<redb::Database>,
    pub global_flag: GlobalFlagTable,
    pub global_config: GlobalConfigTable,
    pub wallets: WalletTable,
}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

#[uniffi::export]
impl Database {
    #[uniffi::constructor(name = "new")]
    pub fn new() -> Self {
        Self::global().clone()
    }

    pub fn wallets(&self) -> WalletTable {
        self.wallets.clone()
    }

    pub fn global_config(&self) -> GlobalConfigTable {
        self.global_config.clone()
    }
}

impl Database {
    pub fn global() -> &'static Database {
        DATABASE.get_or_init(|| {
            let db = get_or_create_database();

            let write_txn = db.begin_write().expect("failed to begin write transaction");

            let db = Arc::new(db);

            let wallets = WalletTable::new(db.clone(), &write_txn);
            let global_flag = GlobalFlagTable::new(db.clone(), &write_txn);
            let global_config = GlobalConfigTable::new(db.clone(), &write_txn);

            write_txn
                .commit()
                .expect("failed to commit write transaction");

            Database {
                db,
                wallets,
                global_flag,
                global_config,
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

fn database_location() -> PathBuf {
    ROOT_DATA_DIR.join("cove.db")
}
