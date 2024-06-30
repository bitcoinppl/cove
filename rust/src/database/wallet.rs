use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use redb::ReadableTableMetadata as _;

use crate::{
    update::{Update, Updater},
    view_model::wallet::WalletId,
};

use super::{Error, WalletKey, WALLETS};

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletTable {
    db: Arc<redb::Database>,
}

impl WalletTable {
    pub fn new(db: Arc<redb::Database>) -> Self {
        Self { db }
    }

    pub fn get(&self, network: Network) -> Result<Vec<WalletId>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(WALLETS)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let key: WalletKey = network.into();
        let key: &'static str = key.into();

        let value = table
            .get(key)
            .map_err(|error| Error::WalletsReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(value)
    }

    pub fn save(&self, network: Network, wallets: Vec<WalletId>) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(WALLETS)
                .map_err(|error| Error::TableAccessError(error.to_string()))?;

            let key: WalletKey = network.into();
            let key: &'static str = key.into();

            table
                .insert(key, wallets)
                .map_err(|error| Error::WalletsSaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdate);

        Ok(())
    }
}

#[uniffi::export]
impl WalletTable {
    pub fn is_empty(&self) -> Result<bool, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(WALLETS)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let count = table
            .is_empty()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Ok(count)
    }

    pub fn len(&self) -> Result<u16, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(WALLETS)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        let count = table
            .len()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        Ok(count as u16)
    }
}
