//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod cbor;
pub mod error;
pub mod global_cache;
pub mod global_config;
pub mod global_flag;
pub mod historical_price;
pub mod key;
pub mod macros;
pub mod record;
pub mod unsigned_transactions;
pub mod wallet;
pub mod wallet_data;

use std::{path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use global_cache::GlobalCacheTable;
use global_config::GlobalConfigTable;
use global_flag::GlobalFlagTable;
use historical_price::HistoricalPriceTable;
use uniffi::custom_newtype;
use unsigned_transactions::UnsignedTransactionsTable;
use wallet::WalletsTable;

use once_cell::sync::OnceCell;
use tracing::{error, info};

use cove_common::consts::ROOT_DATA_DIR;

pub static DATABASE: OnceCell<ArcSwap<Database>> = OnceCell::new();

pub type Error = error::DatabaseError;
pub type Record<T> = record::Record<T>;

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    #[allow(dead_code, unused)]
    pub global_flag: GlobalFlagTable,
    pub global_config: GlobalConfigTable,
    pub global_cache: GlobalCacheTable,
    pub wallets: WalletsTable,
    pub unsigned_transactions: UnsignedTransactionsTable,
    pub historical_prices: HistoricalPriceTable,
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

    pub fn global_flag(&self) -> GlobalFlagTable {
        self.global_flag.clone()
    }

    pub fn unsigned_transactions(&self) -> UnsignedTransactionsTable {
        self.unsigned_transactions.clone()
    }

    pub fn historical_prices(&self) -> HistoricalPriceTable {
        self.historical_prices.clone()
    }

    pub fn dangerous_reset_all_data(&self) {
        if let Err(error) = std::fs::remove_file(database_location()) {
            error!("unable to delete database cove_main error: {error}");
            return;
        }

        DATABASE.get().expect("database not initialized").swap(Arc::new(Self::init()));
    }
}

impl Database {
    pub fn global() -> Arc<Self> {
        let db = DATABASE.get_or_init(|| ArcSwap::new(Arc::new(Self::init()))).load();

        Arc::clone(&db)
    }

    fn init() -> Self {
        let main_db = get_or_create_main_database();
        let main_db_arc = Arc::new(main_db);

        let write_txn = main_db_arc.begin_write().expect("failed to begin write transaction");

        let wallets = WalletsTable::new(main_db_arc.clone(), &write_txn);
        let global_flag = GlobalFlagTable::new(main_db_arc.clone(), &write_txn);
        let global_config = GlobalConfigTable::new(main_db_arc.clone(), &write_txn);
        let global_cache = GlobalCacheTable::new(main_db_arc.clone(), &write_txn);
        let unsigned_transactions = UnsignedTransactionsTable::new(main_db_arc.clone(), &write_txn);
        let historical_prices = HistoricalPriceTable::new(main_db_arc.clone(), &write_txn);

        write_txn.commit().expect("failed to commit write transaction");

        Self {
            global_flag,
            global_config,
            global_cache,
            wallets,
            unsigned_transactions,
            historical_prices,
        }
    }
}

fn get_or_create_main_database() -> redb::Database {
    let location = database_location();
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
    }

    info!("Creating a new database, at {}", database_location.display());

    redb::Database::create(&database_location).expect("failed to create database")
}

#[cfg(not(test))]
fn database_location() -> PathBuf {
    ROOT_DATA_DIR.join("cove.db")
}

#[cfg(test)]
fn database_location() -> PathBuf {
    use rand::distr::Alphanumeric;
    use rand::prelude::*;

    let mut rng = rand::rng();
    let random_string: String = (0..7).map(|_| rng.sample(Alphanumeric) as char).collect();
    let cove_db = format!("cove_{random_string}.db");

    let test_dir = ROOT_DATA_DIR.join("test");
    std::fs::create_dir_all(&test_dir).expect("failed to create test dir");

    ROOT_DATA_DIR.join(test_dir).join(cove_db)
}

#[cfg(test)]
pub fn delete_database() {
    let _ = std::fs::remove_dir(ROOT_DATA_DIR.join("test"));
    let _ = std::fs::remove_dir(ROOT_DATA_DIR.join("wallet_data"));
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum InsertOrUpdate {
    Insert(Timestamp),
    Update(Timestamp),
}

#[derive(Debug, Clone, Copy, derive_more::From, derive_more::AsRef, derive_more::Into)]
pub struct Timestamp(u64);
custom_newtype!(Timestamp, u64);
