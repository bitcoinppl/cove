use std::{borrow::Borrow, fmt::Debug, sync::Arc};

use crate::database::{error::DatabaseError, record::Timestamps, Record};
use bip329::{AddressRecord, InputRecord, Label, Labels, OutputRecord, TransactionRecord};
use bitcoin::{address::NetworkUnchecked, Address};
use redb::{ReadOnlyTable, ReadableTable as _, ReadableTableMetadata as _, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    database::{cbor::Postcard, key::InOutIdKey},
    transaction::TxId,
};

type SerdeRecord<T> = Postcard<Record<T>>;
pub type Error = LabelDbError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum LabelDbError {
    #[error(transparent)]
    Database(#[from] DatabaseError),

    #[error("unsupported label type for saving {0}")]
    UnsupportedLabelType(String),
}

const TXN_TABLE: TableDefinition<TxId, SerdeRecord<TransactionRecord>> =
    TableDefinition::new("transaction_labels.cbor");

const ADDRESS_TABLE: TableDefinition<
    Postcard<Address<NetworkUnchecked>>,
    SerdeRecord<AddressRecord>,
> = TableDefinition::new("address_labels.cbor");

const INPUT_TABLE: TableDefinition<InOutIdKey, SerdeRecord<InputRecord>> =
    TableDefinition::new("input_records.cbor");

const OUTPUT_TABLE: TableDefinition<InOutIdKey, SerdeRecord<OutputRecord>> =
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

    pub fn number_of_labels(&self) -> Result<u64, Error> {
        let txn_table = self.read_table(TXN_TABLE)?;
        let input_table = self.read_table(INPUT_TABLE)?;
        let output_table = self.read_table(OUTPUT_TABLE)?;
        let address_table = self.read_table(ADDRESS_TABLE)?;

        let txns = txn_table.len()?;
        let inputs = input_table.len()?;
        let outputs = output_table.len()?;
        let addresses = address_table.len()?;

        Ok(txns + inputs + outputs + addresses)
    }

    // MARK: LIST

    pub fn all_labels(&self) -> Result<Labels, Error> {
        let txn_table = self.read_table(TXN_TABLE)?;
        let input_table = self.read_table(INPUT_TABLE)?;
        let output_table = self.read_table(OUTPUT_TABLE)?;
        let address_table = self.read_table(ADDRESS_TABLE)?;

        let txns = txn_table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value().item)
            .map(Label::Transaction);

        let inputs = input_table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value().item)
            .map(Label::Input);

        let outputs = output_table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value().item)
            .map(Label::Output);

        let addresses = address_table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value().item)
            .map(Label::Address);

        let labels = txns
            .chain(inputs)
            .chain(outputs)
            .chain(addresses)
            .collect::<Vec<_>>()
            .into();

        Ok(labels)
    }

    pub fn all_labels_for_txn(
        &self,
        txid: impl Borrow<bitcoin::Txid>,
    ) -> Result<Vec<Label>, Error> {
        let txid = txid.borrow();

        let txn = self.get_txn_label_record(txid)?.map(|record| record.item);

        let Some(txn) = txn else { return Ok(vec![]) };
        let inputs = self.txn_inputs_iter(txid)?;
        let outputs = self.txn_outputs_iter(txid)?;

        let labels = std::iter::once(Label::Transaction(txn))
            .chain(inputs.map(Label::Input))
            .chain(outputs.map(Label::Output))
            .collect::<Vec<Label>>();

        Ok(labels)
    }

    pub fn all_txns(&self) -> Result<Vec<TransactionRecord>, Error> {
        let table = self.read_table(TXN_TABLE)?;
        let txns = table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value().item)
            .collect();

        Ok(txns)
    }

    pub fn txn_input_records_iter(
        &self,
        txid: impl AsRef<[u8; 32]>,
    ) -> Result<impl Iterator<Item = Record<InputRecord>>, Error> {
        let table = self.read_table(INPUT_TABLE)?;

        let start_inout_id = InOutIdKey {
            id: *txid.as_ref(),
            index: 0,
        };

        let inputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value());

        Ok(inputs)
    }

    pub fn txn_output_records_iter(
        &self,
        txid: impl AsRef<[u8; 32]>,
    ) -> Result<impl Iterator<Item = Record<OutputRecord>>, Error> {
        let table = self.read_table(OUTPUT_TABLE)?;

        let start_inout_id = InOutIdKey {
            id: *txid.as_ref(),
            index: 0,
        };

        let outputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .map(|(_key, record)| record.value());

        Ok(outputs)
    }

    fn txn_inputs_iter(
        &self,
        txid: impl AsRef<[u8; 32]>,
    ) -> Result<impl Iterator<Item = InputRecord>, Error> {
        Ok(self.txn_input_records_iter(txid)?.map(|record| record.item))
    }

    fn txn_outputs_iter(
        &self,
        txid: impl AsRef<[u8; 32]>,
    ) -> Result<impl Iterator<Item = OutputRecord>, Error> {
        Ok(self
            .txn_output_records_iter(txid)?
            .map(|record| record.item))
    }

    // MARK: GET

    #[allow(dead_code)]
    fn get_label_record(&self, label: Label) -> Result<Option<Record<Label>>, Error> {
        match label {
            Label::Transaction(txn) => {
                let record = self.get_txn_label_record(txn.ref_)?;
                Ok(record.map(|record| record.into()))
            }

            Label::Address(address_record) => {
                let table = self.read_table(ADDRESS_TABLE)?;
                let record = table.get(address_record.ref_)?.map(|record| record.value());
                Ok(record.map(|record| record.into()))
            }

            Label::Input(input_record) => {
                let table = self.read_table(INPUT_TABLE)?;
                let key: InOutIdKey = input_record.ref_.into();

                let record = table.get(key)?.map(|record| record.value());
                Ok(record.map(|record| record.into()))
            }

            Label::Output(output_record) => {
                let table = self.read_table(OUTPUT_TABLE)?;
                let key: InOutIdKey = output_record.ref_.into();

                let record = table.get(key)?.map(|record| record.value());
                Ok(record.map(|record| record.into()))
            }

            // unsupported label types
            Label::ExtendedPublicKey(_) => Err(Error::UnsupportedLabelType(
                "extended public key".to_string(),
            )),

            Label::PublicKey(_) => Err(Error::UnsupportedLabelType("public key".to_string())),
        }
    }

    pub fn get_txn_label_record(
        &self,
        txid: impl Borrow<bitcoin::Txid>,
    ) -> Result<Option<Record<TransactionRecord>>, Error> {
        let txid = txid.borrow();
        let table = self.read_table(TXN_TABLE)?;
        let label = table.get(txid)?.map(|record| record.value());

        Ok(label)
    }

    // MARK: INSERT

    pub fn insert_labels(&self, labels: impl IntoIterator<Item = Label>) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        labels
            .into_iter()
            .try_for_each(|l| self.insert_label_with_write_txn(l, Timestamps::now(), &write_txn))?;

        write_txn
            .commit()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    pub fn insert_or_update_txn_label(&self, label: TransactionRecord) -> Result<(), Error> {
        let current = self.get_txn_label_record(label.ref_).unwrap_or(None);
        let label: Label = label.into();

        if let Some(current) = current {
            let mut updated = current;
            updated.timestamps.updated_at = jiff::Timestamp::now().as_second() as u64;
            let timestamps = updated.timestamps;

            self.insert_label_with_timestamps(label, timestamps)?;
            return Ok(());
        }

        self.insert_label(label)
    }

    pub fn insert_label(&self, label: impl Into<Label>) -> Result<(), Error> {
        self.insert_label_with_timestamps(label.into(), Timestamps::now())
    }

    pub fn insert_label_with_timestamps(
        &self,
        label: impl Into<Label>,
        timestamps: Timestamps,
    ) -> Result<(), Error> {
        let label: Label = label.into();
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        self.insert_label_with_write_txn(label, timestamps, &write_txn)?;

        write_txn
            .commit()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    pub fn insert_records(
        &self,
        records: impl IntoIterator<Item = Record<Label>>,
    ) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        records.into_iter().try_for_each(|record| {
            self.insert_label_with_write_txn(record.item, record.timestamps, &write_txn)
        })?;

        write_txn
            .commit()
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    fn insert_label_with_write_txn(
        &self,
        label: Label,
        timestamps: Timestamps,
        write_txn: &redb::WriteTransaction,
    ) -> Result<(), Error> {
        match label {
            Label::Transaction(txn) => {
                let mut table = write_txn.open_table(TXN_TABLE)?;
                let key: TxId = txn.ref_.into();
                let value: Record<TransactionRecord> = Record::with_timestamps(txn, timestamps);

                table.insert(key, value)?;
            }
            Label::Input(input) => {
                let mut table = write_txn.open_table(INPUT_TABLE)?;
                let key = InOutIdKey::from(&input.ref_);
                let value: Record<InputRecord> = Record::with_timestamps(input, timestamps);

                table.insert(key, value)?;
            }
            Label::Output(output) => {
                let mut table = write_txn.open_table(OUTPUT_TABLE)?;
                let key = InOutIdKey::from(&output.ref_);
                let output: Record<OutputRecord> = Record::with_timestamps(output, timestamps);

                table.insert(key, output)?;
            }
            Label::Address(address) => {
                let mut table = write_txn.open_table(ADDRESS_TABLE)?;
                let key = address.ref_.clone();
                let address: Record<AddressRecord> = Record::with_timestamps(address, timestamps);

                table.insert(key, address)?;
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
            .map_err(|error| DatabaseError::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(table)
            .map_err(|error| DatabaseError::TableAccess(error.to_string()))?;

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
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0","label":"last txn received 2 (received)"}
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:1","label":"last txn received 3 (received)"}
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2","label":"last txn received 4 (received)"}
        "#;

        let labels = Labels::try_from_str(jsonl).expect("failed to parse labels");
        let wallet_db = WalletDataDb::new_test(WalletId::preview_new());
        let db = &wallet_db.labels;

        db.insert_labels(labels).expect("failed to insert labels");

        let txn = db.all_txns().expect("failed to get all txns");
        assert_eq!(txn.len(), 1);

        let labels = db
            .all_labels_for_txn(txn[0].ref_)
            .expect("failed to get labels");

        assert_eq!(labels.len(), 5);
    }
}

impl From<redb::TransactionError> for Error {
    fn from(error: redb::TransactionError) -> Self {
        Self::Database(error.into())
    }
}

impl From<redb::TableError> for Error {
    fn from(error: redb::TableError) -> Self {
        Self::Database(error.into())
    }
}

impl From<redb::StorageError> for Error {
    fn from(error: redb::StorageError) -> Self {
        Self::Database(error.into())
    }
}
