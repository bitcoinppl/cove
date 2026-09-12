//! Module for interacting with redb database, to store high level state, and non sensitive data.
//! That will be available across the app, and will be persisted across app launches.

pub mod cbor;
pub mod cloud_backup;
pub mod diagnostics_reports;
pub mod encrypted_backend;
pub mod error;
pub mod global_cache;
pub mod global_config;
pub mod global_flag;
pub mod historical_price;
pub mod key;
pub mod macros;
pub mod migration;
pub mod record;
pub mod unsigned_transactions;
pub mod wallet;
pub mod wallet_data;

use cove_util::result_ext::ResultExt as _;
use std::{path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use cloud_backup::{
    CloudBackupStateTable, CloudBlobSyncStateTable, ensure_table_type_compatibility,
};
use diagnostics_reports::DiagnosticsReportsTable;
use global_cache::GlobalCacheTable;
use global_config::{GlobalConfigKey, GlobalConfigTable};
use global_flag::GlobalFlagTable;
use historical_price::HistoricalPriceTable;
use uniffi::custom_newtype;
use unsigned_transactions::UnsignedTransactionsTable;
use wallet::WalletsTable;

use once_cell::sync::OnceCell;
use tracing::{error, warn};

use cove_common::consts::ROOT_DATA_DIR;

pub static DATABASE: OnceCell<ArcSwap<Database>> = OnceCell::new();
static DATABASE_LOCATION_OVERRIDE: OnceCell<PathBuf> = OnceCell::new();

pub type Error = error::DatabaseError;
pub type Record<T> = record::Record<T>;

#[derive(Debug, Clone, uniffi::Object)]
pub struct Database {
    pub global_flag: GlobalFlagTable,
    pub global_config: GlobalConfigTable,
    pub global_cache: GlobalCacheTable,
    pub cloud_backup_state: CloudBackupStateTable,
    pub cloud_blob_sync_states: CloudBlobSyncStateTable,
    pub wallets: WalletsTable,
    pub unsigned_transactions: UnsignedTransactionsTable,
    pub historical_prices: HistoricalPriceTable,
    pub diagnostics_reports: DiagnosticsReportsTable,
}

#[uniffi::export]
impl Database {
    #[uniffi::constructor(name = "new")]
    pub fn new() -> Arc<Self> {
        Self::global()
    }

    pub fn wallets(&self) -> WalletsTable {
        self.wallets.clone()
    }

    pub fn global_config(&self) -> GlobalConfigTable {
        self.global_config.clone()
    }

    pub fn global_flag(&self) -> GlobalFlagTable {
        self.global_flag.clone()
    }

    pub fn unsigned_transactions(&self) -> UnsignedTransactionsTable {
        self.unsigned_transactions.clone()
    }

    pub fn historical_prices(&self) -> HistoricalPriceTable {
        self.historical_prices.clone()
    }

    pub fn diagnostics_reports(&self) -> DiagnosticsReportsTable {
        self.diagnostics_reports.clone()
    }

    pub fn dangerous_reset_all_data(&self) -> Result<(), error::DatabaseError> {
        let completed_onboarding = self.global_flag.try_is_onboarding_complete()?;

        match std::fs::remove_file(database_location()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error::DatabaseError::DatabaseAccess(format!(
                    "unable to delete database cove_main: {error}"
                )));
            }
        }

        if let Err(error) = cove_common::logging::capture::clear_default_logs_dir() {
            error!("unable to clear diagnostics logs during data reset: {error}");
        }

        let db = Self::init_with_completed_onboarding(completed_onboarding)?;
        DATABASE.get().expect("database not initialized").swap(Arc::new(db));

        Ok(())
    }
}

impl Database {
    pub fn global() -> Arc<Self> {
        Self::try_global().expect("failed to initialize main database")
    }

    pub(crate) fn initialize_for_bootstrap() -> Result<(), error::DatabaseError> {
        Self::try_global().map(drop)
    }

    fn try_global() -> Result<Arc<Self>, error::DatabaseError> {
        let db = DATABASE.get_or_try_init(|| {
            let db = Self::init()?;

            Ok::<_, error::DatabaseError>(ArcSwap::new(Arc::new(db)))
        })?;

        let db = db.load();

        Ok(Arc::clone(&db))
    }

    /// Re-open the database file and swap the global handle
    ///
    /// Used by worst-case recovery paths that need to surface errors instead of panicking
    pub fn try_reinit() -> Result<(), error::DatabaseError> {
        let Some(arc_swap) = DATABASE.get() else {
            return Ok(());
        };

        let db = Self::init()?;
        arc_swap.swap(Arc::new(db));
        Ok(())
    }

    fn init() -> Result<Self, error::DatabaseError> {
        Self::init_with_completed_onboarding(false)
    }

    fn init_with_completed_onboarding(
        completed_onboarding: bool,
    ) -> Result<Self, error::DatabaseError> {
        crate::bootstrap::ensure_storage_bootstrapped()
            .map_err_str(error::DatabaseError::BootstrapFailed)?;

        let main_db = get_or_create_main_database()?;
        let main_db_arc = Arc::new(main_db);

        let write_txn = main_db_arc.begin_write()?;
        ensure_table_type_compatibility(&write_txn)?;

        let wallets = WalletsTable::new(main_db_arc.clone(), &write_txn);
        let global_flag = GlobalFlagTable::new(main_db_arc.clone(), &write_txn);
        let global_config = GlobalConfigTable::new(main_db_arc.clone(), &write_txn);
        let global_cache = GlobalCacheTable::new(main_db_arc.clone(), &write_txn);
        let cloud_backup_state = CloudBackupStateTable::new(main_db_arc.clone(), &write_txn);
        let cloud_blob_sync_states = CloudBlobSyncStateTable::new(main_db_arc.clone(), &write_txn);
        let unsigned_transactions = UnsignedTransactionsTable::new(main_db_arc.clone(), &write_txn);
        let historical_prices = HistoricalPriceTable::new(main_db_arc.clone(), &write_txn);
        let diagnostics_reports = DiagnosticsReportsTable::new(main_db_arc, &write_txn);

        if completed_onboarding {
            global_flag.set_onboarding_complete_in_transaction(&write_txn)?;
        }

        write_txn.commit()?;

        let database = Self {
            global_flag,
            global_config,
            global_cache,
            cloud_backup_state,
            cloud_blob_sync_states,
            wallets,
            unsigned_transactions,
            historical_prices,
            diagnostics_reports,
        };

        database.backfill_onboarding_complete_from_legacy_state();

        Ok(database)
    }

    fn backfill_onboarding_complete_from_legacy_state(&self) {
        let has_any_wallets = match self.wallets.has_any_wallets() {
            Ok(has_any_wallets) => has_any_wallets,
            Err(error) => {
                warn!("failed to inspect wallets while backfilling onboarding flag: {error}");
                return;
            }
        };

        let has_persisted_onboarding_progress = match self
            .global_config
            .get(GlobalConfigKey::OnboardingProgress)
        {
            Ok(progress) => progress.is_some(),
            Err(error) => {
                warn!(
                    "failed to inspect onboarding progress while backfilling onboarding flag: {error}"
                );
                return;
            }
        };

        self.global_flag.backfill_onboarding_complete_from_legacy_state(
            has_any_wallets,
            has_persisted_onboarding_progress,
        );
    }
}

fn get_or_create_main_database() -> Result<redb::Database, error::DatabaseError> {
    encrypted_backend::open_or_create_database(&database_location())
}

fn database_location() -> PathBuf {
    DATABASE_LOCATION_OVERRIDE
        .get()
        .cloned()
        .unwrap_or_else(|| ROOT_DATA_DIR.join("cove.encrypted.db"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::*;

    const TEST_DATA_DIR_PREFIX: &str = "cove-test-";
    const STALE_TEST_DATA_DIR_AGE: Duration = Duration::from_secs(60 * 60);

    pub(crate) fn init_test_database() {
        let root = process_test_data_dir();
        crate::bootstrap::tests::set_test_bootstrapped();
        crate::app::reconcile::test_support::init_noop_updater();
        let _ = DATABASE_LOCATION_OVERRIDE.set(root.join("cove.encrypted.db"));
    }

    pub(crate) fn delete_database() {
        init_test_database();
        let db_path = database_location();
        let wallet_data_dir = cove_common::consts::wallet_data_dir_path();

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&wallet_data_dir);

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to recreate test dir");
        }

        std::fs::create_dir_all(wallet_data_dir).expect("failed to recreate wallet data test dir");
    }

    fn process_test_data_dir() -> &'static PathBuf {
        static TEST_ROOT: OnceLock<PathBuf> = OnceLock::new();

        TEST_ROOT.get_or_init(|| {
            let parent = std::env::temp_dir();
            sweep_stale_test_data_dirs(&parent);
            remove_legacy_home_test_dir();

            let tempdir = tempfile::Builder::new()
                .prefix(TEST_DATA_DIR_PREFIX)
                .tempdir()
                .expect("failed to create test data directory");

            // keep the directory for the process lifetime; Drop would delete it too early
            let path = tempdir.keep();

            let _ = cove_common::consts::set_root_data_dir(path.clone());
            path
        })
    }

    fn sweep_stale_test_data_dirs(parent: &Path) {
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };

        let now = std::time::SystemTime::now();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };

            if !name.starts_with(TEST_DATA_DIR_PREFIX) {
                continue;
            }

            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            if !metadata.is_dir() {
                continue;
            }

            let Ok(modified) = metadata.modified() else {
                continue;
            };

            let Ok(age) = now.duration_since(modified) else {
                continue;
            };

            // nextest runs binaries in parallel, so only remove dirs older than one hour
            if age > STALE_TEST_DATA_DIR_AGE {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    fn remove_legacy_home_test_dir() {
        let Some(home) = dirs::home_dir() else {
            return;
        };

        let _ = std::fs::remove_dir_all(home.join(".data").join("test"));
    }

    #[test]
    fn test_database_lives_under_temp_dir_not_home_data() {
        init_test_database();
        let path = database_location();
        let temp = std::env::temp_dir();

        assert!(path.starts_with(&temp), "test database {path:?} must be under {temp:?}");

        if let Some(home) = dirs::home_dir() {
            let home_data = home.join(".data");
            assert!(
                !path.starts_with(&home_data),
                "test database {path:?} must not be under {home_data:?}"
            );
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum InsertOrUpdate {
    Insert(Timestamp),
    Update(Timestamp),
}

#[derive(Debug, Clone, Copy, derive_more::From, derive_more::AsRef, derive_more::Into)]
pub struct Timestamp(u64);
custom_newtype!(Timestamp, u64);
