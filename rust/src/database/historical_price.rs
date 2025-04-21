pub mod record;

use std::sync::Arc;

use record::HistoricalPriceRecord;
use redb::TableDefinition;
use serde::{Deserialize, Serialize};

use crate::{
    app::reconcile::{Update, Updater},
    fiat::historical::HistoricalPrice,
};

use super::Error;

// Define a custom type that implements redb::TypeName for BlockNumber
// TODO: Move this to cove-types later
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockNumber(pub u32);

// Table definition with BlockNumber as key and HistoricalPriceRecord as value
pub const TABLE: TableDefinition<BlockNumber, HistoricalPriceRecord> =
    TableDefinition::new("historical_prices.bin");

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
        block_number: u32,
    ) -> Result<Option<HistoricalPriceRecord>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let key = BlockNumber(block_number);
        let value = table
            .get(key)
            .map_err(|error| HistoricalPriceTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    /// Set historical price for a specific block number using the compact record format
    pub fn set_price_for_block(
        &self,
        block_number: u32,
        price: HistoricalPrice,
    ) -> Result<(), Error> {
        // Convert to the more compact record format
        let price_record: HistoricalPriceRecord = price.into();

        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key = BlockNumber(block_number);
            table
                .insert(key, &price_record)
                .map_err(|error| HistoricalPriceTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    /// Delete historical price for a specific block number
    pub fn delete_price_for_block(&self, block_number: u32) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key = BlockNumber(block_number);
            table
                .remove(key)
                .map_err(|error| HistoricalPriceTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}
