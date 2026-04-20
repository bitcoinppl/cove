use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    sync::Arc,
    time::Duration,
};

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

    #[error("invalid wallet reorder: {0}")]
    InvalidWalletReorder(String),
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

    /// Persist a new wallet order for the active wallet list.
    ///
    /// Validation rules:
    /// - `ordered_ids` must be a full permutation of existing wallet IDs in the active bucket.
    /// - Partial lists are rejected.
    /// - Unknown IDs are rejected.
    /// - Duplicate IDs are rejected.
    ///
    /// The write is atomic-like at the application level: validation and reorder construction
    /// happen before `save_all_wallets` is called, so invalid inputs do not mutate persisted state.
    pub fn reorder_wallets(&self, ordered_ids: Vec<WalletId>) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let wallets = self.get_all(network, mode)?;
        let reordered = build_reordered_wallets(&ordered_ids, wallets)?;

        self.save_all_wallets(network, mode, reordered)?;
        Updater::send_update(Update::WalletsChanged);

        Ok(())
    }
}

impl WalletsTable {
    fn save_new_wallet_metadata_with_backup_behavior(
        &self,
        mut wallet: WalletMetadata,
        should_backup_to_cloud: bool,
    ) -> Result<(), Error> {
        let network = wallet.network;
        let mode = wallet.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        if wallets.iter().any(|w| w.id == wallet.id) {
            return Err(WalletTableError::WalletAlreadyExists.into());
        }

        let wallet_for_backup = should_backup_to_cloud.then(|| wallet.clone());
        wallet.position = next_append_position(&wallets);
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

    /// Get all wallets for a network (sorted by [`WalletMetadata::position`], then id).
    pub fn get_all(
        &self,
        network: Network,
        mode: WalletMode,
    ) -> Result<Vec<WalletMetadata>, Error> {
        let mut wallets = self.load_wallets_raw(network, mode)?;
        if migrate_legacy_positions_if_needed(&mut wallets) {
            self.save_all_wallets(network, mode, wallets.clone())?;
        }
        sort_wallets_by_position(&mut wallets);
        Ok(wallets)
    }

    fn load_wallets_raw(
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
            .unwrap_or_default();

        Ok(value)
    }

    pub fn update_wallet_metadata(&self, metadata: WalletMetadata) -> Result<(), Error> {
        let network = metadata.network;
        let mode = metadata.wallet_mode;

        let mut wallets = self.get_all(network, mode)?;

        // update the wallet
        for wallet in &mut wallets {
            if wallet.id == metadata.id {
                *wallet = metadata.clone();
            }
        }

        self.save_all_wallets(network, mode, wallets)?;

        Ok(())
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

fn next_append_position(wallets: &[WalletMetadata]) -> u32 {
    wallets
        .iter()
        .map(|w| w.position)
        .max()
        .map(|m| m.saturating_add(1))
        .unwrap_or(0)
}


/// Zero-inference migration for wallets saved before `WalletMetadata.position` existed.
///
/// Older rows deserialize with `position == 0`. To avoid random or hash-based reordering,
/// we preserve the currently stored vector order exactly by assigning sequential positions
/// from each wallet's existing index (`0..n-1`).
fn migrate_legacy_positions_if_needed(wallets: &mut [WalletMetadata]) -> bool {
    if wallets.len() > 1 && wallets.iter().all(|w| w.position == 0) {
        for (i, w) in wallets.iter_mut().enumerate() {
            w.position = i as u32;
        }
        true
    } else {
        false
    }
}

fn sort_wallets_by_position(wallets: &mut [WalletMetadata]) {
    wallets.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn validate_reorder_order(
    ordered_ids: &[WalletId],
    wallets: &[WalletMetadata],
) -> Result<(), WalletTableError> {
    let expected: HashSet<WalletId> = wallets.iter().map(|w| w.id.clone()).collect();
    if ordered_ids.len() != expected.len() {
        return Err(WalletTableError::InvalidWalletReorder(
            "order must list every wallet exactly once".into(),
        ));
    }
    let mut seen = HashSet::new();
    for id in ordered_ids {
        if !expected.contains(id) {
            return Err(WalletTableError::InvalidWalletReorder(format!(
                "unknown wallet id: {id}"
            )));
        }
        if !seen.insert(id.clone()) {
            return Err(WalletTableError::InvalidWalletReorder(format!(
                "duplicate wallet id: {id}"
            )));
        }
    }
    Ok(())
}

fn build_reordered_wallets(
    ordered_ids: &[WalletId],
    wallets: Vec<WalletMetadata>,
) -> Result<Vec<WalletMetadata>, WalletTableError> {
    validate_reorder_order(ordered_ids, &wallets)?;

    let mut by_id: HashMap<WalletId, WalletMetadata> =
        wallets.into_iter().map(|w| (w.id.clone(), w)).collect();

    let mut reordered = Vec::with_capacity(ordered_ids.len());
    for (i, id) in ordered_ids.iter().enumerate() {
        let mut wallet = by_id.remove(id).ok_or_else(|| {
            WalletTableError::InvalidWalletReorder("missing wallet after validation".into())
        })?;
        wallet.position = i as u32;
        reordered.push(wallet);
    }

    Ok(reordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::Network;
    use crate::wallet::metadata::{WalletMode, WalletType};

    fn wallet_with_id(id: &str, position: u32) -> WalletMetadata {
        let mut m = WalletMetadata::preview_new();
        m.id = id.into();
        m.position = position;
        m.network = Network::Bitcoin;
        m.wallet_mode = WalletMode::Main;
        m.wallet_type = WalletType::Hot;
        m
    }

    #[test]
    fn migrate_legacy_assigns_positions_from_vec_order() {
        let mut wallets = vec![wallet_with_id("a", 0), wallet_with_id("b", 0), wallet_with_id("c", 0)];
        assert!(migrate_legacy_positions_if_needed(&mut wallets));
        assert_eq!(wallets[0].position, 0);
        assert_eq!(wallets[1].position, 1);
        assert_eq!(wallets[2].position, 2);
    }

    #[test]
    fn migrate_legacy_skips_single_wallet() {
        let mut wallets = vec![wallet_with_id("only", 0)];
        assert!(!migrate_legacy_positions_if_needed(&mut wallets));
    }

    #[test]
    fn sort_wallets_by_position_then_id() {
        let mut wallets = vec![
            wallet_with_id("z", 1),
            wallet_with_id("a", 1),
            wallet_with_id("m", 0),
        ];
        sort_wallets_by_position(&mut wallets);
        assert_eq!(AsRef::<str>::as_ref(&wallets[0].id), "m");
        assert_eq!(AsRef::<str>::as_ref(&wallets[1].id), "a");
        assert_eq!(AsRef::<str>::as_ref(&wallets[2].id), "z");
    }

    #[test]
    fn validate_reorder_accepts_permutation() {
        let w = vec![
            wallet_with_id("a", 0),
            wallet_with_id("b", 1),
            wallet_with_id("c", 2),
        ];
        let order = vec!["c".into(), "a".into(), "b".into()];
        assert!(validate_reorder_order(&order, &w).is_ok());
    }

    #[test]
    fn validate_reorder_rejects_duplicate() {
        let w = vec![wallet_with_id("a", 0), wallet_with_id("b", 1)];
        let order = vec!["a".into(), "a".into()];
        assert!(matches!(
            validate_reorder_order(&order, &w),
            Err(WalletTableError::InvalidWalletReorder(_))
        ));
    }

    #[test]
    fn validate_reorder_rejects_unknown_id() {
        let w = vec![wallet_with_id("a", 0)];
        let order = vec!["nope".into()];
        assert!(validate_reorder_order(&order, &w).is_err());
    }

    #[test]
    fn validate_reorder_rejects_partial_list() {
        let w = vec![wallet_with_id("a", 0), wallet_with_id("b", 1)];
        let order = vec!["a".into()];
        assert!(validate_reorder_order(&order, &w).is_err());
    }

    #[test]
    fn build_reordered_wallets_invalid_input_does_not_mutate_original_vector() {
        let original = vec![wallet_with_id("a", 0), wallet_with_id("b", 1)];
        let working = original.clone();
        let order = vec!["a".into(), "unknown".into()];

        assert!(build_reordered_wallets(&order, working.clone()).is_err());
        assert_eq!(working, original);
    }

    #[test]
    fn next_append_is_sequential_for_existing_sequence() {
        let wallets = vec![
            wallet_with_id("a", 0),
            wallet_with_id("b", 1),
            wallet_with_id("c", 2),
        ];
        assert_eq!(next_append_position(&wallets), 3);
    }

    #[test]
    fn next_append_after_gap() {
        let wallets = vec![wallet_with_id("a", 0), wallet_with_id("b", 10)];
        assert_eq!(next_append_position(&wallets), 11);
    }
}

// redb::Key for WalletId is now implemented in the cove-types crate
