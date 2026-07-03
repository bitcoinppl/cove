use std::{borrow::Borrow, collections::HashSet, fmt::Debug, sync::Arc};

use crate::database::{Record, error::DatabaseError, record::Timestamps};
use bip329::{
    AddressRecord, InputRecord, Label, Labels, OutputRecord, ParsedLabels, TransactionRecord,
};
use bitcoin::{Address, address::NetworkUnchecked};
use cove_util::result_ext::ResultExt as _;
use redb::{ReadOnlyTable, ReadableTable as _, TableDefinition};
use serde::{Serialize, de::DeserializeOwned};

use crate::database::{cbor::Cbor, key::OutPointKey};
use crate::transaction::TxId;

type SerdeRecord<T> = Cbor<Record<T>>;
pub type Error = LabelDbError;

enum ImportedLabelAction {
    Insert(Label),
    Ignore,
}

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
        let txns = self.count_meaningful_labels(TXN_TABLE, Label::Transaction)?;
        let inputs = self.count_meaningful_labels(INPUT_TABLE, Label::Input)?;
        let outputs = self.count_meaningful_labels(OUTPUT_TABLE, Label::Output)?;
        let addresses = self.count_meaningful_labels(ADDRESS_TABLE, Label::Address)?;

        Ok((txns + inputs + outputs + addresses) as u64)
    }

    pub fn has_labels(&self) -> Result<bool, Error> {
        if self.has_meaningful_label(TXN_TABLE, Label::Transaction)? {
            return Ok(true);
        }

        if self.has_meaningful_label(INPUT_TABLE, Label::Input)? {
            return Ok(true);
        }

        if self.has_meaningful_label(OUTPUT_TABLE, Label::Output)? {
            return Ok(true);
        }

        self.has_meaningful_label(ADDRESS_TABLE, Label::Address)
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
            .filter(is_meaningful_export_label)
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
            if !record.spendable() {
                outpoints.insert(record.ref_);
            }
        }

        Ok(outpoints)
    }

    // MARK: INSERT

    pub fn insert_imported_labels(&self, parsed: ParsedLabels) -> Result<(), Error> {
        let timestamps = Timestamps::now();
        let mut records_to_insert = Vec::new();

        for label in parsed.labels.into_vec() {
            match self.prepare_imported_label(label)? {
                ImportedLabelAction::Insert(label) => {
                    records_to_insert.push(Record::with_timestamps(label, timestamps));
                }
                ImportedLabelAction::Ignore => {}
            }
        }

        self.insert_records(records_to_insert)
    }

    fn prepare_imported_label(&self, label: Label) -> Result<ImportedLabelAction, Error> {
        let Label::Output(output) = label else {
            if has_non_empty_label(label.label()) {
                return Ok(ImportedLabelAction::Insert(label));
            }

            return Ok(ImportedLabelAction::Ignore);
        };

        self.prepare_imported_output_label(output)
    }

    fn prepare_imported_output_label(
        &self,
        mut output: OutputRecord,
    ) -> Result<ImportedLabelAction, Error> {
        let current = self.get_output_record(output.ref_)?;

        if has_non_empty_label(output.label.as_deref()) {
            if output.spendable.is_none()
                && let Some(current) = current
            {
                output.spendable = current.item.spendable;
            }

            return Ok(ImportedLabelAction::Insert(Label::Output(output)));
        }

        let Some(spendable) = output.spendable else {
            return Ok(ImportedLabelAction::Ignore);
        };

        if !spendable {
            if let Some(current) = current {
                output.label = current.item.label;
            }
            output.spendable = Some(false);

            return Ok(ImportedLabelAction::Insert(Label::Output(output)));
        }

        let Some(mut current) = current else {
            return Ok(ImportedLabelAction::Ignore);
        };

        if current.item.spendable == Some(false) {
            current.item.spendable = Some(true);

            return Ok(ImportedLabelAction::Insert(Label::Output(current.item)));
        }

        Ok(ImportedLabelAction::Ignore)
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

            outpoints.into_iter().try_for_each(|outpoint| -> Result<(), Error> {
                let key = OutPointKey::from(outpoint);
                let Some(mut current) = table.get(key.clone())?.map(|record| record.value()) else {
                    if spendable {
                        return Ok(());
                    }

                    let record =
                        OutputRecord { ref_: outpoint, label: None, spendable: Some(false) };
                    let record = Record::with_timestamps(record, Timestamps::new(now, now));
                    table.insert(key, record)?;

                    return Ok(());
                };

                current.item.spendable = Some(spendable);
                current.timestamps.updated_at = now;
                table.insert(key, current)?;

                Ok(())
            })?;
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

    fn count_meaningful_labels<K, T>(
        &self,
        table: TableDefinition<K, SerdeRecord<T>>,
        to_label: impl Fn(T) -> Label,
    ) -> Result<usize, Error>
    where
        K: redb::Key + Debug + Clone + Send + Sync + 'static,
        T: Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static,
    {
        let table = self.read_table(table)?;

        let count = table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| to_label(record.value().item))
            .filter(is_meaningful_export_label)
            .count();

        Ok(count)
    }

    fn has_meaningful_label<K, T>(
        &self,
        table: TableDefinition<K, SerdeRecord<T>>,
        to_label: impl Fn(T) -> Label,
    ) -> Result<bool, Error>
    where
        K: redb::Key + Debug + Clone + Send + Sync + 'static,
        T: Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static,
    {
        let table = self.read_table(table)?;

        let has_label = table
            .iter()?
            .filter_map(Result::ok)
            .map(|(_key, record)| to_label(record.value().item))
            .any(|label| is_meaningful_export_label(&label));

        Ok(has_label)
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

fn has_non_empty_label(label: Option<&str>) -> bool {
    label.is_some_and(|label| !label.trim().is_empty())
}

fn is_meaningful_export_label(label: &Label) -> bool {
    match label {
        Label::Output(output) => {
            has_non_empty_label(output.label.as_deref()) || output.spendable.is_some()
        }
        _ => has_non_empty_label(label.label()),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use redb::TableDefinition;

    use super::LabelsTable;
    use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletId};

    const MISMATCHED_OUTPUT_TABLE: TableDefinition<&'static str, &'static str> =
        TableDefinition::new("output_records_v2.cbor");

    pub(crate) fn wallet_data_db_with_mismatched_output_table(
        id: WalletId,
    ) -> (WalletDataDb, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = tmp.path().join("wallet_data.encrypted.json.redb");
        let db = Arc::new(redb::Database::create(db_path).expect("failed to create test db"));
        let write_txn = db.begin_write().expect("failed to begin write transaction");
        {
            let mut table =
                write_txn.open_table(MISMATCHED_OUTPUT_TABLE).expect("failed to create table");
            table.insert("key", "value").expect("failed to insert mismatched value");
        }
        write_txn.commit().expect("failed to commit write transaction");

        let labels = LabelsTable { db: db.clone() };
        let wallet_db = WalletDataDb { id, db, labels };

        (wallet_db, tmp)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use crate::{
        database::{
            Record, cbor::Cbor, record::Timestamps,
            wallet_data::test_support::new_test_wallet_data_db,
        },
        wallet::metadata::WalletId,
    };
    use bip329::{Labels, OutputRecord};
    use bitcoin::OutPoint;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct LegacyOutputRecord {
        #[serde(rename = "ref")]
        ref_: bitcoin::OutPoint,
        label: Option<String>,
        spendable: bool,
    }

    #[test]
    fn output_record_deserializes_legacy_bool_spendable_cbor() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let legacy = Record::with_timestamps(
            LegacyOutputRecord {
                ref_: outpoint,
                label: Some("legacy".to_string()),
                spendable: false,
            },
            Timestamps::new(1, 2),
        );

        let bytes = <Cbor<Record<LegacyOutputRecord>> as redb::Value>::as_bytes(&legacy);
        let decoded = <Cbor<Record<OutputRecord>> as redb::Value>::from_bytes(bytes.as_ref());

        assert_eq!(decoded.item.ref_, outpoint);
        assert_eq!(decoded.item.label, Some("legacy".to_string()));
        assert_eq!(decoded.item.spendable, Some(false));
        assert_eq!(decoded.timestamps, Timestamps::new(1, 2));
    }

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
        assert_eq!(record.item.spendable, Some(false));
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
        assert_eq!(record.item.spendable, Some(true));
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
        assert_eq!(record.item.spendable, Some(false));
        assert!(
            db.locked_output_outpoints().expect("failed to get locked outputs").contains(&outpoint)
        );
    }

    #[test]
    fn import_skips_unlabeled_metadata_without_spendability_updates() {
        let ignored_output = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse ignored output");
        let ignored_spendable_true_output = OutPoint::from_str(
            "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2",
        )
        .expect("failed to parse ignored spendable true output");
        let jsonl = r#"
            {"type":"tx","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290","origin":"wpkh([d317f698/84h/1h/0h])"}
            {"type":"addr","ref":"bc1q34aq5drpuwy3wgl9lhup9892qp6svr8ldzyy7c","origin":"wpkh([d317f698/84h/1h/0h])"}
            {"type":"input","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0","origin":"wpkh([d317f698/84h/1h/0h])"}
            {"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","origin":"wpkh([d317f698/84h/1h/0h])"}
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2","origin":"wpkh([d317f698/84h/1h/0h])","spendable":true}
            {"type":"tx","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd","label":"real label"}
        "#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(jsonl).expect("failed to parse labels"),
        )
        .expect("failed to import labels");

        let exported = db.all_labels().expect("failed to load labels").export().unwrap();

        assert_eq!(db.number_of_labels().expect("failed to count labels"), 1);
        assert!(exported.contains(r#""label":"real label""#));
        assert!(
            !exported
                .contains("d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0")
        );
        assert!(db.get_output_record(ignored_output).expect("failed to get output").is_none());
        assert!(
            db.get_output_record(ignored_spendable_true_output)
                .expect("failed to get spendable true output")
                .is_none()
        );
    }

    #[test]
    fn export_includes_only_labeled_or_explicit_spendability_records() {
        let lock_only_output = OutPoint::from_str(
            "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2",
        )
        .expect("failed to parse lock-only output");
        let unlock_output = OutPoint::from_str(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0",
        )
        .expect("failed to parse unlock output");
        let jsonl = r#"
            {"type":"tx","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290"}
            {"type":"addr","ref":"bc1q34aq5drpuwy3wgl9lhup9892qp6svr8ldzyy7c"}
            {"type":"input","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0"}
            {"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1"}
            {"type":"output","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:2","spendable":false}
            {"type":"output","ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0","spendable":true}
            {"type":"tx","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd","label":"real label"}
        "#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(jsonl).expect("failed to parse labels"))
            .expect("failed to insert labels");

        let exported = db.all_labels().expect("failed to load labels").export().unwrap();

        assert_eq!(db.number_of_labels().expect("failed to count labels"), 3);
        assert!(exported.contains(&lock_only_output.to_string()));
        assert!(exported.contains(&unlock_output.to_string()));
        assert!(exported.contains(r#""label":"real label""#));
        assert!(exported.contains(r#""spendable":false"#));
        assert!(exported.contains(r#""spendable":true"#));
        assert!(!exported.contains("bc1q34aq5drpuwy3wgl9lhup9892qp6svr8ldzyy7c"));
        assert!(
            !exported
                .contains("f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1")
        );
    }

    #[test]
    fn import_unlabeled_spendable_true_marks_locked_output_unlocked_for_export() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let imported = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","spendable":true}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.set_output_spendability(outpoint, false).expect("failed to lock output");
        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(imported).expect("failed to parse labels"),
        )
        .expect("failed to import labels");

        let exported = db.all_labels().expect("failed to load labels").export().unwrap();
        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, None);
        assert_eq!(record.item.spendable, Some(true));
        assert!(db.locked_output_outpoints().expect("failed to get locked outputs").is_empty());
        assert!(exported.contains(r#""spendable":true"#));
    }

    #[test]
    fn import_unlabeled_spendability_update_preserves_existing_output_label() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let existing = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"keep me","spendable":false}"#;
        let imported = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","spendable":true}"#;
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.insert_labels(Labels::try_from_str(existing).expect("failed to parse existing labels"))
            .expect("failed to insert existing labels");
        db.insert_imported_labels(
            Labels::try_from_str_with_metadata(imported).expect("failed to parse imported labels"),
        )
        .expect("failed to import labels");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");

        assert_eq!(record.item.label, Some("keep me".to_string()));
        assert_eq!(record.item.spendable, Some(true));
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
        assert_eq!(record.item.spendable, Some(false));
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
    fn export_import_round_trip_preserves_lock_only_output_record() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let (source_wallet_db, _source_tmp) =
            new_test_wallet_data_db(WalletId::preview_new_random());
        let source_db = &source_wallet_db.labels;

        source_db.set_output_spendability(outpoint, false).expect("failed to lock output");
        let exported = source_db.all_labels().expect("failed to load labels").export().unwrap();

        let (destination_wallet_db, _destination_tmp) =
            new_test_wallet_data_db(WalletId::preview_new_random());
        let destination_db = &destination_wallet_db.labels;
        destination_db
            .insert_imported_labels(
                Labels::try_from_str_with_metadata(&exported)
                    .expect("failed to parse exported labels"),
            )
            .expect("failed to import exported labels");

        let record = destination_db
            .get_output_record(outpoint)
            .expect("failed to get imported output record")
            .expect("missing imported output record");
        let locked =
            destination_db.locked_output_outpoints().expect("failed to get locked outputs");

        assert_eq!(record.item.label, None);
        assert_eq!(record.item.spendable, Some(false));
        assert!(locked.contains(&outpoint));
    }

    #[test]
    fn unlocking_lock_only_record_marks_explicit_unlock() {
        let outpoint = OutPoint::from_str(
            "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1",
        )
        .expect("failed to parse outpoint");
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());
        let db = &wallet_db.labels;

        db.set_output_spendability(outpoint, false).expect("failed to lock output");
        db.set_output_spendability(outpoint, true).expect("failed to unlock output");

        let record = db
            .get_output_record(outpoint)
            .expect("failed to get output record")
            .expect("missing output record");
        let exported = db.all_labels().expect("failed to load labels").export().unwrap();

        assert_eq!(record.item.label, None);
        assert_eq!(record.item.spendable, Some(true));
        assert!(db.locked_output_outpoints().expect("failed to get locked outputs").is_empty());
        assert!(exported.contains(r#""spendable":true"#));
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
        assert_eq!(record.item.spendable, Some(true));
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

        assert_eq!(requested.spendable, Some(false));
        assert_eq!(untouched.spendable, Some(true));
    }
}
