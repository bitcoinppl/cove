pub mod network_block_height;
pub mod record;

use std::sync::Arc;

use ahash::AHashMap as HashMap;
use network_block_height::NetworkBlockHeight;
use record::HistoricalPriceRecord;
use redb::TableDefinition;

use super::Error;
use crate::{fiat::historical::HistoricalPrice, network::Network};

// Table definition with NetworkBlockHeight as key and HistoricalPriceRecord as value
pub const TABLE: TableDefinition<NetworkBlockHeight, HistoricalPriceRecord> =
    TableDefinition::new("historical_prices");

#[derive(Debug, Clone, uniffi::Object)]
pub struct HistoricalPriceTable {
    db: Arc<redb::Database>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum HistoricalPriceTableError {
    #[error("failed to save historical price {0}")]
    Save(String),

    #[error("failed to get historical price {0}")]
    Read(String),

    #[error("no record found")]
    NoRecordFound,
}

impl HistoricalPriceTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // Create table if it doesn't exist
        write_txn
            .open_table(TABLE)
            .expect("failed to create historical prices table");

        Self { db }
    }

    /// Get historical price for a specific block number
    pub fn get_price_for_block(
        &self,
        network: Network,
        block_height: u32,
    ) -> Result<Option<HistoricalPriceRecord>, Error> {
        let key = NetworkBlockHeight::new(network, block_height);

        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let value = table
            .get(key)
            .map_err(|error| HistoricalPriceTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    /// Set historical price for a specific block number using the compact record format
    pub fn set_price_for_block(
        &self,
        network: Network,
        block_number: u32,
        price: HistoricalPrice,
    ) -> Result<(), Error> {
        let key = NetworkBlockHeight::new(network, block_number);

        // convert to the more compact record format
        let price_record: HistoricalPriceRecord = price.into();

        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            table
                .insert(key, &price_record)
                .map_err(|error| HistoricalPriceTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    /// Get historical prices for all the blocks given
    /// Rreturns a map of all the blocks and an price if we have one
    pub fn get_prices_for_blocks(
        &self,
        network: Network,
        block_heights: &[u32],
    ) -> Result<HashMap<u32, Option<HistoricalPrice>>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let mut prices = HashMap::with_capacity(block_heights.len());

        for block_height in block_heights {
            let block_height = *block_height;
            let key = NetworkBlockHeight::new(network, block_height);
            let value = table
                .get(key)
                .map_err(|error| HistoricalPriceTableError::Read(error.to_string()))?
                .map(|value| value.value());

            prices.insert(block_height, value.map(HistoricalPrice::from));
        }

        Ok(prices)
    }
}
