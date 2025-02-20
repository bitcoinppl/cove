use std::sync::Arc;

use crate::{
    database::{InsertOrUpdate, Record, record::Timestamps, wallet_data::WalletDataDb},
    multi_format::Bip329Labels,
    transaction::{TransactionDetails, TransactionDirection, TxId},
    wallet::{Address, metadata::WalletId},
};
use ahash::AHashMap as HashMap;
use bip329::{AddressRecord, InOutId, InputRecord, Label, Labels, OutputRecord, TransactionRecord};

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelManager {
    #[allow(dead_code)]
    id: WalletId,
    db: WalletDataDb,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum LabelManagerError {
    #[error("Failed to parse labels: {0}")]
    Parse(String),

    #[error("Failed to save labels: {0}")]
    Save(String),

    #[error("Failed to get labels: {0}")]
    Get(String),

    #[error("Failed to export labels: {0}")]
    Export(String),

    #[error("Unable to get input records for txn: {0}")]
    GetInputRecords(String),

    #[error("Unable to get output records for txn: {0}")]
    GetOutputRecords(String),

    #[error("Unable to create input labels: {0}")]
    SaveInputLabels(String),

    #[error("Unable to create output labels: {0}")]
    SaveOutputLabels(String),

    #[error("Unable to delete labels: {0}")]
    DeleteLabels(String),

    #[error("Unable to save address labels: {0}")]
    SaveAddressLabels(String),
}

pub type Error = LabelManagerError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, uniffi::Object)]
pub struct AddressArgs {
    pub address: Address,
    pub change_address: Option<Address>,
    pub direction: TransactionDirection,
}

#[uniffi::export]
impl AddressArgs {
    #[uniffi::constructor]
    pub fn new(
        address: Arc<Address>,
        change_address: Option<Arc<Address>>,
        direction: TransactionDirection,
    ) -> Self {
        let address = Arc::unwrap_or_clone(address);
        let change_address = change_address.map(Arc::unwrap_or_clone);

        Self {
            address,
            change_address,
            direction,
        }
    }
}

#[uniffi::export]
impl LabelManager {
    #[uniffi::constructor]
    pub fn new(id: WalletId) -> Self {
        let db = WalletDataDb::new_or_existing(id.clone());
        Self { id, db }
    }

    pub fn export_default_file_name(&self, name: String) -> String {
        let name = name
            .replace(" ", "_")
            .replace(|c: char| !c.is_alphanumeric() || c == '_', "")
            .to_ascii_lowercase();

        format!("{name}-bip329-labels")
    }

    pub fn has_labels(&self) -> bool {
        self.db.labels.number_of_labels().unwrap_or(0) > 0
    }

    pub fn transaction_label(&self, tx_id: Arc<TxId>) -> Option<String> {
        let label = self
            .db
            .labels
            .get_txn_label_record(tx_id.0)
            .unwrap_or(None)?;

        let label_str = label.item.label.as_ref()?;
        Some(label_str.to_string())
    }

    pub fn insert_or_update_labels_for_txn(
        &self,
        details: Arc<TransactionDetails>,
        label: String,
        origin: Option<String>,
    ) -> Result<()> {
        // if the label is empty, don't do anything
        if label.is_empty() {
            return Ok(());
        }

        let label = label.trim();
        let tx_id = details.tx_id;
        let insert_or_update =
            self.insert_or_update_transaction_label(&tx_id, label.to_string(), origin)?;

        let input_records_iter = self
            .create_input_records(
                &tx_id,
                label,
                &details.input_indexes,
                details.sent_and_received.direction,
            )
            .into_iter();

        let output_records_iter = self
            .create_output_records(
                &tx_id,
                label,
                &details.output_indexes,
                details.sent_and_received.direction,
            )
            .into_iter();

        let address_args = AddressArgs {
            address: details.address.clone(),
            change_address: details.change_address.clone(),
            direction: details.sent_and_received.direction,
        };

        self.insert_or_update_address_records(label, address_args)?;

        // INSERT
        // if it's a new transaction, we need to insert input and output labels for each
        if let InsertOrUpdate::Insert(now) = insert_or_update {
            let input_labels = input_records_iter.map(Into::into).collect::<Vec<Label>>();
            let output_labels = output_records_iter.map(Into::into).collect::<Vec<Label>>();
            let timestamps = Timestamps::new(now.into(), now.into());

            self.db
                .labels
                .insert_labels_with_timestamps(input_labels, timestamps)
                .map_err(|e| LabelManagerError::SaveInputLabels(e.to_string()))?;

            self.db
                .labels
                .insert_labels_with_timestamps(output_labels, timestamps)
                .map_err(|e| LabelManagerError::SaveOutputLabels(e.to_string()))?;

            return Ok(());
        }

        // UPDATE
        self.update_labels_for_txn(&tx_id, input_records_iter, output_records_iter)?;

        Ok(())
    }

    pub fn delete_labels_for_txn(&self, tx_id: Arc<TxId>) -> Result<(), LabelManagerError> {
        let Some(txn_label) = self
            .db
            .labels
            .get_txn_label_record(tx_id.0)
            .map_err(|e| LabelManagerError::Get(e.to_string()))?
        else {
            return Ok(());
        };

        let txn_label_created_at = txn_label.timestamps.created_at;

        let input_records = self
            .db
            .labels
            .txn_input_records_iter(tx_id.0)
            .map_err(|e| LabelManagerError::GetInputRecords(e.to_string()))?;

        let output_records = self
            .db
            .labels
            .txn_output_records_iter(tx_id.0)
            .map_err(|e| LabelManagerError::GetOutputRecords(e.to_string()))?;

        // create list of labels to delete
        let mut labels_to_delete = vec![Label::from(txn_label.item)];

        // only delete the input record if it hasn't changed since being created with the txn label
        for record in input_records {
            if txn_label_created_at == record.timestamps.created_at
                && txn_label_created_at == record.timestamps.updated_at
            {
                labels_to_delete.push(Label::from(record.item));
            }
        }

        // only delete the output record if it hasn't changed since being created with the txn label
        for record in output_records {
            if txn_label_created_at == record.timestamps.created_at
                && txn_label_created_at == record.timestamps.updated_at
            {
                labels_to_delete.push(Label::from(record.item));
            }
        }

        self.db
            .labels
            .delete_labels(labels_to_delete)
            .map_err(|e| LabelManagerError::DeleteLabels(e.to_string()))?;

        Ok(())
    }

    #[uniffi::method(name = "importLabels")]
    pub fn _import_labels(&self, labels: Arc<Bip329Labels>) -> Result<(), LabelManagerError> {
        let labels = Arc::unwrap_or_clone(labels);
        self.import_labels(labels.0)
    }

    pub fn import(&self, jsonl: &str) -> Result<(), LabelManagerError> {
        let labels =
            Labels::try_from_str(jsonl).map_err(|e| LabelManagerError::Parse(e.to_string()))?;

        self.import_labels(labels)
    }

    pub fn export(&self) -> Result<String, LabelManagerError> {
        let labels = self
            .db
            .labels
            .all_labels()
            .map_err(|e| LabelManagerError::Get(e.to_string()))?;

        let labels = labels
            .export()
            .map_err(|e| LabelManagerError::Export(e.to_string()))?;

        Ok(labels)
    }
}

impl LabelManager {
    pub fn import_labels(&self, labels: impl Into<Labels>) -> Result<(), LabelManagerError> {
        let labels = labels.into();

        self.db
            .labels
            .insert_labels(labels)
            .map_err(|e| LabelManagerError::Save(e.to_string()))?;

        Ok(())
    }

    fn update_labels_for_txn(
        &self,
        tx_id: &TxId,
        new_input_records_iter: impl Iterator<Item = InputRecord>,
        new_output_records_iter: impl Iterator<Item = OutputRecord>,
    ) -> Result<(), LabelManagerError> {
        let mut current_input_records = self
            .db
            .labels
            .txn_input_records_iter(tx_id.as_ref())
            .map_err(|e| LabelManagerError::GetInputRecords(e.to_string()))?
            .map(|record| (record.item.ref_.index, record))
            .collect::<HashMap<u32, Record<InputRecord>>>();

        let mut current_output_records = self
            .db
            .labels
            .txn_output_records_iter(tx_id.as_ref())
            .map_err(|e| LabelManagerError::GetOutputRecords(e.to_string()))?
            .map(|record| (record.item.ref_.index, record))
            .collect::<HashMap<u32, Record<OutputRecord>>>();

        let input_records = new_input_records_iter.into_iter().map(|record| {
            let index = record.ref_.index;
            let label: Label = record.into();

            match current_input_records.remove(&index) {
                Some(current) => {
                    let mut timestamps = current.timestamps;
                    timestamps.updated_at = jiff::Timestamp::now().as_second() as u64;
                    Record::with_timestamps(label, timestamps)
                }
                None => Record::new(label),
            }
        });

        let output_records = new_output_records_iter.into_iter().map(|record| {
            let index = record.ref_.index;
            let label: Label = record.into();

            match current_output_records.remove(&index) {
                Some(current) => {
                    let mut timestamps = current.timestamps;
                    timestamps.updated_at = jiff::Timestamp::now().as_second() as u64;
                    Record::with_timestamps(label, timestamps)
                }
                None => Record::new(label),
            }
        });

        self.db
            .labels
            .insert_records(input_records)
            .map_err(|e| LabelManagerError::SaveInputLabels(e.to_string()))?;

        self.db
            .labels
            .insert_records(output_records)
            .map_err(|e| LabelManagerError::SaveOutputLabels(e.to_string()))?;

        Ok(())
    }

    fn insert_or_update_address_records(
        &self,
        label: &str,
        args: AddressArgs,
    ) -> Result<Option<()>> {
        let now = jiff::Timestamp::now().as_second() as u64;
        let timestamps = Timestamps {
            created_at: now,
            updated_at: now,
        };

        // incoming use address
        let address_record = match args.direction {
            TransactionDirection::Incoming => {
                let address = args.address.into_unchecked();
                Some(AddressRecord {
                    ref_: address.clone(),
                    label: Some(label.to_string()),
                })
            }
            TransactionDirection::Outgoing => {
                let Some(address) = args.change_address else { return Ok(None) };
                Some(AddressRecord {
                    ref_: address.into_unchecked(),
                    label: Some(label.to_string()),
                })
            }
        };

        let Some(address_record) = address_record else { return Ok(None) };

        let current = self
            .db
            .labels
            .get_address_record(&address_record.ref_)
            .unwrap_or(None);

        let mut timestamps = current
            .map(|current| current.timestamps)
            .unwrap_or(timestamps);

        timestamps.updated_at = now;

        self.db
            .labels
            .insert_label_with_timestamps(address_record, timestamps)
            .map_err(|e| LabelManagerError::SaveAddressLabels(e.to_string()))?;

        Ok(Some(()))
    }

    fn insert_or_update_transaction_label(
        &self,
        tx_id: &TxId,
        label: String,
        origin: Option<String>,
    ) -> Result<InsertOrUpdate> {
        let current = self.db.labels.get_txn_label_record(tx_id.0).unwrap_or(None);

        // update the label
        if let Some(current) = current {
            let last_updated_at = current.timestamps.updated_at;

            let mut updated = current.item;
            let mut timestamps = current.timestamps;

            updated.label = Some(label);
            timestamps.updated_at = jiff::Timestamp::now().as_second() as u64;

            self.db
                .labels
                .insert_label_with_timestamps(updated, timestamps)
                .map_err(|e| LabelManagerError::Save(e.to_string()))?;

            return Ok(InsertOrUpdate::Update(last_updated_at.into()));
        };

        // new label,insert new record
        let now = jiff::Timestamp::now().as_second() as u64;
        let label = TransactionRecord {
            ref_: tx_id.0,
            label: Some(label),
            origin,
        };

        self.db
            .labels
            .insert_label_with_timestamps(label, Timestamps::new(now, now))
            .map_err(|e| LabelManagerError::Save(e.to_string()))?;

        Ok(InsertOrUpdate::Insert(now.into()))
    }

    // create input labels for a transaction to match sparrow auto-generated input labels
    fn create_input_records(
        &self,
        tx_id: &TxId,
        label: &str,
        indexs: &[u32],
        direction: TransactionDirection,
    ) -> Vec<InputRecord> {
        // no input lables on incoming transactions
        // the inputs are outputs on someone else's wallet / txn
        if direction == TransactionDirection::Incoming {
            return vec![];
        }

        // input labels only for outgoing transactions, so all inputs are marked at `input`
        let input_label = format!("{label} (input)");

        indexs
            .iter()
            .map(|index| InputRecord {
                ref_: InOutId::new(tx_id.0, *index),
                label: Some(input_label.clone()),
            })
            .collect()
    }

    // create output labels for a transaction to match sparrow auto-generated ouput labels
    fn create_output_records(
        &self,
        tx_id: &TxId,
        label: &str,
        indexs: &[u32],
        direction: TransactionDirection,
    ) -> Vec<OutputRecord> {
        // the outputs for a incoming transaction are received
        // the outputs for an outgoing transaction are change
        let output_label_suffix = match direction {
            TransactionDirection::Incoming => "received",
            TransactionDirection::Outgoing => "change",
        };

        let output_label = format!("{label} {output_label_suffix}");

        indexs
            .iter()
            .map(|index| OutputRecord {
                ref_: InOutId::new(tx_id.0, *index),
                label: Some(output_label.clone()),
                spendable: true,
            })
            .collect()
    }
}
