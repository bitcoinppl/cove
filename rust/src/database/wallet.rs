use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use redb::{ReadOnlyTable, ReadableTableMetadata as _};

use crate::{
    update::{Update, Updater},
    view_model::wallet::WalletId,
};

use super::{Error, WALLETS};

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletTableError {
    #[error("failed to save wallets: {0}")]
    SaveError(String),

    #[error("failed to get wallets: {0}")]
    ReadError(String),
}

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, uniffi::Enum)]
pub enum WalletKey {
    Bitcoin,
    Testnet,
}

impl From<Network> for WalletKey {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => WalletKey::Bitcoin,
            Network::Testnet => WalletKey::Testnet,
            other => panic!("unsupported network: {other:?}"),
        }
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletTable {
    db: Arc<redb::Database>,
}

#[uniffi::export]
impl WalletTable {
    pub fn is_empty(&self) -> Result<bool, Error> {
        let table = self.read_table()?;
        let is_empty = table.is_empty()?;

        Ok(is_empty)
    }

    pub fn len(&self) -> Result<u16, Error> {
        let table = self.read_table()?;
        let count = table.len()?;

        Ok(count as u16)
    }
}

impl WalletTable {
    pub fn new(db: Arc<redb::Database>) -> Self {
        Self { db }
    }

    pub fn get(&self, network: Network) -> Result<Vec<WalletId>, Error> {
        let table = self.read_table()?;

        let key: WalletKey = network.into();
        let key: &'static str = key.into();

        let value = table
            .get(key)
            .map_err(|error| WalletTableError::ReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(value)
    }

    pub fn save(&self, network: Network, wallets: Vec<WalletId>) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;

        {
            let mut table = write_txn.open_table(WALLETS)?;

            let key: WalletKey = network.into();
            let key: &'static str = key.into();

            table
                .insert(key, wallets)
                .map_err(|error| WalletTableError::SaveError(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| WalletTableError::SaveError(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdate);

        Ok(())
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Vec<WalletId>>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccessError(error.to_string()))?;

        let table = read_txn
            .open_table(WALLETS)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        Ok(table)
    }
}
