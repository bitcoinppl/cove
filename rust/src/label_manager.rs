use std::sync::Arc;

use cove_util::result_ext::ResultExt as _;
use tracing::{debug, warn};

use crate::{
    database::{InsertOrUpdate, Record, record::Timestamps, wallet_data::WalletDataDb},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    multi_format::Bip329Labels,
    transaction::{TransactionDetails, TransactionDirection},
    wallet::{Address, metadata::WalletId},
};

use ahash::AHashMap as HashMap;
use bip329::{AddressRecord, InputRecord, Label, Labels, OutputRecord, TransactionRecord};
use cove_types::{TxId, confirm::QrDensity};

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelManager {
    db: WalletDataDb,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
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

#[derive(Debug, Clone, uniffi::Record)]
pub struct LabelParseReport {
    pub imported: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct AddressArgs {
    pub address: Option<Address>,
    pub change_address: Option<Address>,
    pub direction: TransactionDirection,
}

#[uniffi::export]
impl AddressArgs {
    #[uniffi::constructor]
    pub fn new(
        address: Option<Arc<Address>>,
        change_address: Option<Arc<Address>>,
        direction: TransactionDirection,
    ) -> Self {
        let address = address.map(Arc::unwrap_or_clone);
        let change_address = change_address.map(Arc::unwrap_or_clone);

        Self { address, change_address, direction }
    }
}

#[uniffi::export]
impl LabelManager {
    #[uniffi::constructor]
    pub fn new(id: WalletId) -> Self {
        let db = WalletDataDb::new_or_existing(id)
            .expect("failed to open wallet database for label manager");
        Self { db }
    }

    pub fn export_default_file_name(&self, name: String) -> String {
        let name = name
            .replace(' ', "_")
            .replace(|c: char| !c.is_alphanumeric() || c == '_', "")
            .to_ascii_lowercase();

        format!("{name}-bip329-labels")
    }

    pub fn has_labels(&self) -> bool {
        self.db.labels.number_of_labels().unwrap_or(0) > 0
    }

    pub fn transaction_label(&self, tx_id: Arc<TxId>) -> Option<String> {
        let label = self.db.labels.get_txn_label_record(tx_id.0).unwrap_or(None)?;

        let label_str = label.item.label.as_ref()?;
        Some(label_str.to_string())
    }

    pub fn insert_or_update_labels_for_txn(
        &self,
        details: Arc<TransactionDetails>,
        label: String,
        origin: Option<String>,
    ) -> Result<()> {
        let label = label.trim();

        // if the label is empty, delete the label
        if label.is_empty() {
            return self.delete_labels_for_txn(Arc::new(details.tx_id));
        }
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
                .map_err_str(LabelManagerError::SaveInputLabels)?;

            self.db
                .labels
                .insert_labels_with_timestamps(output_labels, timestamps)
                .map_err_str(LabelManagerError::SaveOutputLabels)?;

            self.mark_cloud_backup_dirty();

            return Ok(());
        }

        // UPDATE
        self.update_labels_for_txn(&tx_id, input_records_iter, output_records_iter)?;
        self.mark_cloud_backup_dirty();

        Ok(())
    }

    pub fn delete_labels_for_txn(&self, tx_id: Arc<TxId>) -> Result<(), LabelManagerError> {
        let Some(txn_label) =
            self.db.labels.get_txn_label_record(tx_id.0).map_err_str(LabelManagerError::Get)?
        else {
            return Ok(());
        };

        let txn_label_created_at = txn_label.timestamps.created_at;

        let input_records = self
            .db
            .labels
            .txn_input_records_iter(tx_id.0)
            .map_err_str(LabelManagerError::GetInputRecords)?;

        let output_records = self
            .db
            .labels
            .txn_output_records_iter(tx_id.0)
            .map_err_str(LabelManagerError::GetOutputRecords)?;

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
            .map_err_str(LabelManagerError::DeleteLabels)?;

        self.mark_cloud_backup_dirty();

        Ok(())
    }

    #[uniffi::method(name = "importLabels")]
    pub fn _import_labels(
        &self,
        labels: Arc<Bip329Labels>,
    ) -> Result<LabelParseReport, LabelManagerError> {
        let labels = Arc::unwrap_or_clone(labels);
        self.import_labels(labels.0)
    }

    pub fn import(&self, jsonl: &str) -> Result<LabelParseReport, LabelManagerError> {
        let (labels, report) = parse_labels(jsonl)?;
        self.save_imported_labels(labels, report, true)
    }

    pub async fn export(&self) -> Result<String, LabelManagerError> {
        let db = self.db.clone();

        cove_tokio::task::spawn_blocking(move || {
            let labels = db.labels.all_labels().map_err_str(LabelManagerError::Get)?;
            let labels = labels.export().map_err_str(LabelManagerError::Export)?;
            Ok(labels)
        })
        .await
        .map_err_str(LabelManagerError::Export)?
    }

    /// Export labels as BBQr-encoded QR strings for animated display
    pub async fn export_to_bbqr_with_density(
        &self,
        density: &QrDensity,
    ) -> Result<Vec<String>, LabelManagerError> {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let labels_jsonl = self.export().await?;
        let max_version = density.bbqr_max_version();

        cove_tokio::task::spawn_blocking(move || {
            let data = labels_jsonl.as_bytes();
            let version = Version::try_from(max_version).unwrap_or(Version::V15);

            let split = Split::try_from_data(
                data,
                FileType::Json,
                SplitOptions {
                    encoding: Encoding::Zlib,
                    min_split_number: 1,
                    max_split_number: 100,
                    min_version: Version::V01,
                    max_version: version,
                },
            )
            .map_err_prefix("BBQr encoding failed", LabelManagerError::Export)?;

            Ok(split.parts)
        })
        .await
        .map_err_str(LabelManagerError::Export)?
    }
}

impl LabelManager {
    pub fn try_new(
        id: WalletId,
    ) -> std::result::Result<Self, crate::database::wallet_data::WalletDataError> {
        let db = WalletDataDb::new_or_existing(id)?;
        Ok(Self { db })
    }

    pub fn import_labels(
        &self,
        labels: impl Into<Labels>,
    ) -> Result<LabelParseReport, LabelManagerError> {
        let labels = labels.into();
        // Count only supported variants — insert_label_with_write_txn silently drops
        // PublicKey and ExtendedPublicKey, so labels.len() would overstate the import count.
        let supported = labels
            .iter()
            .filter(|l| {
                matches!(
                    l,
                    Label::Transaction(_) | Label::Address(_) | Label::Input(_) | Label::Output(_)
                )
            })
            .count() as u32;
        let skipped = labels.len() as u32 - supported;
        let report = LabelParseReport { imported: supported, skipped };
        self.save_imported_labels(labels, report, true)
    }

    pub(crate) fn import_without_cloud_backup_dirty(
        &self,
        jsonl: &str,
    ) -> Result<LabelParseReport, LabelManagerError> {
        let (labels, report) = parse_labels(jsonl)?;
        self.save_imported_labels(labels, report, false)
    }

    fn save_imported_labels(
        &self,
        labels: Labels,
        report: LabelParseReport,
        mark_cloud_backup_dirty: bool,
    ) -> Result<LabelParseReport, LabelManagerError> {
        self.db.labels.insert_labels(labels).map_err_str(LabelManagerError::Save)?;
        if mark_cloud_backup_dirty {
            self.mark_cloud_backup_dirty();
        }

        Ok(report)
    }

    fn mark_cloud_backup_dirty(&self) {
        CLOUD_BACKUP_MANAGER.handle_wallet_backup_change(self.db.id.clone());
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
            .map_err_str(LabelManagerError::GetInputRecords)?
            .map(|record| (record.item.ref_.vout, record))
            .collect::<HashMap<u32, Record<InputRecord>>>();

        let mut current_output_records = self
            .db
            .labels
            .txn_output_records_iter(tx_id.as_ref())
            .map_err_str(LabelManagerError::GetOutputRecords)?
            .map(|record| (record.item.ref_.vout, record))
            .collect::<HashMap<u32, Record<OutputRecord>>>();

        let input_records = new_input_records_iter.into_iter().map(|record| {
            let vout = record.ref_.vout;
            let label: Label = record.into();

            match current_input_records.remove(&vout) {
                Some(current) => {
                    let mut timestamps = current.timestamps;
                    timestamps.updated_at = jiff::Timestamp::now().as_second() as u64;
                    Record::with_timestamps(label, timestamps)
                }
                None => Record::new(label),
            }
        });

        let output_records = new_output_records_iter.into_iter().map(|record| {
            let vout = record.ref_.vout;
            let label: Label = record.into();

            match current_output_records.remove(&vout) {
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
            .map_err_str(LabelManagerError::SaveInputLabels)?;

        self.db
            .labels
            .insert_records(output_records)
            .map_err_str(LabelManagerError::SaveOutputLabels)?;

        Ok(())
    }

    fn insert_or_update_address_records(
        &self,
        label: &str,
        args: AddressArgs,
    ) -> Result<Option<()>> {
        let now = jiff::Timestamp::now().as_second() as u64;
        let timestamps = Timestamps { created_at: now, updated_at: now };

        // incoming use address
        let address_record = match args.direction {
            TransactionDirection::Incoming => {
                let Some(address) = args.address else { return Ok(None) };
                Some(AddressRecord {
                    ref_: address.into_unchecked(),
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

        let current = self.db.labels.get_address_record(&address_record.ref_).unwrap_or(None);

        let mut timestamps = current.map_or(timestamps, |current| current.timestamps);

        timestamps.updated_at = now;

        self.db
            .labels
            .insert_label_with_timestamps(address_record, timestamps)
            .map_err_str(LabelManagerError::SaveAddressLabels)?;

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
                .map_err_str(LabelManagerError::Save)?;

            return Ok(InsertOrUpdate::Update(last_updated_at.into()));
        }

        // new label,insert new record
        let now = jiff::Timestamp::now().as_second() as u64;
        let label = TransactionRecord { ref_: tx_id.0, label: Some(label), origin };

        self.db
            .labels
            .insert_label_with_timestamps(label, Timestamps::new(now, now))
            .map_err_str(LabelManagerError::Save)?;

        Ok(InsertOrUpdate::Insert(now.into()))
    }

    // create input labels for a transaction to match sparrow auto-generated input labels
    fn create_input_records(
        &self,
        tx_id: &TxId,
        label: &str,
        vouts: &[u32],
        direction: TransactionDirection,
    ) -> Vec<InputRecord> {
        // no input lables on incoming transactions
        // the inputs are outputs on someone else's wallet / txn
        if direction == TransactionDirection::Incoming {
            return vec![];
        }

        // input labels only for outgoing transactions, so all inputs are marked at `input`
        let input_label = format!("{label} (input)");

        vouts
            .iter()
            .map(|vout| InputRecord {
                ref_: bitcoin::OutPoint { txid: tx_id.0, vout: *vout },
                label: Some(input_label.clone()),
            })
            .collect()
    }

    // create output labels for a transaction to match sparrow auto-generated ouput labels
    fn create_output_records(
        &self,
        tx_id: &TxId,
        label: &str,
        vouts: &[u32],
        direction: TransactionDirection,
    ) -> Vec<OutputRecord> {
        // the outputs for a incoming transaction are received
        // the outputs for an outgoing transaction are change
        let output_label_suffix = match direction {
            TransactionDirection::Incoming => "received",
            TransactionDirection::Outgoing => "change",
        };

        let output_label = format!("{label} ({output_label_suffix})");

        vouts
            .iter()
            .map(|vout| OutputRecord {
                ref_: bitcoin::OutPoint { txid: tx_id.0, vout: *vout },
                label: Some(output_label.clone()),
                spendable: true,
            })
            .collect()
    }
}

fn parse_labels(jsonl: &str) -> Result<(Labels, LabelParseReport), LabelManagerError> {
    let lines: Vec<&str> = jsonl.trim().lines().filter(|line| !line.trim().is_empty()).collect();

    if lines.is_empty() {
        return Ok((Labels::new(vec![]), LabelParseReport { imported: 0, skipped: 0 }));
    }

    let mut parsed = Vec::with_capacity(lines.len());
    let mut first_error: Option<String> = None;
    let mut skipped = 0u32;

    for line in &lines {
        match serde_json::from_str::<Label>(line) {
            Ok(label) => {
                if matches!(
                    label,
                    Label::Transaction(_) | Label::Address(_) | Label::Input(_) | Label::Output(_)
                ) {
                    parsed.push(label);
                } else {
                    debug!("skipping unsupported label type");
                    skipped += 1;
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
                debug!("skipping unrecognized label entry: {e}");
                skipped += 1;
            }
        }
    }

    if parsed.is_empty() {
        return Err(LabelManagerError::Parse(first_error.unwrap_or_default()));
    }

    if skipped > 0 {
        warn!(
            "imported {} of {} labels, skipped {skipped} unrecognized entries",
            parsed.len(),
            lines.len()
        );
    }

    let report = LabelParseReport { imported: parsed.len() as u32, skipped };
    Ok((Labels::new(parsed), report))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TX: &str = r#"{"type":"tx","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd","label":"test tx"}"#;
    const VALID_OUTPUT: &str = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:0","label":"test output"}"#;
    const VALID_INPUT: &str = r#"{"type":"input","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:1","label":"test input"}"#;

    // Nunchuk-style invalid output ref: vout contains non-numeric characters
    const BAD_VOUT_OUTPUT: &str = r#"{"type":"output","ref":"f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd:bad","label":"broken"}"#;

    #[test]
    fn test_parse_labels_valid() {
        let jsonl = format!("{VALID_TX}\n{VALID_OUTPUT}\n{VALID_INPUT}");
        let (labels, report) = parse_labels(&jsonl).unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(report.imported, 3);
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn test_parse_labels_skips_bad_vout_line() {
        let jsonl = format!("{VALID_TX}\n{BAD_VOUT_OUTPUT}\n{VALID_OUTPUT}");
        let (labels, report) = parse_labels(&jsonl).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn test_parse_labels_all_invalid_returns_error() {
        let jsonl = format!("{BAD_VOUT_OUTPUT}\n{BAD_VOUT_OUTPUT}");
        assert!(parse_labels(&jsonl).is_err());
    }

    #[test]
    fn test_parse_labels_empty_returns_empty() {
        let (labels, report) = parse_labels("").unwrap();
        assert!(labels.is_empty());
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn test_parse_labels_ignores_blank_lines() {
        let jsonl = format!("{VALID_TX}\n\n   \n{VALID_OUTPUT}");
        let (labels, report) = parse_labels(&jsonl).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 0);
    }
}
