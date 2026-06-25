use std::sync::Arc;

use redb::TableDefinition;
use tracing::{debug, warn};

use cove_util::result_ext::ResultExt as _;

use crate::app::reconcile::{Update, Updater};

use super::Error;

pub const TABLE: TableDefinition<&'static str, bool> = TableDefinition::new("global_flag");
const LEGACY_ACCEPTED_TERMS_KEY: &str = "AcceptedTerms";

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalFlagKey {
    CompletedOnboarding,
    BetaFeaturesEnabled,
    BetaImportExportEnabled,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct GlobalFlagTable {
    db: Arc<redb::Database>,
}

impl GlobalFlagTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_table() -> (tempfile::TempDir, GlobalFlagTable) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = tmp.path().join("global_flag.redb");
        let db = Arc::new(redb::Database::create(db_path).expect("failed to create redb"));
        let write_txn = db.begin_write().expect("failed to begin write txn");
        let table = GlobalFlagTable::new(db, &write_txn);
        write_txn.commit().expect("failed to commit table creation");

        (tmp, table)
    }

    fn write_legacy_accepted_terms(table: &GlobalFlagTable, value: bool) {
        let write_txn = table.db.begin_write().expect("failed to begin write txn");

        {
            let mut redb_table = write_txn.open_table(TABLE).expect("failed to open table");
            redb_table
                .insert(LEGACY_ACCEPTED_TERMS_KEY, value)
                .expect("failed to write legacy terms flag");
        }

        write_txn.commit().expect("failed to commit legacy terms flag");
    }

    #[test]
    fn mark_onboarding_complete_persists_completed_flag() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        assert!(!table.is_onboarding_complete());

        table.mark_onboarding_complete().expect("failed to mark onboarding complete");

        assert!(table.is_onboarding_complete());
    }

    #[test]
    fn legacy_backfill_requires_accepted_terms_and_wallets() {
        let (_tmp, table) = test_table();

        table.backfill_onboarding_complete_from_legacy_state(true);
        assert!(!table.is_onboarding_complete());

        write_legacy_accepted_terms(&table, true);
        table.backfill_onboarding_complete_from_legacy_state(false);
        assert!(!table.is_onboarding_complete());

        table.backfill_onboarding_complete_from_legacy_state(true);
        assert!(table.is_onboarding_complete());
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum GlobalFlagTableError {
    #[error("failed to save global flag: {0}")]
    Save(String),

    #[error("failed to get global flag: {0}")]
    Read(String),
}

#[uniffi::export]
impl GlobalFlagTable {
    pub fn get(&self, key: GlobalFlagKey) -> Result<bool, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err_str(GlobalFlagTableError::Read)?
            .is_some_and(|value| value.value());

        Ok(value)
    }

    pub fn set(&self, key: GlobalFlagKey, value: bool) -> Result<(), Error> {
        self.set_inner(key, value, true)
    }

    fn set_inner(&self, key: GlobalFlagKey, value: bool, notify: bool) -> Result<(), Error> {
        debug!("setting global flag: {key:?} to {value}");
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

            let key: &'static str = key.into();
            table.insert(key, value).map_err_str(GlobalFlagTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        if notify {
            Updater::send_update(Update::DatabaseUpdated);
        }

        Ok(())
    }

    pub fn get_bool_config(&self, key: GlobalFlagKey) -> bool {
        self.get(key).unwrap_or(false)
    }

    pub fn set_bool_config(&self, key: GlobalFlagKey, value: bool) -> Result<(), Error> {
        self.set(key, value)
    }

    pub fn toggle_bool_config(&self, key: GlobalFlagKey) -> Result<(), Error> {
        let value = self.get(key)?;

        let new_value = !value;
        self.set(key, new_value)?;

        Ok(())
    }
}

impl GlobalFlagTable {
    pub(crate) fn is_onboarding_complete(&self) -> bool {
        self.get_bool_config(GlobalFlagKey::CompletedOnboarding)
    }

    pub(crate) fn mark_onboarding_complete(&self) -> Result<(), Error> {
        if self.is_onboarding_complete() {
            return Ok(());
        }

        self.set_inner(GlobalFlagKey::CompletedOnboarding, true, true)
    }

    pub(crate) fn backfill_onboarding_complete_from_legacy_state(&self, has_any_wallets: bool) {
        if self.is_onboarding_complete() || !self.is_legacy_terms_accepted() || !has_any_wallets {
            return;
        }

        if let Err(error) = self.set_inner(GlobalFlagKey::CompletedOnboarding, true, false) {
            warn!("failed to backfill completed onboarding flag: {error}");
        }
    }

    fn is_legacy_terms_accepted(&self) -> bool {
        let read_txn = match self.db.begin_read().map_err_str(Error::DatabaseAccess) {
            Ok(read_txn) => read_txn,
            Err(error) => {
                warn!("failed to read legacy accepted terms flag: {error}");
                return false;
            }
        };

        let table = match read_txn.open_table(TABLE).map_err_str(Error::TableAccess) {
            Ok(table) => table,
            Err(error) => {
                warn!("failed to open global flag table for legacy accepted terms: {error}");
                return false;
            }
        };

        match table.get(LEGACY_ACCEPTED_TERMS_KEY).map_err_str(GlobalFlagTableError::Read) {
            Ok(value) => value.is_some_and(|value| value.value()),
            Err(error) => {
                warn!("failed to read legacy accepted terms flag: {error}");
                false
            }
        }
    }
}
