use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use redb::TableHandle as _;

use crate::bootstrap::{AppInitError, BootstrapStep};
use crate::database::encrypted_backend::{EncryptedBackend, encryption_key};
use crate::database::migration::MigrationFailure;
use crate::wallet::metadata::WalletId;

macro_rules! report_line {
    ($report:expr) => {
        $report.push('\n');
    };
    ($report:expr, $($arg:tt)*) => {
        let _ = writeln!($report, $($arg)*);
    };
}

static LAST_BOOTSTRAP_FAILURE: Mutex<Option<LastBootstrapFailure>> = Mutex::new(None);
static LAST_WALLET_MIGRATION_FAILURES: Mutex<Vec<MigrationFailure>> = Mutex::new(Vec::new());
static LAST_KNOWN_WALLET_IDS: Mutex<Option<BTreeSet<String>>> = Mutex::new(None);

#[derive(Debug, Clone)]
struct LastBootstrapFailure {
    timestamp: String,
    category: ErrorCategory,
    message: String,
    step: BootstrapStep,
    migration_progress: Option<(u32, u32)>,
    wallet_migration_failures: Vec<MigrationFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCategory {
    KeyDerivation,
    MainDatabaseMigration,
    WalletDatabaseMigration,
    Cancelled,
    AlreadyCalled,
    DatabaseKeyMismatch,
    DatabaseVerificationFailed,
}

impl ErrorCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::KeyDerivation => "key_derivation",
            Self::MainDatabaseMigration => "main_database_migration",
            Self::WalletDatabaseMigration => "wallet_database_migration",
            Self::Cancelled => "cancelled",
            Self::AlreadyCalled => "already_called",
            Self::DatabaseKeyMismatch => "database_key_mismatch",
            Self::DatabaseVerificationFailed => "database_verification_failed",
        }
    }
}

impl From<&AppInitError> for ErrorCategory {
    fn from(error: &AppInitError) -> Self {
        match error {
            AppInitError::KeyDerivation(_) => Self::KeyDerivation,
            AppInitError::MainDatabaseMigration(_) => Self::MainDatabaseMigration,
            AppInitError::WalletDatabaseMigration(_) => Self::WalletDatabaseMigration,
            AppInitError::Cancelled(_) => Self::Cancelled,
            AppInitError::AlreadyCalled(_) => Self::AlreadyCalled,
            AppInitError::DatabaseKeyMismatch(_) => Self::DatabaseKeyMismatch,
            AppInitError::DatabaseVerificationFailed(_) => Self::DatabaseVerificationFailed,
        }
    }
}

#[derive(Debug, Clone)]
struct DatabaseFileReport {
    present_file: bool,
    tables: Option<TableInventory>,
}

#[derive(Debug, Clone)]
enum TableInventory {
    Tables(BTreeSet<String>),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalletDirStatus {
    Known,
    Orphan,
    Unknown,
}

impl WalletDirStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Known => "known",
            Self::Orphan => "orphan",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn record_bootstrap_failure(error: &AppInitError) {
    let category = ErrorCategory::from(error);
    let progress = super::migration::active_migration()
        .map(|migration| migration.progress())
        .map(|progress| (progress.current, progress.total));
    let wallet_migration_failures = LAST_WALLET_MIGRATION_FAILURES.lock().clone();

    let failure = LastBootstrapFailure {
        timestamp: timestamp(),
        category,
        message: error.to_string(),
        step: super::bootstrap_progress(),
        migration_progress: progress,
        wallet_migration_failures,
    };

    let mut last_failure = LAST_BOOTSTRAP_FAILURE.lock();
    if category == ErrorCategory::AlreadyCalled
        && last_failure
            .as_ref()
            .is_some_and(|failure| failure.category != ErrorCategory::AlreadyCalled)
    {
        return;
    }

    *last_failure = Some(failure);
}

pub(crate) fn clear_bootstrap_failure() {
    *LAST_BOOTSTRAP_FAILURE.lock() = None;
    LAST_WALLET_MIGRATION_FAILURES.lock().clear();
    *LAST_KNOWN_WALLET_IDS.lock() = None;
}

pub(crate) fn record_wallet_migration_failures(failures: &[MigrationFailure]) {
    *LAST_WALLET_MIGRATION_FAILURES.lock() = failures.to_vec();
}

pub(crate) fn record_known_wallet_ids(ids: &BTreeSet<WalletId>) {
    let ids = ids.iter().map(ToString::to_string).collect();
    *LAST_KNOWN_WALLET_IDS.lock() = Some(ids);
}

pub(crate) fn text_report() -> String {
    let root_dir = &*cove_common::consts::ROOT_DATA_DIR;
    let wallet_dir = root_dir.join("wallets");

    text_report_for_paths(root_dir, &wallet_dir)
}

pub(crate) fn text_report_for_paths(root_dir: &Path, wallet_dir: &Path) -> String {
    let mut report = String::new();
    let generated_at = timestamp();
    let step = super::bootstrap_progress();
    let active_progress = super::migration::active_migration()
        .map(|migration| migration.progress())
        .map(|progress| (progress.current, progress.total));
    let known_wallet_ids = LAST_KNOWN_WALLET_IDS.lock().clone();
    let last_failure = LAST_BOOTSTRAP_FAILURE.lock().clone();
    let wallet_failures = last_failure
        .as_ref()
        .map(|failure| failure.wallet_migration_failures.clone())
        .unwrap_or_else(|| LAST_WALLET_MIGRATION_FAILURES.lock().clone());

    report_line!(report, "Cove core startup diagnostics");
    report_line!(report, "Generated: {generated_at}");
    report_line!(report);
    report_line!(report, "Bootstrap");
    report_line!(report, "Current step: {step:?}");
    report_line!(report, "Current migration progress: {}", format_progress(active_progress));

    if let Some(failure) = last_failure {
        report_line!(report, "Last failure recorded: {}", failure.timestamp);
        report_line!(report, "Last failure category: {}", failure.category.as_str());
        report_line!(report, "Last failure step: {:?}", failure.step);
        report_line!(
            report,
            "Last failure migration progress: {}",
            format_progress(failure.migration_progress)
        );
        report_line!(report, "Last failure message: {}", failure.message);
    } else {
        report_line!(report, "Last failure: none recorded");
    }

    report_line!(report);
    report_line!(report, "Storage files");
    report_line!(report, "Root: {}", root_dir.display());
    append_main_database_state(&mut report, root_dir);

    report_line!(report);
    report_line!(report, "Wallet storage");
    report_line!(report, "Wallet dir: {}", wallet_dir.display());
    append_wallet_dirs(&mut report, wallet_dir, known_wallet_ids.as_ref());

    report_line!(report);
    report_line!(report, "Last wallet migration failures");
    if wallet_failures.is_empty() {
        report_line!(report, "none recorded");
    } else {
        for failure in wallet_failures {
            report_line!(report, "- {failure}");
        }
    }

    report
}

fn append_main_database_state(report: &mut String, root_dir: &Path) {
    let table_classification = main_table_classification();
    report_line!(
        report,
        "Main migration state: {}",
        migration_state(root_dir, "cove.db", "cove.encrypted.db", "cove.encrypted.db.tmp")
    );

    let source = append_database_file(report, root_dir, "cove.db", "", &table_classification);
    let dest =
        append_database_file(report, root_dir, "cove.encrypted.db", "", &table_classification);
    let tmp =
        append_database_file(report, root_dir, "cove.encrypted.db.tmp", "", &table_classification);
    append_database_file(report, root_dir, "cove.db.bak", "", &table_classification);
    append_database_file(report, root_dir, "cove.db.enc.tmp", "", &table_classification);
    append_database_file(report, root_dir, "cove.encrypted.db.corrupt", "", &table_classification);

    append_table_comparison(report, "cove.db -> cove.encrypted.db", &source, &dest, "");
    append_table_comparison(report, "cove.db -> cove.encrypted.db.tmp", &source, &tmp, "");
}

fn append_wallet_dirs(
    report: &mut String,
    wallet_dir: &Path,
    known_wallet_ids: Option<&BTreeSet<String>>,
) {
    let entries = match sorted_dirs(wallet_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report_line!(report, "wallet directory missing");
            return;
        }
        Err(error) => {
            report_line!(report, "wallet directory unreadable: {error}");
            return;
        }
    };

    report_line!(report, "wallet directories: {}", entries.len());
    if known_wallet_ids.is_none() {
        report_line!(report, "known wallet ids: unavailable");
    }

    for path in entries {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("<invalid>");
        let status = match known_wallet_ids {
            Some(ids) if ids.contains(name) => WalletDirStatus::Known,
            Some(_) => WalletDirStatus::Orphan,
            None => WalletDirStatus::Unknown,
        };

        report_line!(report, "- {name} ({})", status.as_str());
        append_wallet_database_state(report, &path, status);
    }
}

fn append_wallet_database_state(report: &mut String, wallet_dir: &Path, status: WalletDirStatus) {
    let table_classification = wallet_table_classification();
    let state = migration_state(
        wallet_dir,
        "wallet_data.json",
        "wallet_data.encrypted.json.redb",
        "wallet_data.encrypted.json.redb.tmp",
    );
    if status == WalletDirStatus::Orphan {
        report_line!(report, "  migration state: skipped: orphan wallet dir ({state})");
    } else {
        report_line!(report, "  migration state: {state}");
    }

    let source =
        append_database_file(report, wallet_dir, "wallet_data.json", "  ", &table_classification);
    let dest = append_database_file(
        report,
        wallet_dir,
        "wallet_data.encrypted.json.redb",
        "  ",
        &table_classification,
    );
    let tmp = append_database_file(
        report,
        wallet_dir,
        "wallet_data.encrypted.json.redb.tmp",
        "  ",
        &table_classification,
    );
    append_database_file(report, wallet_dir, "wallet_data.json.bak", "  ", &table_classification);
    append_database_file(
        report,
        wallet_dir,
        "wallet_data.json.enc.tmp",
        "  ",
        &table_classification,
    );
    append_database_file(
        report,
        wallet_dir,
        "wallet_data.encrypted.json.redb.corrupt",
        "  ",
        &table_classification,
    );

    append_table_comparison(
        report,
        "wallet_data.json -> wallet_data.encrypted.json.redb",
        &source,
        &dest,
        "  ",
    );
    append_table_comparison(
        report,
        "wallet_data.json -> wallet_data.encrypted.json.redb.tmp",
        &source,
        &tmp,
        "  ",
    );
}

fn append_database_file(
    report: &mut String,
    directory: &Path,
    name: &str,
    prefix: &str,
    table_classification: &TableClassification,
) -> DatabaseFileReport {
    let path = directory.join(name);
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report_line!(report, "{prefix}{name}: missing");
            return DatabaseFileReport { present_file: false, tables: None };
        }
        Err(error) => {
            report_line!(report, "{prefix}{name}: unreadable: {error}");
            return DatabaseFileReport { present_file: false, tables: None };
        }
    };

    if !metadata.is_file() {
        report_line!(report, "{prefix}{name}: present, not a file");
        return DatabaseFileReport { present_file: false, tables: None };
    }

    let encrypted = EncryptedBackend::is_encrypted(&path);
    let encrypted_suffix = if encrypted { ", cove_header=true" } else { "" };
    report_line!(report, "{prefix}{name}: present, bytes={}{}", metadata.len(), encrypted_suffix);

    let tables = inspect_tables(&path, encrypted);
    append_table_inventory(report, &tables, table_classification, prefix);

    DatabaseFileReport { present_file: true, tables: Some(tables) }
}

fn inspect_tables(path: &Path, encrypted: bool) -> TableInventory {
    if encrypted {
        let Some(key) = encryption_key() else {
            return TableInventory::Unavailable("encrypted redb: key unavailable".into());
        };

        let backend = match EncryptedBackend::open(path, &key) {
            Ok(backend) => backend,
            Err(error) => {
                return TableInventory::Unavailable(format!(
                    "failed to open encrypted redb backend: {error}"
                ));
            }
        };

        let db = match redb::Database::builder().create_with_backend(backend) {
            Ok(db) => db,
            Err(error) => {
                return TableInventory::Unavailable(format!(
                    "failed to open encrypted redb: {error}"
                ));
            }
        };

        return table_names(&db);
    }

    let db = match redb::Database::open(path) {
        Ok(db) => db,
        Err(error) => return TableInventory::Unavailable(format!("failed to open redb: {error}")),
    };

    table_names(&db)
}

fn table_names(db: &redb::Database) -> TableInventory {
    let read_txn = match db.begin_read() {
        Ok(txn) => txn,
        Err(error) => {
            return TableInventory::Unavailable(format!(
                "failed to begin table listing read: {error}"
            ));
        }
    };
    let tables = match read_txn.list_tables() {
        Ok(tables) => tables,
        Err(error) => {
            return TableInventory::Unavailable(format!("failed to list tables: {error}"));
        }
    };

    TableInventory::Tables(tables.map(|handle| handle.name().to_string()).collect())
}

fn append_table_inventory(
    report: &mut String,
    inventory: &TableInventory,
    table_classification: &TableClassification,
    prefix: &str,
) {
    match inventory {
        TableInventory::Tables(tables) if tables.is_empty() => {
            report_line!(report, "{prefix}  schema tables: none");
        }
        TableInventory::Tables(tables) => {
            let labelled = tables
                .iter()
                .map(|table| {
                    let status = table_classification.label(table);
                    format!("{table} ({status})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            report_line!(report, "{prefix}  schema tables: {labelled}");

            let unknown = tables
                .iter()
                .filter(|table| !table_classification.current.contains(*table))
                .filter(|table| !table_classification.historical.contains(*table))
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            if !unknown.is_empty() {
                report_line!(report, "{prefix}  unknown tables: {unknown}");
            }

            let historical = tables
                .intersection(&table_classification.historical)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            if !historical.is_empty() {
                report_line!(report, "{prefix}  historical skipped tables: {historical}");
            }
        }
        TableInventory::Unavailable(error) => {
            report_line!(report, "{prefix}  schema unavailable: {error}");
        }
    }
}

fn append_table_comparison(
    report: &mut String,
    label: &str,
    source: &DatabaseFileReport,
    dest: &DatabaseFileReport,
    prefix: &str,
) {
    if !source.present_file || !dest.present_file {
        return;
    }

    let Some(source_tables) = source.tables.as_ref() else {
        return;
    };
    let Some(dest_tables) = dest.tables.as_ref() else {
        return;
    };

    match (source_tables, dest_tables) {
        (TableInventory::Tables(source), TableInventory::Tables(dest)) => {
            let missing = source.difference(dest).map(String::as_str).collect::<Vec<_>>();
            let extra = dest.difference(source).map(String::as_str).collect::<Vec<_>>();
            if missing.is_empty() && extra.is_empty() {
                report_line!(report, "{prefix}table comparison {label}: matches");
                return;
            }

            report_line!(report, "{prefix}table comparison {label}:");
            report_line!(report, "{prefix}  missing in destination: {}", format_list(&missing));
            report_line!(report, "{prefix}  extra in destination: {}", format_list(&extra));
        }
        (TableInventory::Unavailable(error), _) => {
            report_line!(report, "{prefix}table comparison {label}: source unavailable: {error}");
        }
        (_, TableInventory::Unavailable(error)) => {
            report_line!(
                report,
                "{prefix}table comparison {label}: destination unavailable: {error}"
            );
        }
    }
}

fn migration_state(directory: &Path, source_name: &str, dest_name: &str, tmp_name: &str) -> String {
    let source = directory.join(source_name);
    let dest = directory.join(dest_name);
    let tmp = directory.join(tmp_name);
    let source_exists = source.exists();
    let dest_exists = dest.exists();
    let tmp_exists = tmp.exists();

    match (source_exists, dest_exists, tmp_exists) {
        (true, true, true) => "source, destination, and tmp all present".into(),
        (true, true, false) => "source and destination both present".into(),
        (true, false, true) => "interrupted tmp present with source preserved".into(),
        (false, true, true) => "interrupted tmp present with destination".into(),
        (false, false, true) => "interrupted tmp present".into(),
        (true, false, false) if EncryptedBackend::is_encrypted(&source) => {
            "legacy encrypted rename".into()
        }
        (true, false, false) => "needs plaintext migration".into(),
        (false, true, false) => "already encrypted".into(),
        (false, false, false) => "missing".into(),
    }
}

#[derive(Debug)]
struct TableClassification {
    current: BTreeSet<String>,
    historical: BTreeSet<String>,
}

impl TableClassification {
    fn label(&self, table: &str) -> &'static str {
        if self.current.contains(table) {
            "current"
        } else if self.historical.contains(table) {
            "historical"
        } else {
            "unknown"
        }
    }
}

fn main_table_classification() -> TableClassification {
    let current = [
        crate::database::global_flag::TABLE.name(),
        crate::database::global_config::TABLE.name(),
        crate::database::global_cache::TABLE.name(),
        crate::database::cloud_backup::CLOUD_BACKUP_STATE_TABLE.name(),
        crate::database::cloud_backup::CLOUD_BLOB_SYNC_STATE_TABLE.name(),
        crate::database::wallet::TABLE.name(),
        crate::database::unsigned_transactions::MAIN_TABLE.name(),
        crate::database::unsigned_transactions::BY_WALLET_TABLE.name(),
        crate::database::historical_price::TABLE.name(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let historical = crate::database::migration::HISTORICAL_MAIN_REDB_TABLES
        .iter()
        .copied()
        .map(str::to_string)
        .collect();

    TableClassification { current, historical }
}

fn wallet_table_classification() -> TableClassification {
    let current = [
        crate::database::wallet_data::TABLE.name(),
        crate::database::wallet_data::label::TXN_TABLE.name(),
        crate::database::wallet_data::label::ADDRESS_TABLE.name(),
        crate::database::wallet_data::label::INPUT_TABLE.name(),
        crate::database::wallet_data::label::OUTPUT_TABLE.name(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let historical = crate::database::migration::HISTORICAL_WALLET_REDB_TABLES
        .iter()
        .copied()
        .map(str::to_string)
        .collect();

    TableClassification { current, historical }
}

fn format_list(items: &[&str]) -> String {
    if items.is_empty() { "none".into() } else { items.join(", ") }
}

fn sorted_dirs(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = std::fs::read_dir(path)?
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(_) => None,
        })
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(entries)
}

fn format_progress(progress: Option<(u32, u32)>) -> String {
    progress.map(|(current, total)| format!("{current}/{total}")).unwrap_or_else(|| "none".into())
}

fn timestamp() -> String {
    jiff::Timestamp::now().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use redb::TableDefinition;
    use tempfile::TempDir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_test_state() {
        *LAST_BOOTSTRAP_FAILURE.lock() = None;
        LAST_WALLET_MIGRATION_FAILURES.lock().clear();
        *LAST_KNOWN_WALLET_IDS.lock() = None;
    }

    fn create_redb_with_tables(path: &Path, tables: &[&'static str]) -> eyre::Result<()> {
        let db = redb::Database::create(path)?;
        let write_txn = db.begin_write()?;

        for table_name in tables {
            let table_def = TableDefinition::<&str, &str>::new(table_name);
            let mut table = write_txn.open_table(table_def)?;
            table.insert("seed material key", "secret descriptor address label")?;
        }

        write_txn.commit()?;

        Ok(())
    }

    #[test]
    fn already_called_does_not_replace_actionable_bootstrap_failure() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;

        reset_test_state();
        record_bootstrap_failure(&AppInitError::MainDatabaseMigration("root cause".into()));
        record_bootstrap_failure(&AppInitError::AlreadyCalled("second call".into()));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("Last failure category: main_database_migration"));
        assert!(
            report.contains("Last failure message: Main database migration failed: root cause")
        );
        assert!(!report.contains("Last failure category: already_called"));
        assert!(!report.contains("Last failure message: Bootstrap already called: second call"));

        Ok(())
    }

    #[test]
    fn already_called_records_when_no_prior_bootstrap_failure() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;

        reset_test_state();
        record_bootstrap_failure(&AppInitError::AlreadyCalled("second call".into()));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("Last failure category: already_called"));
        assert!(report.contains("Last failure message: Bootstrap already called: second call"));

        Ok(())
    }

    #[test]
    fn actionable_bootstrap_failure_replaces_already_called() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;

        reset_test_state();
        record_bootstrap_failure(&AppInitError::AlreadyCalled("first call".into()));
        record_bootstrap_failure(&AppInitError::WalletDatabaseMigration("root cause".into()));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("Last failure category: wallet_database_migration"));
        assert!(
            report.contains("Last failure message: Wallet database migration failed: root cause")
        );
        assert!(!report.contains("Last failure category: already_called"));
        assert!(!report.contains("Last failure message: Bootstrap already called: first call"));

        Ok(())
    }

    #[test]
    fn clear_bootstrap_failure_clears_known_wallet_ids() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        let known_dir = wallet_dir.join("known_wallet");
        std::fs::create_dir_all(&known_dir)?;

        reset_test_state();
        *LAST_KNOWN_WALLET_IDS.lock() = Some(BTreeSet::from(["known_wallet".to_string()]));

        clear_bootstrap_failure();
        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("known wallet ids: unavailable"));
        assert!(report.contains("known_wallet (unknown)"));
        assert!(!report.contains("known_wallet (known)"));

        Ok(())
    }

    #[test]
    fn report_describes_interrupted_files_without_row_contents() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        let known_dir = wallet_dir.join("known_wallet");
        let orphan_dir = wallet_dir.join("orphan_wallet");
        std::fs::create_dir_all(&known_dir)?;
        std::fs::create_dir_all(&orphan_dir)?;
        std::fs::write(dir.path().join("cove.encrypted.db.tmp"), b"secret row label")?;
        std::fs::write(known_dir.join("wallet_data.json"), b"address row contents")?;
        std::fs::write(orphan_dir.join("wallet_data.encrypted.json.redb.tmp"), b"tmp")?;

        reset_test_state();
        *LAST_KNOWN_WALLET_IDS.lock() = Some(BTreeSet::from(["known_wallet".to_string()]));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("known_wallet (known)"));
        assert!(report.contains("orphan_wallet (orphan)"));
        assert!(report.contains("cove.encrypted.db.tmp: present, bytes=16"));
        assert!(report.contains("wallet_data.encrypted.json.redb.tmp: present, bytes=3"));
        assert!(!report.contains("secret row label"));
        assert!(!report.contains("address row contents"));

        Ok(())
    }

    #[test]
    fn report_marks_unknown_main_tables_and_compares_destination() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;
        create_redb_with_tables(&dir.path().join("cove.db"), &["global_flag", "future_table"])?;
        create_redb_with_tables(&dir.path().join("cove.encrypted.db"), &["global_flag"])?;

        reset_test_state();

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("Main migration state: source and destination both present"));
        assert!(report.contains("future_table (unknown)"));
        assert!(report.contains("unknown tables: future_table"));
        assert!(report.contains("table comparison cove.db -> cove.encrypted.db:"));
        assert!(report.contains("missing in destination: future_table"));
        assert!(!report.contains("seed material key"));
        assert!(!report.contains("secret descriptor address label"));

        Ok(())
    }

    #[test]
    fn report_marks_historical_main_tables() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;
        create_redb_with_tables(
            &dir.path().join("cove.db"),
            &["global_flag", "global_bool_config"],
        )?;

        reset_test_state();

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("global_flag (current)"));
        assert!(report.contains("global_bool_config (historical)"));
        assert!(report.contains("historical skipped tables: global_bool_config"));
        assert!(!report.contains("unknown tables: global_bool_config"));

        Ok(())
    }

    #[test]
    fn report_marks_unknown_wallet_tables() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        let known_dir = wallet_dir.join("known_wallet");
        std::fs::create_dir_all(&known_dir)?;
        create_redb_with_tables(
            &known_dir.join("wallet_data.json"),
            &["wallet_data.json", "old_wallet_table"],
        )?;

        reset_test_state();
        *LAST_KNOWN_WALLET_IDS.lock() = Some(BTreeSet::from(["known_wallet".to_string()]));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("known_wallet (known)"));
        assert!(report.contains("migration state: needs plaintext migration"));
        assert!(report.contains("old_wallet_table (unknown)"));
        assert!(report.contains("unknown tables: old_wallet_table"));
        assert!(!report.contains("seed material key"));
        assert!(!report.contains("secret descriptor address label"));

        Ok(())
    }

    #[test]
    fn report_marks_historical_wallet_tables() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        let known_dir = wallet_dir.join("known_wallet");
        std::fs::create_dir_all(&known_dir)?;
        create_redb_with_tables(
            &known_dir.join("wallet_data.json"),
            &["wallet_data.json", "transaction_labels.json"],
        )?;

        reset_test_state();
        *LAST_KNOWN_WALLET_IDS.lock() = Some(BTreeSet::from(["known_wallet".to_string()]));

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("wallet_data.json (current)"));
        assert!(report.contains("transaction_labels.json (historical)"));
        assert!(report.contains("historical skipped tables: transaction_labels.json"));
        assert!(!report.contains("unknown tables: transaction_labels.json"));

        Ok(())
    }

    #[test]
    fn report_includes_schema_open_errors() -> eyre::Result<()> {
        let _guard = TEST_LOCK.lock();
        let dir = TempDir::new()?;
        let wallet_dir = dir.path().join("wallets");
        std::fs::create_dir_all(&wallet_dir)?;
        std::fs::write(dir.path().join("cove.db"), b"not a redb database")?;

        reset_test_state();

        let report = text_report_for_paths(dir.path(), &wallet_dir);

        assert!(report.contains("cove.db: present"));
        assert!(report.contains("schema unavailable: failed to open redb"));
        assert!(!report.contains("not a redb database"));

        Ok(())
    }
}
