use std::{borrow::Borrow, collections::HashSet, fmt::Debug, sync::Arc};

use crate::database::{Record, error::DatabaseError, record::Timestamps};
use bip329::{
    AddressRecord, InputRecord, Label, Labels, OutputRecord, ParsedLabels, TransactionRecord,
};
use bitcoin::{Address, address::NetworkUnchecked};
use cove_util::result_ext::ResultExt as _;
use redb::{ReadOnlyTable, ReadableTable as _, ReadableTableMetadata as _, TableDefinition};
use serde::{Serialize, de::DeserializeOwned};

use crate::database::{cbor::Cbor, key::OutPointKey};
use crate::transaction::TxId;

type SerdeRecord<T> = Cbor<Record<T>>;
pub type Error = LabelDbError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum LabelDbError {
    #[error(transparent)]
    Database(#[from] DatabaseError),

    #[error("unsupported label type for saving {0}")]
    UnsupportedLabelType(String),
}

pub(crate) const TXN_TABLE: TableDefinition<TxId, SerdeRecord<TransactionRecord>> =
    TableDefinition::new("transaction_labels.cbor");

pub(crate) const ADDRESS_TABLE: TableDefinition<
    Cbor<Address<NetworkUnchecked>>,
    SerdeRecord<AddressRecord>,
> = TableDefinition::new("address_labels.cbor");

pub(crate) const INPUT_TABLE: TableDefinition<OutPointKey, SerdeRecord<InputRecord>> =
    TableDefinition::new("input_records_v2.cbor");

pub(crate) const OUTPUT_TABLE: TableDefinition<OutPointKey, SerdeRecord<OutputRecord>> =
    TableDefinition::new("output_records_v2.cbor");

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelsTable {
    db: Arc<redb::Database>,
}

impl LabelsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create tables  if it doesn't exist
        write_txn.open_table(TXN_TABLE).expect("failed to transactions create table");

        write_txn.open_table(ADDRESS_TABLE).expect("failed to create address table");

        write_txn.open_table(INPUT_TABLE).expect("failed to create input table");

        write_txn.open_table(OUTPUT_TABLE).expect("failed to create output table");

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

        let labels = txns.chain(inputs).chain(outputs).chain(addresses).collect::<Vec<_>>().into();

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

        let txid = *txid.as_ref();
        let start_inout_id = OutPointKey { id: txid, index: 0 };

        let inputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .take_while(move |(key, _record)| key.value().id == txid)
            .map(|(_key, record)| record.value());

        Ok(inputs)
    }

    pub fn txn_output_records_iter(
        &self,
        txid: impl AsRef<[u8; 32]>,
    ) -> Result<impl Iterator<Item = Record<OutputRecord>>, Error> {
        let table = self.read_table(OUTPUT_TABLE)?;

        let txid = *txid.as_ref();
        let start_inout_id = OutPointKey { id: txid, index: 0 };

        let outputs = table
            .range(start_inout_id..)?
            .filter_map(Result::ok)
            .take_while(move |(key, _record)| key.value().id == txid)
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
        Ok(self.txn_output_records_iter(txid)?.map(|record| record.item))
    }

    // MARK: GET

    pub fn get_txn_label_record(
        &self,
        txid: impl Borrow<bitcoin::Txid>,
    ) -> Result<Option<Record<TransactionRecord>>, Error> {
        let txid = txid.borrow();
        let table = self.read_table(TXN_TABLE)?;
        let label = table.get(txid)?.map(|record| record.value());

        Ok(label)
    }

    pub fn get_address_record(
        &self,
        address: impl Borrow<Address<NetworkUnchecked>>,
    ) -> Result<Option<Record<AddressRecord>>, Error> {
        let address = address.borrow();
        let table = self.read_table(ADDRESS_TABLE)?;
        let label = table.get(address)?.map(|record| record.value());

        Ok(label)
    }

    pub fn get_output_record(
        &self,
        outpoint: impl Borrow<bitcoin::OutPoint>,
    ) -> Result<Option<Record<OutputRecord>>, Error> {
        let outpoint = outpoint.borrow();
        let table = self.read_table(OUTPUT_TABLE)?;
        let label = table.get(OutPointKey::from(outpoint))?.map(|record| record.value());

        Ok(label)
    }

    pub fn locked_output_outpoints(&self) -> Result<HashSet<bitcoin::OutPoint>, Error> {
        let table = self.read_table(OUTPUT_TABLE)?;
        let mut outpoints = HashSet::new();

        for row in table.iter()? {
            let (_key, record) = row?;
            let record = record.value().item;
            if !record.spendable {
                outpoints.insert(record.ref_);
            }
        }

        Ok(outpoints)
    }

    // MARK: INSERT

    pub fn insert_imported_labels(&self, parsed: ParsedLabels) -> Result<(), Error> {
        let spendable_by_outpoint = parsed
            .output_spendable
            .into_iter()
            .map(|field| (field.ref_, field.value))
            .collect::<std::collections::HashMap<_, _>>();

        let labels = parsed
            .labels
            .into_vec()
            .into_iter()
            .map(|label| {
                let Label::Output(mut output) = label else { return Ok(label) };

                match spendable_by_outpoint
                    .get(&output.ref_)
                    .and_then(|field| field.explicit_value())
                {
                    Some(spendable) => output.spendable = spendable,
                    None => {
                        if let Some(current) = self.get_output_record(output.ref_)? {
                            output.spendable = current.item.spendable;
                        }
                    }
                }

                Ok(Label::Output(output))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        self.insert_labels(labels)
    }

    pub fn insert_labels(&self, labels: impl IntoIterator<Item = Label>) -> Result<(), Error> {
        self.insert_labels_with_timestamps(labels, Timestamps::now())
    }

    pub fn insert_labels_with_timestamps(
        &self,
        labels: impl IntoIterator<Item = Label>,
        timestamp: Timestamps,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        labels
            .into_iter()
            .try_for_each(|l| self.insert_label_with_write_txn(l, timestamp, &write_txn))?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    pub fn insert_or_update_txn_label(&self, label: TransactionRecord) -> Result<(), Error> {
        let current = self.get_txn_label_record(label.ref_).unwrap_or(None);
        let label: Label = label.into();

        if let Some(current) = current {
            let mut updated = current;
            updated.timestamps.updated_at = jiff::Timestamp::now().as_second().cast_unsigned();
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
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        self.insert_label_with_write_txn(label, timestamps, &write_txn)?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    pub fn insert_records(
        &self,
        records: impl IntoIterator<Item = Record<Label>>,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        records.into_iter().try_for_each(|record| {
            self.insert_label_with_write_txn(record.item, record.timestamps, &write_txn)
        })?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    pub fn delete_labels_and_insert_records(
        &self,
        labels: impl IntoIterator<Item = Label>,
        records: impl IntoIterator<Item = Record<Label>>,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        labels
            .into_iter()
            .try_for_each(|label| self.delete_label_with_write_txn(label, &write_txn))?;
        records.into_iter().try_for_each(|record| {
            self.insert_label_with_write_txn(record.item, record.timestamps, &write_txn)
        })?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    pub fn set_output_spendability(
        &self,
        outpoint: bitcoin::OutPoint,
        spendable: bool,
    ) -> Result<(), Error> {
        self.set_output_spendability_for_outpoints([outpoint], spendable)
    }

    pub fn set_output_spendability_for_outpoints(
        &self,
        outpoints: impl IntoIterator<Item = bitcoin::OutPoint>,
        spendable: bool,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;
        let now = jiff::Timestamp::now().as_second().cast_unsigned();

        {
            let mut table = write_txn.open_table(OUTPUT_TABLE)?;

            for outpoint in outpoints {
                let key = OutPointKey::from(outpoint);
                let current = table.get(key.clone())?.map(|record| record.value());

                match (current, spendable) {
                    (Some(mut current), true) if current.item.label.is_some() => {
                        current.item.spendable = true;
                        current.timestamps.updated_at = now;
                        table.insert(key, current)?;
                    }
                    (Some(_current), true) => {
                        table.remove(key)?;
                    }
                    (Some(mut current), false) => {
                        current.item.spendable = false;
                        current.timestamps.updated_at = now;
                        table.insert(key, current)?;
                    }
                    (None, true) => {}
                    (None, false) => {
                        let record = OutputRecord { ref_: outpoint, label: None, spendable: false };
                        let record = Record::with_timestamps(record, Timestamps::new(now, now));
                        table.insert(key, record)?;
                    }
                }
            }
        }

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

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
                let key = OutPointKey::from(&input.ref_);
                let value: Record<InputRecord> = Record::with_timestamps(input, timestamps);

                table.insert(key, value)?;
            }
            Label::Output(output) => {
                let mut table = write_txn.open_table(OUTPUT_TABLE)?;
                let key = OutPointKey::from(&output.ref_);
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

    // MARK: DELETE
    pub fn delete_labels(&self, labels: impl IntoIterator<Item = Label>) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        labels.into_iter().try_for_each(|l| self.delete_label_with_write_txn(l, &write_txn))?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    pub fn delete_labels_and_insert_records(
        &self,
        labels: impl IntoIterator<Item = Label>,
        records: impl IntoIterator<Item = Record<Label>>,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        labels.into_iter().try_for_each(|l| self.delete_label_with_write_txn(l, &write_txn))?;
        records.into_iter().try_for_each(|record| {
            self.insert_label_with_write_txn(record.item, record.timestamps, &write_txn)
        })?;

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;

        Ok(())
    }

    fn delete_label_with_write_txn(
        &self,
        label: Label,
        write_txn: &redb::WriteTransaction,
    ) -> Result<(), Error> {
        match label {
            Label::Transaction(txn) => {
                let key: TxId = txn.ref_.into();
                let mut table = write_txn.open_table(TXN_TABLE)?;
                table.remove(key)?;
            }
            Label::Input(input) => {
                let key = OutPointKey::from(&input.ref_);
                let mut table = write_txn.open_table(INPUT_TABLE)?;
                table.remove(key)?;
            }
            Label::Output(output) => {
                let key = OutPointKey::from(&output.ref_);
                let mut table = write_txn.open_table(OUTPUT_TABLE)?;
                table.remove(key)?;
            }
            Label::Address(address) => {
                let key = address.ref_.clone();
                let mut table = write_txn.open_table(ADDRESS_TABLE)?;
                table.remove(key)?;
            }
            _ => {
                tracing::warn!("unsupported label type for deleting {label:?}");
            }
        }

        Ok(())
    }

    fn read_table<K, V>(
        &self,
        table: TableDefinition<K, Cbor<V>>,
    ) -> Result<ReadOnlyTable<K, Cbor<V>>, Error>
    where
        K: redb::Key + Debug + Clone + Send + Sync + 'static,
        V: Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static,
    {
        let read_txn = self.db.begin_read().map_err_str(DatabaseError::DatabaseAccess)?;

        let table = read_txn.open_table(table).map_err_str(DatabaseError::TableAccess)?;

        Ok(table)
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

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use crate::{
        database::wallet_data::test_support::new_test_wallet_data_db, wallet::metadata::WalletId,
    };
    use bip329::Labels;
    use bitcoin::OutPoint;

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
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(labels).expect("failed to insert labels");

        let txn = db.all_txns().expect("failed to get all txns");
        assert_eq!(txn.len(), 1);

        let labels = db.all_labels_for_txn(txn[0].ref_).expect("failed to get labels");

        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn txn_record_iters_are_bounded_to_requested_txid() {
        let first_txid = "0000000000000000000000000000000000000000000000000000000000000001";
        let second_txid = "0000000000000000000000000000000000000000000000000000000000000002";
        let jsonl = format!(
            r#"
            {{"type":"tx","ref":"{first_txid}","label":"first"}}
            {{"type":"output","ref":"{first_txid}:0","label":"first output"}}
            {{"type":"input","ref":"{first_txid}:0","label":"first input"}}
            {{"type":"output","ref":"{second_txid}:0","label":"second output"}}
            {{"type":"input","ref":"{second_txid}:0","label":"second input"}}
        "#
        );

        let labels = Labels::try_from_str(&jsonl).expect("failed to parse labels");
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(labels).expect("failed to insert labels");

        let txid = bitcoin::Txid::from_str(first_txid).expect("failed to parse txid");
        let inputs =
            db.txn_input_records_iter(txid).expect("failed to get inputs").collect::<Vec<_>>();
        let outputs =
            db.txn_output_records_iter(txid).expect("failed to get outputs").collect::<Vec<_>>();

        assert_eq!(inputs.len(), 1);
        assert_eq!(outputs.len(), 1);
        assert_eq!(inputs[0].item.ref_.txid, txid);
        assert_eq!(outputs[0].item.ref_.txid, txid);
    }

    #[test]
    fn imported_output_with_omitted_spendable_preserves_current_spendability() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let existing = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"locked","spendable":false}"#;
        let imported = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"renamed"}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(imported).expect("failed to parse imported labels"),
        )
        .expect("failed to insert imported labels");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, Some("renamed".to_string()));
        assert!(!record.item.spendable);
    }

    #[test]
    fn imported_explicit_output_spendable_overrides_current_spendability() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let existing = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"locked","spendable":false}"#;
        let imported = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"unlocked","spendable":true}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(imported).expect("failed to parse imported labels"),
        )
        .expect("failed to insert imported labels");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, Some("unlocked".to_string()));
        assert!(record.item.spendable);
    }

    #[test]
    fn imported_explicit_locked_output_locks_current_output() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let existing = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"unlocked","spendable":true}"#;
        let imported = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"locked","spendable":false}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(imported).expect("failed to parse imported labels"),
        )
        .expect("failed to insert imported labels");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, Some("locked".to_string()));
        assert!(!record.item.spendable);
        assert!(
            db.locked_output_outpoints().expect("failed to get locked outputs").contains(&outpoint)
        );
    }

    #[test]
    fn setting_output_unspendable_creates_lock_only_record() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.set_output_spendability(outpoint, false).expect("failed to lock output");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");
        let locked = db.locked_output_outpoints().expect("failed to get locked outputs");

        assert_eq!(record.item.label, None);
        assert!(!record.item.spendable);
        assert!(locked.contains(&outpoint));
    }

    #[test]
    fn export_includes_lock_only_output_record() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.set_output_spendability(outpoint, false).expect("failed to lock output");

        let exported = db.all_labels().expect("failed to load labels").export().unwrap();

        assert!(exported.contains(r#""type":"output""#));
        assert!(exported.contains(
            r#""ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1""#
        ));
        assert!(exported.contains(r#""spendable":false"#));
    }

    #[test]
    fn unlocking_lock_only_record_removes_output_record() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.set_output_spendability(outpoint, false).expect("failed to lock output");
        db.set_output_spendability(outpoint, true).expect("failed to unlock output");

        assert!(db.get_output_record(outpoint).expect("failed to get output record").is_none());
        assert!(db.locked_output_outpoints().expect("failed to get locked outputs").is_empty());
    }

    #[test]
    fn unlocking_labeled_output_preserves_label() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let existing = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"keep me","spendable":false}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.set_output_spendability(outpoint, true).expect("failed to unlock output");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, Some("keep me".to_string()));
        assert!(record.item.spendable);
    }

    #[test]
    fn bulk_spendability_update_only_changes_requested_outpoints() {
        let requested = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse requested outpoint");
        let untouched = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:2",
        )
        .expect("failed to parse untouched outpoint");
        let existing = r#"
            {"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"requested","spendable":true}
            {"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:2","label":"untouched","spendable":true}
        "#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.set_output_spendability_for_outpoints([requested], false)
            .expect("failed to lock requested output");

        let requested = db
            .get_output_record(requested)
            .expect("failed to get requested output")
            .expect("missing requested output")
            .item;
        let untouched = db
            .get_output_record(untouched)
            .expect("failed to get untouched output")
            .expect("missing untouched output")
            .item;

        assert!(!requested.spendable);
        assert!(untouched.spendable);
    }
}
