use std::sync::Arc;

use redb::TableDefinition;

use crate::{
    app::reconcile::{Update, Updater},
    fiat::client::PriceResponse,
    redb::Json,
};

use super::Error;

pub const TABLE: TableDefinition<&'static str, Json<GlobalCacheData>> =
    TableDefinition::new("global_cache");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr)]
pub enum GlobalCacheKey {
    Prices(PricesKey),
}

#[derive(Debug, Clone, Copy)]
pub struct PricesKey;

#[derive(Debug, Clone, derive_more::From, serde::Serialize, serde::Deserialize)]
pub enum GlobalCacheData {
    Prices(PriceResponse),
}

#[derive(Debug, Clone)]
pub struct GlobalCacheTable {
    db: Arc<redb::Database>,
}

impl GlobalCacheTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum GlobalCacheTableError {
    #[error("failed to save global flag: {0}")]
    Save(String),

    #[error("failed to get global flag: {0}")]
    Read(String),
}

impl GlobalCacheTable {
    pub fn get_prices(&self) -> Result<Option<PriceResponse>, Error> {
        let key = GlobalCacheKey::Prices(PricesKey);
        if let Some(GlobalCacheData::Prices(prices)) = self.get(key)? {
            return Ok(Some(prices));
        }

        Ok(None)
    }

    pub fn set_prices(&self, prices: PriceResponse) -> Result<(), Error> {
        let key = GlobalCacheKey::Prices(PricesKey);
        self.set(key, prices.into())
    }
}

impl GlobalCacheTable {
    pub fn get(&self, key: GlobalCacheKey) -> Result<Option<GlobalCacheData>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| GlobalCacheTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    pub fn set(&self, key: GlobalCacheKey, value: GlobalCacheData) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| GlobalCacheTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}
