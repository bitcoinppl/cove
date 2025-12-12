use std::sync::Arc;

use redb::TableDefinition;
use tracing::debug;

use cove_util::result_ext::ResultExt as _;

use crate::{
    app::reconcile::{Update, Updater},
    fee_client::FeeResponse,
    fiat::client::PriceResponse,
};

use super::Error;
use cove_types::redb::Json;

pub const TABLE: TableDefinition<&'static str, Json<GlobalCacheData>> =
    TableDefinition::new("global_cache");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr)]
pub enum GlobalCacheKey {
    Prices(PricesKey),
    Fees(FeesKey),
}

#[derive(Debug, Clone, Copy)]
pub struct PricesKey;

#[derive(Debug, Clone, Copy)]
pub struct FeesKey;

#[derive(Debug, Clone, derive_more::From, serde::Serialize, serde::Deserialize)]
pub enum GlobalCacheData {
    Prices(PriceResponse),
    Fees(FeeResponse),
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
#[uniffi::export(Display)]
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

    pub fn get_fees(&self) -> Result<Option<FeeResponse>, Error> {
        let key = GlobalCacheKey::Fees(FeesKey);
        if let Some(GlobalCacheData::Fees(fees)) = self.get(key)? {
            return Ok(Some(fees));
        }

        Ok(None)
    }

    pub fn set_fees(&self, fees: FeeResponse) -> Result<(), Error> {
        let key = GlobalCacheKey::Fees(FeesKey);
        self.set(key, fees.into())
    }
}

impl GlobalCacheTable {
    pub fn get(&self, key: GlobalCacheKey) -> Result<Option<GlobalCacheData>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

        let key: &'static str = key.into();
        let value =
            table.get(key).map_err_str(GlobalCacheTableError::Read)?.map(|value| value.value());

        Ok(value)
    }

    pub fn set(&self, key: GlobalCacheKey, value: GlobalCacheData) -> Result<(), Error> {
        debug!("set global cache: {key:?} -> {value:?}");
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

            let key: &'static str = key.into();
            table.insert(key, value).map_err_str(GlobalCacheTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}
