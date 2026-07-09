use std::sync::Arc;

use redb::{TableDefinition, TypeName, Value};
use serde::{Deserialize, Serialize};
use tracing::error;

use cove_util::result_ext::ResultExt as _;

use super::Error;

const HISTORY_KEY: &str = "history";
const MAX_REPORTS: usize = 5;
const REPORTS_JSON_TYPE_NAME: &str =
    "SerdeJson<cove::database::diagnostics_reports::DiagnosticsReportHistoryV1>";

pub const TABLE: TableDefinition<&'static str, DiagnosticsReportsJson> =
    TableDefinition::new("diagnostics_reports");

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticsReportRecord {
    pub report_id: String,
    pub submitted_at: u64,
    pub description: Option<String>,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct DiagnosticsReportsTable {
    db: Arc<redb::Database>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum DiagnosticsReportsTableError {
    #[error("failed to save diagnostics report history: {0}")]
    Save(String),

    #[error("failed to read diagnostics report history: {0}")]
    Read(String),
}

#[derive(Debug)]
pub struct DiagnosticsReportsJson;

#[derive(Debug)]
pub struct DiagnosticsReportHistory {
    records: Vec<DiagnosticsReportRecord>,
    decode_error: Option<String>,
}

impl Value for DiagnosticsReportsJson {
    type SelfType<'a>
        = DiagnosticsReportHistory
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        DiagnosticsReportHistory::from_bytes(data)
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        serde_json::to_vec(&value.records).expect("failed to serialize diagnostics report history")
    }

    fn type_name() -> TypeName {
        // pin the table metadata so moving this module does not break old installs
        TypeName::new(REPORTS_JSON_TYPE_NAME)
    }
}

impl DiagnosticsReportHistory {
    fn from_records(records: Vec<DiagnosticsReportRecord>) -> Self {
        Self { records, decode_error: None }
    }

    fn from_bytes(data: &[u8]) -> Self {
        match serde_json::from_slice(data) {
            Ok(records) => Self::from_records(records),
            Err(error) => {
                let error = error.to_string();
                error!("Failed to decode diagnostics report history: {error}");

                Self { records: Vec::new(), decode_error: Some(error) }
            }
        }
    }

    fn into_records(self) -> Result<Vec<DiagnosticsReportRecord>, DiagnosticsReportsTableError> {
        if let Some(error) = self.decode_error {
            return Err(DiagnosticsReportsTableError::Read(error));
        }

        Ok(self.records)
    }
}

impl DiagnosticsReportRecord {
    pub fn now(report_id: String, description: Option<String>) -> Self {
        let submitted_at = jiff::Timestamp::now().as_second().cast_unsigned();

        Self { report_id, submitted_at, description }
    }
}

impl DiagnosticsReportsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        write_txn.open_table(TABLE).expect("failed to create diagnostics reports table");

        Self { db }
    }

    pub fn add(&self, record: DiagnosticsReportRecord) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;
            let mut records = read_history(&table)?;

            records.insert(0, record);
            records.truncate(MAX_REPORTS);

            let history = DiagnosticsReportHistory::from_records(records);
            table.insert(HISTORY_KEY, &history).map_err_str(DiagnosticsReportsTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }
}

fn read_history(
    table: &impl redb::ReadableTable<&'static str, DiagnosticsReportsJson>,
) -> Result<Vec<DiagnosticsReportRecord>, Error> {
    table
        .get(HISTORY_KEY)
        .map_err_str(DiagnosticsReportsTableError::Read)?
        .map(|value| value.value().into_records())
        .transpose()
        .map_err(Error::from)
        .map(Option::unwrap_or_default)
}

#[uniffi::export]
impl DiagnosticsReportsTable {
    pub fn all(&self) -> Result<Vec<DiagnosticsReportRecord>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;
        let table = read_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;
        let records = read_history(&table)?;

        Ok(records)
    }

    pub fn clear(&self) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;
            table.remove(HISTORY_KEY).map_err_str(DiagnosticsReportsTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use redb::{TableDefinition, TypeName, Value};

    use super::{DiagnosticsReportsTable, HISTORY_KEY, REPORTS_JSON_TYPE_NAME};

    #[derive(Debug)]
    struct InvalidDiagnosticsReportsJson;

    const INVALID_TABLE: TableDefinition<&'static str, InvalidDiagnosticsReportsJson> =
        TableDefinition::new("diagnostics_reports");

    impl Value for InvalidDiagnosticsReportsJson {
        type SelfType<'a>
            = Vec<u8>
        where
            Self: 'a;

        type AsBytes<'a>
            = Vec<u8>
        where
            Self: 'a;

        fn fixed_width() -> Option<usize> {
            None
        }

        fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
        where
            Self: 'a,
        {
            data.to_vec()
        }

        fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
        where
            Self: 'b,
        {
            value.clone()
        }

        fn type_name() -> TypeName {
            TypeName::new(REPORTS_JSON_TYPE_NAME)
        }
    }

    pub(crate) fn write_invalid_history(table: &DiagnosticsReportsTable) {
        let write_txn = table.db.begin_write().expect("failed to begin write txn");

        {
            let mut raw_table =
                write_txn.open_table(INVALID_TABLE).expect("failed to open invalid table");
            raw_table
                .insert(HISTORY_KEY, &b"not json".to_vec())
                .expect("failed to insert invalid history");
        }

        write_txn.commit().expect("failed to commit invalid history");
    }

    pub(crate) fn read_invalid_history(db: &redb::Database) -> Vec<u8> {
        let read_txn = db.begin_read().expect("failed to begin read txn");
        let raw_table = read_txn.open_table(INVALID_TABLE).expect("failed to open invalid table");

        raw_table
            .get(HISTORY_KEY)
            .expect("failed to read invalid history")
            .expect("invalid history exists")
            .value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_table() -> (tempfile::TempDir, DiagnosticsReportsTable) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = tmp.path().join("diagnostics_reports.redb");
        let db = Arc::new(redb::Database::create(db_path).expect("failed to create redb"));
        let write_txn = db.begin_write().expect("failed to begin write txn");
        let table = DiagnosticsReportsTable::new(db, &write_txn);
        write_txn.commit().expect("failed to commit table creation");

        (tmp, table)
    }

    fn record(index: u64) -> DiagnosticsReportRecord {
        DiagnosticsReportRecord {
            report_id: format!("report-{index}"),
            submitted_at: index,
            description: Some(format!("description {index}")),
        }
    }

    #[test]
    fn value_type_name_is_pinned() {
        assert_eq!(DiagnosticsReportsJson::type_name(), TypeName::new(REPORTS_JSON_TYPE_NAME));
    }

    #[test]
    fn add_keeps_newest_five_reports() {
        let (_tmp, table) = test_table();

        for index in 0..7 {
            table.add(record(index)).expect("failed to add report");
        }

        let reports = table.all().expect("failed to read reports");
        let ids = reports.into_iter().map(|record| record.report_id).collect::<Vec<_>>();

        assert_eq!(ids, ["report-6", "report-5", "report-4", "report-3", "report-2"]);
    }

    #[test]
    fn clear_removes_report_history() {
        let (_tmp, table) = test_table();

        table.add(record(1)).expect("failed to add report");
        assert_eq!(table.all().expect("failed to read reports").len(), 1);

        table.clear().expect("failed to clear reports");

        assert!(table.all().expect("failed to read reports").is_empty());
    }

    #[test]
    fn add_does_not_overwrite_history_after_decode_failure() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = tmp.path().join("diagnostics_reports.redb");
        let db = Arc::new(redb::Database::create(db_path).expect("failed to create redb"));
        let write_txn = db.begin_write().expect("failed to begin write txn");
        let table = DiagnosticsReportsTable::new(db.clone(), &write_txn);
        write_txn.commit().expect("failed to commit table creation");
        test_support::write_invalid_history(&table);

        assert!(table.add(record(1)).is_err());
        assert!(table.all().is_err());

        let raw_history = test_support::read_invalid_history(&db);

        assert_eq!(raw_history, b"not json".to_vec());
    }
}
