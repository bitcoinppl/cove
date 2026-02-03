use std::sync::Arc;

use redb::TableDefinition;
use tracing::debug;

use cove_util::result_ext::ResultExt as _;

use crate::app::reconcile::{Update, Updater};

use super::Error;

pub const TABLE: TableDefinition<&'static str, bool> = TableDefinition::new("global_flag");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalFlagKey {
    CompletedOnboarding,
    AcceptedTerms,
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
        debug!("setting global flag: {key:?} to {value}");
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

            let key: &'static str = key.into();
            table.insert(key, value).map_err_str(GlobalFlagTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub fn is_terms_accepted(&self) -> bool {
        self.get_bool_config(GlobalFlagKey::AcceptedTerms)
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
