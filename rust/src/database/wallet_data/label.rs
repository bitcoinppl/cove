use std::{fmt::Debug, sync::Arc};

use bip329::{AddressRecord, InOutId, InputRecord, OutputRecord, TransactionRecord};
use bitcoin::{address::NetworkUnchecked, Address, Txid};
use redb::{ReadOnlyTable, ReadableTable as _, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};

use crate::database::postcard::Postcard;

pub type Error = crate::database::error::DatabaseError;
const TXN_TABLE: TableDefinition<Postcard<Txid>, Postcard<TransactionRecord>> =
    TableDefinition::new("transaction_labels.json");

const ADDRESS_TABLE: TableDefinition<Postcard<Address<NetworkUnchecked>>, Postcard<AddressRecord>> =
    TableDefinition::new("address_labels.json");

const INPUT_TABLE: TableDefinition<Postcard<InOutId>, Postcard<InputRecord>> =
    TableDefinition::new("input_records.json");

const OUTPUT_TABLE: TableDefinition<Postcard<InOutId>, Postcard<OutputRecord>> =
    TableDefinition::new("output_records.json");

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelsTable {
    db: Arc<redb::Database>,
}

impl LabelsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn
            .open_table(TXN_TABLE)
            .expect("failed to transactions create table");

        write_txn
            .open_table(ADDRESS_TABLE)
            .expect("failed to create address table");

        write_txn
            .open_table(INPUT_TABLE)
            .expect("failed to create input table");

        write_txn
            .open_table(OUTPUT_TABLE)
            .expect("failed to create output table");

        Self { db }
    }

    pub fn all_txns(&self) -> Result<Vec<TransactionRecord>, Error> {
        let table = self.read_table(TXN_TABLE)?;
        let txns = table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value())
            .collect();

        Ok(txns)
    }

    fn read_table<K, V>(
        &self,
        table: TableDefinition<K, Postcard<V>>,
    ) -> Result<ReadOnlyTable<K, Postcard<V>>, Error>
    where
        K: redb::Key + Debug + Clone + Send + Sync + 'static,
        V: Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static,
    {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(table)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        Ok(table)
    }
}
