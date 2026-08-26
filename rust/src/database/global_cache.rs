use std::sync::Arc;

use redb::TableDefinition;
use tracing::debug;

use cove_util::result_ext::ResultExt as _;

use crate::{
    app::reconcile::{Update, Updater},
    fee_client::{FeeResponse, FeeSnapshot},
    fiat::client::PriceResponse,
    network::Network,
};

use super::Error;
use cove_types::{BlockSizeLast, redb::Json};

pub const TABLE: TableDefinition<&'static str, Json<GlobalCacheData>> =
    TableDefinition::new("global_cache");

#[derive(Debug, Clone, Copy)]
pub enum GlobalCacheKey {
    Prices(PricesKey),
    Fees(FeesKey),
    BlockHeight(Network),
}

#[derive(Debug, Clone, Copy)]
pub struct PricesKey;

#[derive(Debug, Clone, Copy)]
pub struct FeesKey;

impl GlobalCacheKey {
    fn key(self) -> String {
        match self {
            Self::Prices(_) => "Prices".to_string(),
            Self::Fees(_) => "Fees".to_string(),
            Self::BlockHeight(network) => format!("BlockHeight::{network:?}"),
        }
    }
}

#[derive(Debug, Clone, derive_more::From, serde::Serialize, serde::Deserialize)]
pub enum GlobalCacheData {
    Prices(PriceResponse),
    /// Legacy fee data without a fetch timestamp. It is retained for decoding old databases but
    /// must never be treated as a current fee snapshot
    Fees(FeeResponse),
    BlockHeight(BlockSizeLast),
    /// Fee data with a wall-clock timestamp for bounded offline fallback
    FeesV2(FeeSnapshot),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee_client::{FeeFetchedAt, FeeSnapshot};

    fn test_table() -> (tempfile::TempDir, GlobalCacheTable) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db = Arc::new(
            redb::Database::create(tmp.path().join("global_cache.redb"))
                .expect("failed to create redb"),
        );
        let write_txn = db.begin_write().expect("failed to begin write transaction");
        let table = GlobalCacheTable::new(db, &write_txn);
        write_txn.commit().expect("failed to commit table creation");

        (tmp, table)
    }

    fn fees() -> FeeResponse {
        FeeResponse {
            fastest_fee: 5.0,
            half_hour_fee: 3.0,
            hour_fee: 2.0,
            economy_fee: 1.0,
            minimum_fee: 1.0,
        }
    }

    #[test]
    fn legacy_fee_data_is_stale_and_timestamped_data_round_trips() {
        let (_tmp, table) = test_table();
        let key = GlobalCacheKey::Fees(FeesKey);

        table.set(key, GlobalCacheData::Fees(fees())).expect("legacy fee data saves");
        assert!(table.get_fee_snapshot().expect("fee snapshot loads").is_none());

        let snapshot = FeeSnapshot {
            fees: fees(),
            fetched_at: FeeFetchedAt::from_unix_seconds(1_700_000_000),
        };
        table.set_fee_snapshot(snapshot).expect("timestamped fee data saves");

        assert_eq!(table.get_fee_snapshot().expect("fee snapshot loads"), Some(snapshot));
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

    pub fn get_fee_snapshot(&self) -> Result<Option<FeeSnapshot>, Error> {
        let key = GlobalCacheKey::Fees(FeesKey);
        if let Some(GlobalCacheData::FeesV2(snapshot)) = self.get(key)? {
            return Ok(Some(snapshot));
        }

        Ok(None)
    }

    pub fn set_fee_snapshot(&self, snapshot: FeeSnapshot) -> Result<(), Error> {
        let key = GlobalCacheKey::Fees(FeesKey);
        self.set(key, GlobalCacheData::FeesV2(snapshot))
    }

    pub fn get_block_height(&self, network: Network) -> Result<Option<BlockSizeLast>, Error> {
        let key = GlobalCacheKey::BlockHeight(network);
        if let Some(GlobalCacheData::BlockHeight(block_height)) = self.get(key)? {
            return Ok(Some(block_height));
        }

        Ok(None)
    }

    pub fn set_block_height(
        &self,
        network: Network,
        block_height: BlockSizeLast,
    ) -> Result<(), Error> {
        let key = GlobalCacheKey::BlockHeight(network);
        self.set(key, block_height.into())
    }
}

impl GlobalCacheTable {
    pub fn get(&self, key: GlobalCacheKey) -> Result<Option<GlobalCacheData>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

        let key = key.key();
        let value = table
            .get(key.as_str())
            .map_err_str(GlobalCacheTableError::Read)?
            .map(|value| value.value());

        Ok(value)
    }

    pub fn set(&self, key: GlobalCacheKey, value: GlobalCacheData) -> Result<(), Error> {
        debug!("set global cache: {key:?} -> {value:?}");
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

            let key = key.key();
            table.insert(key.as_str(), value).map_err_str(GlobalCacheTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}
