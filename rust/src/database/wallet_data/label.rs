use std::{borrow::Borrow, fmt::Debug, sync::Arc};

use bip329::{AddressRecord, InputRecord, Label, OutputRecord, TransactionRecord};
use bitcoin::{address::NetworkUnchecked, Address, Txid};
use redb::{ReadOnlyTable, ReadableTable as _, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    database::{cbor::Postcard, in_out_id::InOutId},
    transaction::TxId,
};

pub type Error = crate::database::error::DatabaseError;
const TXN_TABLE: TableDefinition<TxId, Postcard<TransactionRecord>> =
    TableDefinition::new("transaction_labels.cbor");

const ADDRESS_TABLE: TableDefinition<Postcard<Address<NetworkUnchecked>>, Postcard<AddressRecord>> =
    TableDefinition::new("address_labels.cbor");

const INPUT_TABLE: TableDefinition<InOutId, Postcard<InputRecord>> =
    TableDefinition::new("input_records.cbor");

const OUTPUT_TABLE: TableDefinition<InOutId, Postcard<OutputRecord>> =
    TableDefinition::new("output_records.cbor");

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelsTable {
    db: Arc<redb::Database>,
}

impl LabelsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create tables  if it doesn't exist
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
        drop(table);

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

    pub fn insert_labels(&self, labels: Vec<Label>) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        labels
            .into_iter()
            .map(|l| self.insert_label_with_write(l, &write_txn))
            .collect::<Result<(), Error>>()?;

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    pub fn insert_label_with_write(
        &self,
        label: Label,
        write_txn: &redb::WriteTransaction,
    ) -> Result<(), Error> {
        match label {
            Label::Transaction(txn) => {
                let mut table = write_txn.open_table(TXN_TABLE)?;
                table.insert(&txn.ref_.clone(), txn)?;
            }
            Label::Input(input) => {
                let mut table = write_txn.open_table(INPUT_TABLE)?;
                let key = InOutId::from(&input.ref_);
                table.insert(key, input)?;
            }
            Label::Output(output) => {
                let mut table = write_txn.open_table(OUTPUT_TABLE)?;
                let key = InOutId::from(&output.ref_);
                table.insert(key, output)?;
            }
            Label::Address(address) => {
                let mut table = write_txn.open_table(ADDRESS_TABLE)?;
                table.insert(&address.ref_.clone(), address)?;
            }
            _ => {
                tracing::warn!("unsupported label type for saving {label:?}");
            }
        }

        Ok(())
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

#[cfg(test)]
mod tests {
    use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletId};
    use bip329::Labels;

    #[test]
    fn test_all_labels_for_txn() {
        let jsonl = r#"
            {"type":"tx","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290","label":"last txn received","origin":"pkh([73c5da0a/44h/0h/0h])"}
            {"type":"input","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0","label":"last txn received 1 (input)"}
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:1","label":"last txn received 2 (received)", "spendable": true}
        "#;

        // let jsonl = r#"
        //     {"type":"tx","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290","label":"last txn received","origin":"pkh([73c5da0a/44h/0h/0h])"}
        //     {"type":"input","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0","label":"last txn received 1 (input)"}
        //     {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0","label":"last txn received 2 (received)"}
        //     {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:1","label":"last txn received 3 (received)"}
        //     {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2","label":"last txn received 4 (received)"}
        // "#;

        let labels = Labels::try_from_str(jsonl).expect("failed to parse labels");
        let wallet_db = WalletDataDb::new_test(WalletId::preview_new());
        let db = &wallet_db.labels;

        println!("{labels:?}");

        db.insert_labels(labels.into())
            .expect("failed to insert labels");

        let txn = db.all_txns().expect("failed to get all txns");
        assert_eq!(txn.len(), 1);

        let labels = db
            .all_labels_for_txn(txn[0].ref_.clone())
            .expect("failed to get labels");

        println!("{labels:?}");

        // assert_eq!(labels.len(), 4);
    }
}
