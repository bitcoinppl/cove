use std::path::PathBuf;

use bdk_file_store::Store as FileStore;
use bdk_wallet::{KeychainKind, Wallet};
use bitcoin::Network;
use eyre::{Context as _, ContextCompat as _, Result};
use tracing::{info, warn};

use crate::{
    consts::ROOT_DATA_DIR,
    database::Database,
    wallet::metadata::{StoreType, WalletId},
};

pub struct BdkStore {
    id: WalletId,
    network: Network,
    pub conn: bdk_wallet::rusqlite::Connection,
}

impl BdkStore {
    pub fn try_new(id: &WalletId, network: impl Into<Network>) -> Result<Self> {
        let mut me = Self {
            id: id.clone(),
            network: network.into(),
            conn: bdk_wallet::rusqlite::Connection::open(sqlite_data_path(id))
                .context("unable to open connection to sqlite db")?,
        };

        if let Err(e) = me.check_and_migrate_from_file_store() {
            tracing::error!("{id} failed to migrate from file store: {e:?}");
            return Err(e);
        }

        Ok(me)
    }

    // check if we have a file store
    // if we do, migrate to the new SQLite store
    fn check_and_migrate_from_file_store(&mut self) -> Result<bool> {
        let id = &self.id;
        let network = self.network;

        if !file_store_data_path(id).exists() {
            return Ok(false);
        }

        // get the metadata for the wallet
        let mode = Database::global().global_config().wallet_mode();
        let Some(mut metadata) = Database::global()
            .wallets()
            .get(id, self.network.into(), mode)
            .context("unable to get metadata for wallet")?
        else {
            // if not metdata found this is a new wallet so we can just return
            return Ok(false);
        };

        tracing::debug!("wallet metadata: {:?}", metadata.internal.store_type);
        if metadata.internal.store_type == StoreType::Sqlite {
            return Ok(false);
        }

        warn!("{id} migrating wallet from file store");
        let mut file_store_db = FileStore::<bdk_wallet::ChangeSet>::open(
            id.to_string().as_bytes(),
            file_store_data_path(id),
        )
        .context("failed to open file store")?;

        let file_store_wallet = Wallet::load()
            .load_wallet(&mut file_store_db)
            .context("failed to load wallet")?
            .context("no wallet found")?;

        let external_descriptor = file_store_wallet.public_descriptor(KeychainKind::External);
        let change_descriptor = file_store_wallet.public_descriptor(KeychainKind::Internal);

        let mut persisted_wallet =
            Wallet::create(external_descriptor.clone(), change_descriptor.clone())
                .network(network)
                .create_wallet(&mut self.conn)
                .context("failed to create wallet")?;

        persisted_wallet
            .persist(&mut self.conn)
            .context("failed to persist wallet")?;

        // reset metadata scanning state to default so we force a full scan
        metadata.internal.last_scan_finished = None;
        metadata.internal.last_height_fetched = None;
        metadata.internal.performed_full_scan_at = None;
        metadata.internal.store_type = StoreType::Sqlite;

        Database::global()
            .wallets()
            .update_wallet_metadata(metadata)
            .context("unable to save updated metadata")?;

        // TODO: put back when we are sure this works
        // std::fs::remove_file(file_store_data_path(id)).context("unable to delete filestore")?;

        info!("completed migrating from file store to sqlite store");

        Ok(true)
    }

    pub fn delete_wallet_stores(wallet_id: &WalletId) -> Result<()> {
        let file_store_data_path = file_store_data_path(wallet_id);
        let sqlite_data_path = sqlite_data_path(wallet_id);

        if file_store_data_path.exists() {
            std::fs::remove_file(file_store_data_path).context("unable to delete filestore")?;
        }

        if sqlite_data_path.exists() {
            std::fs::remove_file(sqlite_data_path).context("unable to delete sqlite store")?;
        }

        Ok(())
    }

    pub fn delete_sqlite_store(wallet_id: &WalletId) -> Result<()> {
        let sqlite_data_path = sqlite_data_path(wallet_id);

        if sqlite_data_path.exists() {
            std::fs::remove_file(sqlite_data_path).context("unable to delete sqlite store")?;
        }

        Ok(())
    }
}

fn file_store_data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}

fn sqlite_data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_sqlite_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}
