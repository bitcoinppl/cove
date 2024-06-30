use std::{fmt::Display, sync::Arc};

use bdk_wallet::bitcoin::Network;
use redb::{ReadOnlyTable, ReadableTableMetadata as _, TableDefinition};

use crate::{
    update::{Update, Updater},
    view_model::wallet::WalletId,
};

use super::Error;

const TABLE: TableDefinition<&'static str, Vec<WalletId>> = TableDefinition::new("wallets");
pub const VERSION: Version = Version(1);

#[derive(Debug, Clone, derive_more::Display, derive_more::From, derive_more::FromStr)]
pub struct Version(u32);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletTableError {
    #[error("failed to save wallets: {0}")]
    SaveError(String),

    #[error("failed to get wallets: {0}")]
    ReadError(String),
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum WalletKey {
    Bitcoin,
    Testnet,
}

impl Display for WalletKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.for_version(VERSION))
    }
}

impl WalletKey {
    pub fn for_version(&self, version: Version) -> String {
        // do it here, so only way to get to string is to call `to_string`
        let network = match self {
            WalletKey::Bitcoin => "bitcoin",
            WalletKey::Testnet => "testnet",
        };

        format!("{network}:{version}")
    }
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
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }

    pub fn get(&self, network: Network) -> Result<Vec<WalletId>, Error> {
        let table = self.read_table()?;
        let key = WalletKey::from(network).to_string();

        let value = table
            .get(&*key)
            .map_err(|error| WalletTableError::ReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(value)
    }

    pub fn save(&self, network: Network, wallets: Vec<WalletId>) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;

        {
            let mut table = write_txn.open_table(TABLE)?;
            let key = WalletKey::from(network).to_string();
            table
                .insert(&*key, wallets)
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
            .open_table(TABLE)
            .map_err(|error| Error::TableAccessError(error.to_string()))?;

        Ok(table)
    }
}
