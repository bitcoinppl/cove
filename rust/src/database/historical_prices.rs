use std::sync::Arc;

use redb::{TableDefinition, TypeName};
use serde::{Deserialize, Serialize};

use crate::{
    app::reconcile::{Update, Updater},
    database::historical_price_record::HistoricalPriceRecord,
    fiat::historical::HistoricalPrice,
    redb::Json,
};

use super::Error;

// Define a custom type that implements redb::TypeName for BlockNumber
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockNumber(pub u32);

impl TypeName for BlockNumber {
    fn type_name() -> &'static str {
        "block_number"
    }
}

// Table definition with BlockNumber as key and HistoricalPriceRecord as value
pub const TABLE: TableDefinition<BlockNumber, Json<HistoricalPriceRecord>> =
    TableDefinition::new("historical_prices");

#[derive(Debug, Clone)]
pub struct HistoricalPricesTable {
    db: Arc<redb::Database>,
}

impl HistoricalPricesTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // Create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create historical prices table");

        Self { db }
    }

    /// Get historical price for a specific block number
    pub fn get_price_by_block(&self, block_number: u32) -> Result<Option<HistoricalPriceRecord>, Error> {
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
            .map_err(|error| Error::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    /// Set historical price for a specific block number using the compact record format
    pub fn set_price_for_block(&self, block_number: u32, price: HistoricalPrice) -> Result<(), Error> {
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
                .insert(key, price_record)
                .map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
    
    /// Set historical price record directly
    pub fn set_price_record_for_block(&self, block_number: u32, price_record: HistoricalPriceRecord) -> Result<(), Error> {
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
                .insert(key, price_record)
                .map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    /// Get all historical prices
    pub fn get_all_prices(&self) -> Result<Vec<(BlockNumber, HistoricalPriceRecord)>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let mut prices = Vec::new();
        for item in table.iter().map_err(|error| Error::Read(error.to_string()))? {
            let (key, value) = item.map_err(|error| Error::Read(error.to_string()))?;
            prices.push((key, value.value()));
        }

        Ok(prices)
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
                .map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}