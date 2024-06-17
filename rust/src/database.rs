//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use log::{error, info};
use once_cell::sync::OnceCell;
use redb::{Error, ReadableTable, TableDefinition};

pub static DATABASE: OnceCell<Database> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    pub db: Arc<redb::Database>,
}

pub fn global() {
    Database::global();
}

impl Database {
    pub fn global() -> &'static Database {
        DATABASE.get_or_init(|| {
            let db = get_or_create_database();
            Database { db: Arc::new(db) }
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
    let parent = dirs::home_dir()
        .expect("failed to get home directory")
        .join("data");

    if !parent.exists() {
        std::fs::create_dir_all(&parent).expect("failed to create data directory");
    }

    parent.join("cove.db")
}
