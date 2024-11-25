//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod error;
pub mod global_cache;
pub mod global_config;
pub mod global_flag;
pub mod unsigned_transactions;
pub mod wallet;
pub mod wallet_data;

use std::{path::PathBuf, sync::Arc};

use global_cache::GlobalCacheTable;
use global_config::GlobalConfigTable;
use global_flag::GlobalFlagTable;
use unsigned_transactions::UnsignedTransactionsTable;
use wallet::WalletsTable;

use once_cell::sync::OnceCell;
use tracing::{error, info};

use crate::consts::ROOT_DATA_DIR;

pub static DATABASE: OnceCell<Database> = OnceCell::new();

pub type Error = error::DatabaseError;

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    #[allow(dead_code)]
    pub global_flag: GlobalFlagTable,
    pub global_config: GlobalConfigTable,
    pub global_cache: GlobalCacheTable,
    pub wallets: WalletsTable,
    pub unsigned_transactions: UnsignedTransactionsTable,
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

    pub fn wallets(&self) -> WalletsTable {
        self.wallets.clone()
    }

    pub fn global_config(&self) -> GlobalConfigTable {
        self.global_config.clone()
    }

    pub fn unconfirmed_transactions(&self) -> UnsignedTransactionsTable {
        self.unsigned_transactions.clone()
    }
}

impl Database {
    pub fn global() -> &'static Database {
        DATABASE.get_or_init(|| {
            let db = get_or_create_database();

            let write_txn = db.begin_write().expect("failed to begin write transaction");
            let db = Arc::new(db);

            let wallets = WalletsTable::new(db.clone(), &write_txn);
            let global_flag = GlobalFlagTable::new(db.clone(), &write_txn);
            let global_config = GlobalConfigTable::new(db.clone(), &write_txn);
            let global_cache = GlobalCacheTable::new(db.clone(), &write_txn);
            let unsigned_transactions = UnsignedTransactionsTable::new(db.clone(), &write_txn);

            write_txn
                .commit()
                .expect("failed to commit write transaction");

            Database {
                wallets,
                global_flag,
                global_config,
                global_cache,
                unsigned_transactions,
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

#[cfg(not(test))]
fn database_location() -> PathBuf {
    ROOT_DATA_DIR.join("cove.db")
}

#[cfg(test)]
fn database_location() -> PathBuf {
    use rand::distributions::Alphanumeric;
    use rand::prelude::*;

    let mut rng = rand::thread_rng();
    let random_string: String = (0..7).map(|_| rng.sample(Alphanumeric) as char).collect();
    let cove_db = format!("cove_{}.db", random_string);

    let test_dir = ROOT_DATA_DIR.join("test");
    std::fs::create_dir_all(&test_dir).expect("failed to create test dir");

    ROOT_DATA_DIR.join(test_dir).join(cove_db)
}

#[cfg(test)]
pub fn delete_database() {
    let _ = std::fs::remove_dir(ROOT_DATA_DIR.join("test"));
    let _ = std::fs::remove_dir(ROOT_DATA_DIR.join("wallet_data"));
}
