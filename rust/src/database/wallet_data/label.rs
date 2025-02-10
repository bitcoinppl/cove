use std::{borrow::Borrow, fmt::Debug, sync::Arc};

use bip329::{AddressRecord, InputRecord, Label, OutputRecord, TransactionRecord};
use bitcoin::{address::NetworkUnchecked, Address, Txid};
use redb::{ReadOnlyTable, ReadableTable as _, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    database::{in_out_id::InOutId, postcard::Postcard},
    transaction::TxId,
};

pub type Error = crate::database::error::DatabaseError;
const TXN_TABLE: TableDefinition<TxId, Postcard<TransactionRecord>> =
    TableDefinition::new("transaction_labels.json");

const ADDRESS_TABLE: TableDefinition<Postcard<Address<NetworkUnchecked>>, Postcard<AddressRecord>> =
    TableDefinition::new("address_labels.json");

const INPUT_TABLE: TableDefinition<InOutId, Postcard<InputRecord>> =
    TableDefinition::new("input_records.json");

const OUTPUT_TABLE: TableDefinition<InOutId, Postcard<OutputRecord>> =
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

    pub fn all_labels_for_txn(&self, txid: impl Borrow<Txid>) -> Result<Vec<Label>, Error> {
        let txid = txid.borrow();

        let table = self.read_table(TXN_TABLE)?;
        let txn = table.get(txid)?.map(|record| record.value());

        let Some(txn) = txn else { return Ok(vec![]) };
        let inputs = self.txn_inputs(txid)?;
        let outputs = self.txn_ouputs(txid)?;

        let mut labels = Vec::with_capacity(inputs.len() + outputs.len() + 1);
        labels.push(Label::Transaction(txn));

        for input in inputs {
            labels.push(Label::Input(input));
        }

        for output in outputs {
            labels.push(Label::Output(output));
        }

        Ok(labels)
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

    pub fn txn_inputs(&self, txid: impl AsRef<[u8; 32]>) -> Result<Vec<InputRecord>, Error> {
        let table = self.read_table(INPUT_TABLE)?;

        let start_inout_id = InOutId {
            id: *txid.as_ref(),
            index: 0,
        };

        let inputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value())
            .collect::<Vec<InputRecord>>();

        Ok(inputs)
    }

    pub fn txn_ouputs(&self, txid: impl AsRef<[u8; 32]>) -> Result<Vec<OutputRecord>, Error> {
        let table = self.read_table(OUTPUT_TABLE)?;

        let start_inout_id = InOutId {
            id: *txid.as_ref(),
            index: 0,
        };

        let outputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value())
            .collect::<Vec<OutputRecord>>();

        Ok(outputs)
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
