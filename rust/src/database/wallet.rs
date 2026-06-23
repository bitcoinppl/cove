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
use crate::manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER;
use cove_types::WalletId;
use cove_types::redb::Json;

pub(crate) const TABLE: TableDefinition<&'static str, Json<Vec<WalletMetadata>>> =
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

#[derive(Debug, Clone, Copy)]
enum InternalMetadataUpdate {
    PreserveExisting,
    Replace,
}

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
        Self(network, VERSION, mode)
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

    /// Check if any wallets exist across all networks and modes
    pub fn has_any_wallets(&self) -> Result<bool, Error> {
        use strum::IntoEnumIterator;

        let table = self.read_table()?;
        if table.is_empty()? {
            return Ok(false);
        }

        for network in Network::iter() {
            for mode in WalletMode::iter() {
                if self.len(network, mode)? > 0 {
                    return Ok(true);
                }
            }
        }

        Ok(false)
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
    fn save_new_wallet_metadata_with_backup_behavior(
        &self,
        wallet: WalletMetadata,
        should_backup_to_cloud: bool,
    ) -> Result<(), Error> {
        let network = wallet.network;
        let mode = wallet.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        if wallets.iter().any(|w| w.id == wallet.id) {
            return Err(WalletTableError::WalletAlreadyExists.into());
        }

        let wallet_for_backup = should_backup_to_cloud.then(|| wallet.clone());
        wallets.push(wallet);
        self.save_all_wallets(network, mode, wallets)?;

        Updater::send_update(Update::WalletsChanged);
        if let Some(wallet_for_backup) = wallet_for_backup {
            CLOUD_BACKUP_MANAGER.backup_new_wallet(wallet_for_backup);
        }

        Ok(())
    }

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
        for wallet in &mut wallets {
            if &wallet.id == id {
                wallet.verified = true;
            }
        }

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

    pub fn update_wallet_metadata(
        &self,
        metadata: WalletMetadata,
    ) -> Result<WalletMetadata, Error> {
        self.update_wallet_metadata_inner(metadata, InternalMetadataUpdate::PreserveExisting)
    }

    pub(crate) fn replace_wallet_metadata(
        &self,
        metadata: WalletMetadata,
    ) -> Result<WalletMetadata, Error> {
        self.update_wallet_metadata_inner(metadata, InternalMetadataUpdate::Replace)
    }

    fn update_wallet_metadata_inner(
        &self,
        mut metadata: WalletMetadata,
        internal_update: InternalMetadataUpdate,
    ) -> Result<WalletMetadata, Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        for wallet in &mut wallets {
            if wallet.id == metadata.id {
                if matches!(internal_update, InternalMetadataUpdate::PreserveExisting) {
                    metadata.internal = wallet.internal.clone();
                }

                *wallet = metadata.clone();
            }
        }

        self.save_all_wallets(network, mode, wallets)?;

        Ok(metadata)
    }

    // update just the discovery state
    pub fn update_metadata_discovery_state(&self, metadata: &WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        for wallet in &mut wallets {
            if metadata.id == wallet.id {
                wallet.discovery_state = metadata.discovery_state.clone();
            }
        }

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn update_internal_metadata(&self, metadata: &WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        for wallet in &mut wallets {
            if wallet.id == metadata.id {
                wallet.internal = metadata.internal.clone();
            }
        }

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
    }

    pub fn delete(&self, id: &WalletId) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let mut wallets = self.get_all(network, mode)?;

        wallets.retain(|wallet| &wallet.id != id);
        self.save_all_wallets(network, mode, wallets)?;

        Updater::send_update(Update::WalletsChanged);

        Ok(())
    }

    pub fn save_new_wallet_metadata(&self, wallet: WalletMetadata) -> Result<(), Error> {
        self.save_new_wallet_metadata_with_backup_behavior(wallet, true)
    }

    pub fn save_restored_wallet_metadata(&self, wallet: WalletMetadata) -> Result<(), Error> {
        self.save_new_wallet_metadata_with_backup_behavior(wallet, false)
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
            wallet.hardware_metadata.as_ref().is_some_and(|hw| match hw {
                HardwareWalletMetadata::TapSigner(t) => t.card_ident == ident,
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet_table() -> (tempfile::TempDir, WalletsTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = WalletsTable::new(db, &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }

    #[test]
    fn update_wallet_metadata_preserves_existing_internal_metadata() {
        let (_tmp, table) = wallet_table();
        let mut stored = WalletMetadata::preview_new();
        stored.internal.last_scan_finished = Some(Duration::from_secs(10));
        stored.internal.performed_full_scan_at = Some(20);

        table.save_all_wallets(stored.network, stored.wallet_mode, vec![stored.clone()]).unwrap();

        let mut stale_update = stored.clone();
        stale_update.name = "renamed wallet".to_string();
        stale_update.internal = Default::default();

        let updated = table.update_wallet_metadata(stale_update).unwrap();
        let persisted = table.get(&stored.id, stored.network, stored.wallet_mode).unwrap().unwrap();

        assert_eq!(updated.name, "renamed wallet");
        assert_eq!(updated.internal, stored.internal);
        assert_eq!(persisted.internal, stored.internal);
    }

    #[test]
    fn replace_wallet_metadata_allows_internal_metadata_reset() {
        let (_tmp, table) = wallet_table();
        let mut stored = WalletMetadata::preview_new();
        stored.internal.last_scan_finished = Some(Duration::from_secs(10));
        stored.internal.performed_full_scan_at = Some(20);

        table.save_all_wallets(stored.network, stored.wallet_mode, vec![stored.clone()]).unwrap();

        let mut replacement = stored.clone();
        replacement.internal = Default::default();

        let updated = table.replace_wallet_metadata(replacement).unwrap();
        let persisted = table.get(&stored.id, stored.network, stored.wallet_mode).unwrap().unwrap();

        assert_eq!(updated.internal, Default::default());
        assert_eq!(persisted.internal, Default::default());
    }
}
