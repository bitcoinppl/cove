use std::{fmt::Display, sync::Arc};

use log::debug;
use redb::{ReadOnlyTable, ReadableTableMetadata, TableDefinition};

use crate::{
    app::reconcile::{AppStateReconcileMessage, Updater},
    redb::Json,
    wallet::{Network, WalletId, WalletMetadata},
};

use super::{Database, Error};

const TABLE: TableDefinition<&'static str, Json<Vec<WalletMetadata>>> =
    TableDefinition::new("wallets.json");

pub const VERSION: Version = Version(1);

#[derive(Debug, Clone, Copy, derive_more::Display, derive_more::From, derive_more::FromStr)]
pub struct Version(u32);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletTableError {
    #[error("failed to save wallets: {0}")]
    SaveError(String),

    #[error("failed to get wallets: {0}")]
    ReadError(String),
}

#[derive(Debug, Clone, Copy, uniffi::Object)]
pub struct WalletKey(Network, Version);

impl Display for WalletKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.0, self.1)
    }
}

impl From<Network> for WalletKey {
    fn from(network: Network) -> Self {
        WalletKey(network, VERSION)
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletTable {
    db: Arc<redb::Database>,
}

#[uniffi::export]
impl WalletTable {
    pub fn is_empty(&self) -> Result<bool, Error> {
        let network = Database::global().global_config.selected_network();

        let table = self.read_table()?;
        if table.is_empty()? {
            return Ok(true);
        }

        Ok(self.len(network)? == 0)
    }

    pub fn len(&self, network: Network) -> Result<u16, Error> {
        let count = self.get(network).map(|wallets| wallets.len() as u16)?;
        Ok(count)
    }

    pub fn all(&self) -> Result<Vec<WalletMetadata>, Error> {
        let network = Database::global().global_config.selected_network();

        debug!("getting all wallets for {network}");
        let wallets = self.get(network)?;

        Ok(wallets)
    }
}

impl WalletTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }

    pub fn get_selected_wallet(
        &self,
        id: WalletId,
        network: Network,
    ) -> Result<Option<WalletMetadata>, Error> {
        let table = self.read_table()?;
        let key = WalletKey::from(network).to_string();

        let value = table
            .get(key.as_str())
            .map_err(|error| WalletTableError::ReadError(error.to_string()))?
            .map(|value| value.value())
            .expect("wallets not found");

        let wallet_metadata = value.iter().find(|wallet| wallet.id == id).cloned();

        Ok(wallet_metadata)
    }

    pub fn mark_wallet_as_verified(&self, id: WalletId) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mut wallets = self.get(network)?;

        // update the wallet
        wallets.iter_mut().for_each(|wallet| {
            if wallet.id == id {
                wallet.verified = true;
            }
        });

        self.save(Network::Bitcoin, wallets)?;

        Ok(())
    }

    pub fn get(&self, network: Network) -> Result<Vec<WalletMetadata>, Error> {
        let table = self.read_table()?;
        let key = WalletKey::from(network).to_string();

        let value = table
            .get(key.as_str())
            .map_err(|error| WalletTableError::ReadError(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or(vec![]);

        Ok(value)
    }

    pub fn save(&self, network: Network, wallets: Vec<WalletMetadata>) -> Result<(), Error> {
        assert!(!wallets.is_empty());

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

        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        Ok(())
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Json<Vec<WalletMetadata>>>, Error> {
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
