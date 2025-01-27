//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod error;
pub mod global_cache;
pub mod global_config;
pub mod global_flag;
pub mod macros;
pub mod unsigned_transactions;
pub mod wallet;
pub mod wallet_data;

use std::{path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use global_cache::GlobalCacheTable;
use global_config::GlobalConfigTable;
use global_flag::GlobalFlagTable;
use unsigned_transactions::UnsignedTransactionsTable;
use wallet::WalletsTable;

use once_cell::sync::OnceCell;
use tracing::{error, info};

use crate::consts::ROOT_DATA_DIR;

pub static DATABASE: OnceCell<ArcSwap<Database>> = OnceCell::new();

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

#[uniffi::export]
impl Database {
    #[uniffi::constructor(name = "new")]
    pub fn new() -> Arc<Self> {
        Self::global().clone()
    }

    pub fn wallets(&self) -> WalletsTable {
        self.wallets.clone()
    }

    pub fn global_config(&self) -> GlobalConfigTable {
        self.global_config.clone()
    }

    pub fn unsigned_transactions(&self) -> UnsignedTransactionsTable {
        self.unsigned_transactions.clone()
    }

    pub fn dangerous_reset_all_data(&self) {
        let dbs = [
            (database_location(), "cove_main"),
            (decoy_database_location(), "cove_decoy"),
        ];

        for (location, name) in dbs {
            if let Err(error) = std::fs::remove_file(&location) {
                error!("unable to delete database {name} error: {error}");
                return;
            }
        }

        DATABASE
            .get()
            .expect("database not initialized")
            .swap(Self::init_main());
    }
}

impl Database {
    pub fn global() -> Arc<Database> {
        let db = DATABASE
            .get_or_init(|| ArcSwap::new(Self::init_main()))
            .load();

        Arc::clone(&db)
    }

    pub fn switch_to_main_mode() -> Arc<Database> {
        let db = Self::switch_to_mode(Self::init_main);
        db.global_config
            .set_main_mode()
            .expect("failed to set main mode");

        db
    }

    pub fn switch_to_decoy_mode() -> Arc<Database> {
        let db = Self::switch_to_mode(Self::init_decoy);
        db.global_config
            .set_decoy_mode()
            .expect("failed to set decoy mode");

        db
    }

    fn switch_to_mode(init_fn: fn() -> Arc<Database>) -> Arc<Database> {
        if let Some(db) = DATABASE.get() {
            db.swap(init_fn());
        } else {
            DATABASE
                .set(ArcSwap::new(init_fn()))
                .expect("failed to set database");
        }

        let db = DATABASE.get().expect("already checked").load();
        Arc::clone(&db)
    }

    fn init_main() -> Arc<Database> {
        let db = Self::do_init();
        Arc::new(db)
    }

    fn init_decoy() -> Arc<Database> {
        let mut db = Self::do_init();

        let decoy_db = get_or_create_decoy_database();
        let write_txn = decoy_db
            .begin_write()
            .expect("failed to begin write transaction");

        let wallets = WalletsTable::new(Arc::new(decoy_db), &write_txn);
        db.wallets = wallets;

        Arc::new(db)
    }

    fn do_init() -> Database {
        let db = get_or_create_main_database();

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
    }
}

fn get_or_create_main_database() -> redb::Database {
    let location = database_location();
    get_or_create_database_with_location(location)
}

fn get_or_create_decoy_database() -> redb::Database {
    let location = decoy_database_location();
    get_or_create_database_with_location(location)
}

fn get_or_create_database_with_location(database_location: PathBuf) -> redb::Database {
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

fn decoy_database_location() -> PathBuf {
    ROOT_DATA_DIR.join("cove_decoy.db")
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
