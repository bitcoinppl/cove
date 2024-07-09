use std::sync::Arc;

use redb::TableDefinition;

use crate::app::reconcile::{Update, Updater};

use super::Error;

pub const TABLE: TableDefinition<&'static str, bool> = TableDefinition::new("global_flag");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalFlagKey {
    CompletedOnboarding,
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
pub enum GlobalFlagTableError {
    #[error("failed to save global flag: {0}")]
    SaveError(String),

    #[error("failed to get global flag: {0}")]
    ReadError(String),
}

#[uniffi::export]
impl GlobalFlagTable {
    pub fn get(&self, key: GlobalFlagKey) -> Result<bool, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| GlobalFlagTableError::ReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or(false);

        Ok(value)
    }

    pub fn set(&self, key: GlobalFlagKey, value: bool) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccessError(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| GlobalFlagTableError::SaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub fn toggle_bool_config(&self, key: GlobalFlagKey) -> Result<(), Error> {
        let value = self.get(key)?;

        let new_value = !value;
        self.set(key, new_value)?;

        Ok(())
    }
}
