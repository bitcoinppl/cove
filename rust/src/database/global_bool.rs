use std::sync::Arc;

use crate::update::{Update, Updater};

use super::{Error, GLOBAL_BOOL_CONFIG};

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalBoolConfigKey {
    CompletedOnboarding,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct GlobalBoolTable {
    db: Arc<redb::Database>,
}

impl GlobalBoolTable {
    pub fn new(db: Arc<redb::Database>) -> Self {
        Self { db }
    }
}

#[uniffi::export]
impl GlobalBoolTable {
    pub fn get_bool_config(&self, key: GlobalBoolConfigKey) -> Result<bool, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(GLOBAL_BOOL_CONFIG)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| Error::ConfigReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or(false);

        Ok(value)
    }

    pub fn set_bool_config(&self, key: GlobalBoolConfigKey, value: bool) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(GLOBAL_BOOL_CONFIG)
                .map_err(|error| Error::TableAccessError(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| Error::ConfigSaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdate);

        Ok(())
    }

    pub fn toggle_bool_config(&self, key: GlobalBoolConfigKey) -> Result<(), Error> {
        let value = self.get_bool_config(key)?;

        let new_value = !value;
        self.set_bool_config(key, new_value)?;

        Ok(())
    }
}
