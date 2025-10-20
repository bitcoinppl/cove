use std::{fmt::Display, sync::Arc, time::Duration};

use redb::{ReadOnlyTable, ReadableTableMetadata, TableDefinition};
use tracing::debug;

use cove_util::result_ext::ResultExt as _;

use crate::{
    app::reconcile::{AppStateReconcileMessage, Update, Updater},
    network::Network,
    wallet::metadata::{HardwareWalletMetadata, WalletMetadata, WalletMode},
};

use super::{Database, Error};
use cove_types::WalletId;
use cove_types::redb::Json;

const TABLE: TableDefinition<&'static str, Json<Vec<WalletMetadata>>> =
    TableDefinition::new("wallets.json");

pub const VERSION: Version = Version(1);

#[derive(Debug, Clone, Copy, derive_more::Display, derive_more::From, derive_more::FromStr)]
pub struct Version(u32);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum WalletTableError {
    #[error("failed to save wallets: {0}")]
    SaveError(String),

    #[error("failed to get wallets: {0}")]
    ReadError(String),

    #[error("wallet already exists")]
    WalletAlreadyExists,
}

#[derive(Debug, Clone, Copy, uniffi::Object)]
pub struct WalletKey(Network, Version, WalletMode);

impl Display for WalletKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.2 == WalletMode::Main {
            write!(f, "{}::{}", self.0, self.1)
        } else {
            write!(f, "DECOY::{}::{}", self.0, self.1)
        }
    }
}

impl From<(Network, WalletMode)> for WalletKey {
    fn from((network, mode): (Network, WalletMode)) -> Self {
        WalletKey(network, VERSION, mode)
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct WalletsTable {
    db: Arc<redb::Database>,
}

#[uniffi::export]
impl WalletsTable {
    pub fn is_empty(&self) -> Result<bool, Error> {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        let table = self.read_table()?;
        if table.is_empty()? {
            return Ok(true);
        }

        Ok(self.len(network, wallet_mode)? == 0)
    }

    pub fn len(&self, network: Network, mode: WalletMode) -> Result<u16, Error> {
        let count = self.get_all(network, mode).map(|wallets| wallets.len() as u16)?;

        Ok(count)
    }

    pub fn all(&self) -> Result<Vec<WalletMetadata>, Error> {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        debug!("getting all wallets for {network}");
        let wallets = self.get_all(network, wallet_mode)?;

        Ok(wallets)
    }

    pub fn all_sorted_active(&self) -> Result<Vec<WalletMetadata>, Error> {
        let mut wallets = self.all()?;

        wallets.sort_unstable_by(|a, b| {
            let a_last_scan = a.internal.last_scan_finished.unwrap_or(Duration::ZERO);
            let b_last_scan = b.internal.last_scan_finished.unwrap_or(Duration::ZERO);

            // largest to smallest
            a_last_scan.cmp(&b_last_scan).reverse()
        });

        Ok(wallets)
    }
}

impl WalletsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
    }

    pub fn mark_wallet_as_verified(&self, id: &WalletId) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        wallets.iter_mut().for_each(|wallet| {
            if &wallet.id == id {
                wallet.verified = true;
            }
        });

        self.save_all_wallets(network, mode, wallets)?;
        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    /// Get a wallet by id for that network
    pub fn get(
        &self,
        id: &WalletId,
        network: Network,
        mode: WalletMode,
    ) -> Result<Option<WalletMetadata>, Error> {
        let wallets = self.get_all(network, mode)?;
        let wallet = wallets.into_iter().find(|wallet| &wallet.id == id);

        Ok(wallet)
    }

    /// Get all wallets for a network
    pub fn get_all(
        &self,
        network: Network,
        mode: WalletMode,
    ) -> Result<Vec<WalletMetadata>, Error> {
        let table = self.read_table()?;
        let key = WalletKey::from((network, mode)).to_string();

        let value = table
            .get(key.as_str())
            .map_err_str(WalletTableError::ReadError)?
            .map(|value| value.value())
            .unwrap_or(vec![]);

        Ok(value)
    }

    pub fn update_wallet_metadata(&self, metadata: WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        wallets.iter_mut().for_each(|wallet| {
            if wallet.id == metadata.id {
                *wallet = metadata.clone();
            }
        });

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    // update just the discovery state
    pub fn update_metadata_discovery_state(&self, metadata: &WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        wallets.iter_mut().for_each(|wallet| {
            if metadata.id == wallet.id {
                wallet.discovery_state = metadata.discovery_state.clone();
            }
        });

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn update_internal_metadata(&self, metadata: &WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        wallets.iter_mut().for_each(|wallet| {
            if wallet.id == metadata.id {
                wallet.internal = metadata.internal.clone();
            }
        });

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn delete(&self, id: &WalletId) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let mut wallets = self.get_all(network, mode)?;

        wallets.retain(|wallet| &wallet.id != id);
        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn save_new_wallet_metadata(&self, wallet: WalletMetadata) -> Result<(), Error> {
        let network = wallet.network;
        let mode = wallet.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        if wallets.iter().any(|w| w.id == wallet.id) {
            return Err(WalletTableError::WalletAlreadyExists.into());
        }

        wallets.push(wallet);
        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn save_all_wallets(
        &self,
        network: Network,
        mode: WalletMode,
        wallets: Vec<WalletMetadata>,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;

        {
            let mut table = write_txn.open_table(TABLE)?;
            let key = WalletKey::from((network, mode)).to_string();

            table.insert(&*key, wallets).map_err_str(WalletTableError::SaveError)?;
        }

        write_txn.commit().map_err_str(WalletTableError::SaveError)?;

        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        Ok(())
    }

    pub fn find_by_tap_signer_ident(
        &self,
        ident: &str,
        network: Network,
        mode: WalletMode,
    ) -> Result<Option<WalletMetadata>, Error> {
        let wallets = self.get_all(network, mode)?;

        let wallet = wallets.into_iter().find(|wallet| {
            wallet
                .hardware_metadata
                .as_ref()
                .map(|hw| match hw {
                    HardwareWalletMetadata::TapSigner(t) => t.card_ident == ident,
                })
                .unwrap_or(false)
        });

        Ok(wallet)
    }

    fn read_table<'a>(&self) -> Result<ReadOnlyTable<&'a str, Json<Vec<WalletMetadata>>>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(TABLE).map_err_str(Error::TableAccess)?;

        Ok(table)
    }
}

// redb::Key for WalletId is now implemented in the cove-types crate
