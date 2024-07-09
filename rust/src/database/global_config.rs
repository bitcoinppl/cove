use std::sync::Arc;

use redb::TableDefinition;

use crate::{
    app::reconcile::{Update, Updater},
    wallet::{Network, WalletId},
};

use super::Error;

pub const TABLE: TableDefinition<&'static str, String> = TableDefinition::new("global_config");

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum GlobalConfigKey {
    SelectedWalletId,
    SelectedNetwork,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct GlobalConfigTable {
    db: Arc<redb::Database>,
}

impl GlobalConfigTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum GlobalConfigTableError {
    #[error("failed to save global config: {0}")]
    SaveError(String),

    #[error("failed to get global config: {0}")]
    ReadError(String),
}

#[uniffi::export]
impl GlobalConfigTable {
    pub fn select_wallet(&self, id: WalletId) -> Result<(), Error> {
        self.set(GlobalConfigKey::SelectedWalletId, id.to_string())?;

        Ok(())
    }

    pub fn selected_wallet(&self) -> Option<WalletId> {
        let id = self
            .get(GlobalConfigKey::SelectedWalletId)
            .unwrap_or(None)?;

        let wallet_id = WalletId::from(id);

        Some(wallet_id)
    }

    pub fn selected_network(&self) -> Network {
        let network = self
            .get(GlobalConfigKey::SelectedNetwork)
            .unwrap_or(None)
            .unwrap_or("bitcoin".to_string());

        let network = Network::try_from(network.as_str()).unwrap_or(Network::Bitcoin);

        network
    }

    pub fn set_selected_network(&self, network: Network) -> Result<(), Error> {
        self.set(GlobalConfigKey::SelectedNetwork, network.to_string())?;

        Ok(())
    }

    pub fn get(&self, key: GlobalConfigKey) -> Result<Option<String>, Error> {
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
            .map_err(|error| GlobalConfigTableError::ReadError(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    pub fn set(&self, key: GlobalConfigKey, value: String) -> Result<(), Error> {
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
                .map_err(|error| GlobalConfigTableError::SaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}
