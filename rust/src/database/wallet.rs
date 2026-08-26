use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    sync::Arc,
    time::Duration,
};

use redb::{ReadOnlyTable, ReadableTable as _, ReadableTableMetadata, TableDefinition};
use tracing::{debug, warn};

use cove_util::result_ext::ResultExt as _;

use crate::transaction::Unit;
use crate::{
    app::reconcile::{AppStateReconcileMessage, Update, Updater},
    network::Network,
    wallet::{
        WalletAddressType,
        deletion::WalletInventoryFailure,
        fingerprint::Fingerprint,
        metadata::{
            DiscoveryState, FiatOrBtc, HardwareWalletMetadata, WalletColor, WalletMetadata,
            WalletMode, WalletType,
        },
    },
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

    #[error("wallet not found")]
    WalletNotFound,
}

#[derive(Debug, Clone, Copy, uniffi::Object)]
pub struct WalletKey(Network, Version, WalletMode);

#[derive(Debug, Clone, Copy)]
enum InternalMetadataUpdate {
    PreserveExisting,
    Replace,
}

/// A patch that changes only the named wallet metadata fields
///
/// Patches are applied to the row read inside the same redb write transaction. This keeps
/// unrelated fields and the user's wallet order intact when concurrent metadata updates arrive
#[derive(Debug, Clone)]
pub(crate) enum WalletMetadataPatch {
    UserFacing(WalletUserMetadataPatch),
    Verified(bool),
    WalletType(WalletType),
    DiscoveryState(DiscoveryState),
    AddressType(WalletAddressTypePatch),
    Internal(WalletInternalMetadataPatch),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WalletUserMetadataPatch {
    pub name: Option<String>,
    pub color: Option<WalletColor>,
    pub selected_unit: Option<Unit>,
    pub fiat_or_btc: Option<FiatOrBtc>,
    pub sensitive_visible: Option<bool>,
    pub details_expanded: Option<bool>,
    pub show_labels: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WalletInternalMetadataPatch {
    pub address_index: Option<Option<cove_types::AddressIndex>>,
    pub last_scan_finished: Option<Option<Duration>>,
    pub last_height_fetched: Option<Option<cove_types::BlockSizeLast>>,
    pub performed_full_scan_at: Option<Option<u64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct WalletAddressTypePatch {
    pub address_type: WalletAddressType,
    pub discovery_state: DiscoveryState,
    pub origin: Option<String>,
    pub master_fingerprint: Option<Arc<Fingerprint>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct WalletMetadataPatchResult {
    pub before: WalletMetadata,
    pub after: WalletMetadata,
}

impl WalletMetadataPatch {
    pub(crate) fn apply_to(&self, metadata: &mut WalletMetadata) {
        match self {
            Self::UserFacing(patch) => {
                if let Some(name) = &patch.name {
                    metadata.name.clone_from(name);
                }

                if let Some(color) = patch.color {
                    metadata.color = color;
                }

                if let Some(selected_unit) = patch.selected_unit {
                    metadata.selected_unit = selected_unit;
                }

                if let Some(fiat_or_btc) = patch.fiat_or_btc {
                    metadata.fiat_or_btc = fiat_or_btc;
                }

                if let Some(sensitive_visible) = patch.sensitive_visible {
                    metadata.sensitive_visible = sensitive_visible;
                }

                if let Some(details_expanded) = patch.details_expanded {
                    metadata.details_expanded = details_expanded;
                }

                if let Some(show_labels) = patch.show_labels {
                    metadata.show_labels = show_labels;
                }
            }
            Self::Verified(verified) => metadata.verified = *verified,
            Self::WalletType(wallet_type) => metadata.wallet_type = *wallet_type,
            Self::DiscoveryState(discovery_state) => {
                metadata.discovery_state = discovery_state.clone();
            }
            Self::AddressType(patch) => {
                metadata.address_type = patch.address_type;
                metadata.discovery_state = patch.discovery_state.clone();
                metadata.origin = patch.origin.clone();
                metadata.master_fingerprint = patch.master_fingerprint.clone();
                metadata.internal.reset_scan_state_for_address_type_switch();
            }
            Self::Internal(patch) => {
                if let Some(address_index) = &patch.address_index {
                    metadata.internal.address_index = address_index.clone();
                }

                if let Some(last_scan_finished) = patch.last_scan_finished {
                    metadata.internal.last_scan_finished = last_scan_finished;
                }

                if let Some(last_height_fetched) = patch.last_height_fetched {
                    metadata.internal.last_height_fetched = last_height_fetched;
                }

                if let Some(performed_full_scan_at) = patch.performed_full_scan_at {
                    metadata.internal.performed_full_scan_at = performed_full_scan_at;
                }
            }
        }
    }

    fn affects_wallet_list(&self) -> bool {
        matches!(
            self,
            Self::UserFacing(_) | Self::Verified(_) | Self::WalletType(_) | Self::AddressType(_)
        )
    }
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

    /// Returns wallets in persisted user-facing display order
    pub fn all(&self) -> Result<Vec<WalletMetadata>, Error> {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        debug!("getting all wallets for {network}");
        let wallets = self.get_all(network, wallet_mode)?;

        Ok(wallets)
    }

    /// Returns wallets sorted by recent scan activity for launch selection
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

    /// Persists user-facing display order for the current network and mode
    ///
    /// Cloud restore can only preserve the restored Vec order; reorder is local database state
    pub fn reorder_wallets(&self, wallet_ids: Vec<WalletId>) -> Result<Vec<WalletMetadata>, Error> {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        self.reorder(network, wallet_mode, wallet_ids)
    }
}

impl WalletsTable {
    /// Read every supported network and wallet-mode bucket or fail the full inventory
    pub(crate) fn complete_inventory(&self) -> Result<Vec<WalletMetadata>, WalletInventoryFailure> {
        use strum::IntoEnumIterator as _;

        let mut inventory = Vec::new();
        for network in Network::iter() {
            for wallet_mode in WalletMode::iter() {
                let mut wallets = self.get_all(network, wallet_mode).map_err(|source| {
                    WalletInventoryFailure {
                        network: Some(network),
                        wallet_mode: Some(wallet_mode),
                        source_detail: source.to_string(),
                    }
                })?;

                inventory.append(&mut wallets);
            }
        }

        Ok(inventory)
    }

    fn save_new_wallet_metadata_with_backup_behavior(
        &self,
        wallet: WalletMetadata,
        should_backup_to_cloud: bool,
    ) -> Result<(), Error> {
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(wallet.id.clone())?;

        let network = wallet.network;
        let mode = wallet.wallet_mode;
        let wallet_for_backup = should_backup_to_cloud.then(|| wallet.clone());

        self.update_wallets(network, mode, |wallets| {
            if wallets.iter().any(|stored| stored.id == wallet.id) {
                return Err(WalletTableError::WalletAlreadyExists.into());
            }

            wallets.push(wallet);
            Ok(())
        })?;

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

    /// Applies a named metadata patch atomically to the latest persisted wallet row
    pub(crate) fn patch_wallet_metadata(
        &self,
        id: &WalletId,
        network: Network,
        mode: WalletMode,
        patch: WalletMetadataPatch,
    ) -> Result<WalletMetadataPatchResult, Error> {
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())?;

        let write_txn = self.db.begin_write()?;

        let result = {
            let mut table = write_txn.open_table(TABLE)?;
            let key = WalletKey::from((network, mode)).to_string();
            let mut wallets = table
                .get(key.as_str())
                .map_err_str(WalletTableError::ReadError)?
                .map(|value| value.value())
                .unwrap_or_default();

            let Some(wallet) = wallets.iter_mut().find(|wallet| &wallet.id == id) else {
                return Err(WalletTableError::WalletNotFound.into());
            };

            let before = wallet.clone();
            patch.apply_to(wallet);
            let after = wallet.clone();

            table.insert(&*key, wallets).map_err_str(WalletTableError::SaveError)?;

            WalletMetadataPatchResult { before, after }
        };

        write_txn.commit().map_err_str(WalletTableError::SaveError)?;
        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        if patch.affects_wallet_list() {
            Updater::send_update(Update::WalletsChanged);
        }

        Ok(result)
    }

    /// Updates a wallet row before a live actor owns its metadata
    ///
    /// Live wallet managers must use `patch_wallet_metadata` so a stale full row cannot overwrite
    /// fields changed by another actor message
    pub(crate) fn update_wallet_metadata(
        &self,
        metadata: WalletMetadata,
    ) -> Result<WalletMetadata, Error> {
        self.update_wallet_metadata_inner(metadata, InternalMetadataUpdate::PreserveExisting)
    }

    /// Replaces a wallet row for explicit restore and migration operations
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
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(metadata.id.clone())?;

        let network = metadata.network;
        let mode = metadata.wallet_mode;
        let write_txn = self.db.begin_write()?;

        let result = {
            let mut table = write_txn.open_table(TABLE)?;
            let key = WalletKey::from((network, mode)).to_string();
            let mut wallets = table
                .get(key.as_str())
                .map_err_str(WalletTableError::ReadError)?
                .map(|value| value.value())
                .unwrap_or_default();

            let Some(wallet) = wallets.iter_mut().find(|wallet| wallet.id == metadata.id) else {
                return Err(WalletTableError::WalletNotFound.into());
            };

            if matches!(internal_update, InternalMetadataUpdate::PreserveExisting) {
                metadata.internal = wallet.internal.clone();
            }

            *wallet = metadata.clone();
            table.insert(&*key, wallets).map_err_str(WalletTableError::SaveError)?;

            metadata
        };

        write_txn.commit().map_err_str(WalletTableError::SaveError)?;
        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        Updater::send_update(Update::WalletsChanged);

        Ok(result)
    }

    pub fn delete(&self, id: &WalletId) -> Result<(), Error> {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        self.delete_inner(network, mode, id)
    }

    fn delete_inner(&self, network: Network, mode: WalletMode, id: &WalletId) -> Result<(), Error> {
        self.remove_wallet_metadata(network, mode, id)?;

        Updater::send_update(Update::WalletsChanged);

        Ok(())
    }

    pub(crate) fn remove_wallet_metadata(
        &self,
        network: Network,
        mode: WalletMode,
        id: &WalletId,
    ) -> Result<bool, Error> {
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())?;
        self.remove_prepared_wallet_metadata(network, mode, id)
    }

    pub(crate) fn remove_prepared_wallet_metadata(
        &self,
        network: Network,
        mode: WalletMode,
        id: &WalletId,
    ) -> Result<bool, Error> {
        self.update_wallets(network, mode, |wallets| {
            let before = wallets.len();
            wallets.retain(|wallet| &wallet.id != id);

            Ok(wallets.len() < before)
        })
    }

    fn reorder(
        &self,
        network: Network,
        mode: WalletMode,
        wallet_ids: Vec<WalletId>,
    ) -> Result<Vec<WalletMetadata>, Error> {
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_unscoped_persistence_operation()?;

        let wallets = self.get_all(network, mode)?;

        if wallets.len() != wallet_ids.len() {
            warn!(
                current_len = wallets.len(),
                requested_len = wallet_ids.len(),
                ?network,
                ?mode,
                "Ignoring wallet reorder with mismatched id count"
            );

            return Ok(wallets);
        }

        let mut requested_id_set = HashSet::with_capacity(wallet_ids.len());
        for wallet_id in &wallet_ids {
            if !requested_id_set.insert(wallet_id.clone()) {
                warn!(?wallet_id, ?network, ?mode, "Ignoring wallet reorder with duplicate id");

                return Ok(wallets);
            }
        }

        let current_id_set = wallets.iter().map(|wallet| wallet.id.clone()).collect::<HashSet<_>>();
        if requested_id_set != current_id_set {
            warn!(?network, ?mode, "Ignoring wallet reorder with unknown or missing id");

            return Ok(wallets);
        }

        let is_identical_order = wallets
            .iter()
            .zip(&wallet_ids)
            .all(|(wallet, requested_id)| wallet.id == *requested_id);
        if is_identical_order {
            return Ok(wallets);
        }

        let mut wallets_by_id = wallets
            .into_iter()
            .map(|wallet| (wallet.id.clone(), wallet))
            .collect::<HashMap<_, _>>();
        let reordered = wallet_ids
            .into_iter()
            .map(|wallet_id| wallets_by_id.remove(&wallet_id).expect("validated wallet id set"))
            .collect::<Vec<_>>();

        self.save_all_wallets(network, mode, reordered.clone())?;

        Updater::send_update(Update::WalletsChanged);

        Ok(reordered)
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
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_unscoped_persistence_operation()?;

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

    fn update_wallets<T>(
        &self,
        network: Network,
        mode: WalletMode,
        update: impl FnOnce(&mut Vec<WalletMetadata>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let write_txn = self.db.begin_write()?;

        let result = {
            let mut table = write_txn.open_table(TABLE)?;
            let key = WalletKey::from((network, mode)).to_string();
            let mut wallets = table
                .get(key.as_str())
                .map_err_str(WalletTableError::ReadError)?
                .map(|value| value.value())
                .unwrap_or_default();
            let result = update(&mut wallets)?;

            table.insert(&*key, wallets).map_err_str(WalletTableError::SaveError)?;
            result
        };

        write_txn.commit().map_err_str(WalletTableError::SaveError)?;
        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        Ok(result)
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

    #[test]
    fn wallet_key_strings_cover_every_network_and_mode() {
        use strum::IntoEnumIterator as _;

        let keys = Network::iter()
            .flat_map(|network| {
                WalletMode::iter().map(move |mode| WalletKey::from((network, mode)).to_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "Bitcoin::1",
                "DECOY::Bitcoin::1",
                "Testnet::1",
                "DECOY::Testnet::1",
                "Testnet4::1",
                "DECOY::Testnet4::1",
                "Signet::1",
                "DECOY::Signet::1",
            ]
        );
    }

    fn wallet(name: &str) -> WalletMetadata {
        let mut wallet = WalletMetadata::preview_new();
        wallet.id = WalletId::from(name.to_string());
        wallet.name = name.to_string();
        wallet
    }

    fn wallet_ids(wallets: &[WalletMetadata]) -> Vec<WalletId> {
        wallets.iter().map(|wallet| wallet.id.clone()).collect()
    }

    fn names(wallets: &[WalletMetadata]) -> Vec<String> {
        wallets.iter().map(|wallet| wallet.name.clone()).collect()
    }

    fn wallet_table() -> (tempfile::TempDir, WalletsTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = WalletsTable::new(db, &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }

    #[test]
    fn reorder_persists_and_round_trips() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        let third = wallet("third");
        let original = vec![first.clone(), second.clone(), third.clone()];

        table.save_all_wallets(first.network, first.wallet_mode, original).unwrap();

        let requested_ids = vec![third.id.clone(), first.id.clone(), second.id.clone()];
        let reordered = table.reorder(first.network, first.wallet_mode, requested_ids).unwrap();
        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();

        assert_eq!(names(&reordered), ["third", "first", "second"]);
        assert_eq!(wallet_ids(&persisted), wallet_ids(&reordered));
    }

    #[test]
    fn reorder_rejects_mismatched_ids() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        let third = wallet("third");
        let original = vec![first.clone(), second.clone(), third.clone()];

        table.save_all_wallets(first.network, first.wallet_mode, original.clone()).unwrap();

        let missing_id = vec![third.id.clone(), first.id.clone()];
        let returned = table.reorder(first.network, first.wallet_mode, missing_id).unwrap();
        assert_eq!(wallet_ids(&returned), wallet_ids(&original));

        let duplicate_id = vec![third.id.clone(), first.id.clone(), first.id.clone()];
        let returned = table.reorder(first.network, first.wallet_mode, duplicate_id).unwrap();
        assert_eq!(wallet_ids(&returned), wallet_ids(&original));

        let unknown = wallet("unknown");
        let unknown_id = vec![third.id.clone(), first.id.clone(), unknown.id.clone()];
        let returned = table.reorder(first.network, first.wallet_mode, unknown_id).unwrap();
        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();

        assert_eq!(wallet_ids(&returned), wallet_ids(&original));
        assert_eq!(wallet_ids(&persisted), wallet_ids(&original));
    }

    #[test]
    fn reorder_is_noop_for_identical_order() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        let third = wallet("third");
        let original = vec![first.clone(), second.clone(), third.clone()];

        table.save_all_wallets(first.network, first.wallet_mode, original.clone()).unwrap();

        let returned =
            table.reorder(first.network, first.wallet_mode, wallet_ids(&original)).unwrap();
        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();

        assert_eq!(wallet_ids(&returned), wallet_ids(&original));
        assert_eq!(wallet_ids(&persisted), wallet_ids(&original));
    }

    #[test]
    fn new_wallet_appends_after_reorder() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        let third = wallet("third");
        let fourth = wallet("fourth");
        let original = vec![first.clone(), second.clone(), third.clone()];

        table.save_all_wallets(first.network, first.wallet_mode, original).unwrap();

        let requested_ids = vec![third.id.clone(), first.id.clone(), second.id.clone()];
        table.reorder(first.network, first.wallet_mode, requested_ids).unwrap();
        table.save_new_wallet_metadata_with_backup_behavior(fourth.clone(), false).unwrap();
        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();

        assert_eq!(names(&persisted), ["third", "first", "second", "fourth"]);
    }

    #[test]
    fn concurrent_restore_add_and_rollback_preserve_new_wallet() {
        let (_tmp, table) = wallet_table();
        let restored = wallet("restored");
        let concurrent = wallet("concurrent");

        for _ in 0..20 {
            table
                .save_all_wallets(restored.network, restored.wallet_mode, vec![restored.clone()])
                .unwrap();

            let barrier = Arc::new(std::sync::Barrier::new(3));
            std::thread::scope(|scope| {
                let remove_table = table.clone();
                let remove_barrier = barrier.clone();
                let remove_id = restored.id.clone();
                let network = restored.network;
                let mode = restored.wallet_mode;
                scope.spawn(move || {
                    remove_barrier.wait();
                    remove_table.remove_wallet_metadata(network, mode, &remove_id).unwrap();
                });

                let add_table = table.clone();
                let add_barrier = barrier.clone();
                let concurrent = concurrent.clone();
                scope.spawn(move || {
                    add_barrier.wait();
                    add_table
                        .save_new_wallet_metadata_with_backup_behavior(concurrent, false)
                        .unwrap();
                });

                barrier.wait();
            });

            let persisted = table.get_all(restored.network, restored.wallet_mode).unwrap();
            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0].id, concurrent.id);
        }
    }

    #[test]
    fn delete_and_metadata_update_preserve_order() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        let third = wallet("third");
        let original = vec![first.clone(), second.clone(), third.clone()];

        table.save_all_wallets(first.network, first.wallet_mode, original).unwrap();

        let requested_ids = vec![third.id.clone(), first.id.clone(), second.id.clone()];
        table.reorder(first.network, first.wallet_mode, requested_ids).unwrap();

        let mut renamed_first = first.clone();
        renamed_first.name = "renamed first".to_string();
        table.update_wallet_metadata(renamed_first).unwrap();
        table.delete_inner(third.network, third.wallet_mode, &third.id).unwrap();
        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();

        assert_eq!(names(&persisted), ["renamed first", "second"]);
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

    #[test]
    fn concurrent_disjoint_patches_preserve_latest_fields_and_order() {
        let (_tmp, table) = wallet_table();
        let first = wallet("first");
        let second = wallet("second");
        table
            .save_all_wallets(first.network, first.wallet_mode, vec![first.clone(), second.clone()])
            .unwrap();
        let network = first.network;
        let mode = first.wallet_mode;
        let first_id = first.id.clone();

        let barrier = Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|scope| {
            let rename_table = table.clone();
            let rename_barrier = barrier.clone();
            let rename_id = first_id.clone();
            scope.spawn(move || {
                rename_barrier.wait();
                rename_table
                    .patch_wallet_metadata(
                        &rename_id,
                        network,
                        mode,
                        WalletMetadataPatch::UserFacing(WalletUserMetadataPatch {
                            name: Some("renamed".to_string()),
                            ..Default::default()
                        }),
                    )
                    .unwrap();
            });

            let internal_table = table.clone();
            let internal_barrier = barrier.clone();
            let internal_id = first_id;
            scope.spawn(move || {
                internal_barrier.wait();
                internal_table
                    .patch_wallet_metadata(
                        &internal_id,
                        network,
                        mode,
                        WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
                            performed_full_scan_at: Some(Some(42)),
                            ..Default::default()
                        }),
                    )
                    .unwrap();
            });

            barrier.wait();
        });

        let persisted = table.get_all(first.network, first.wallet_mode).unwrap();
        assert_eq!(wallet_ids(&persisted), vec![first.id, second.id]);
        assert_eq!(persisted[0].name, "renamed");
        assert_eq!(persisted[0].internal.performed_full_scan_at, Some(42));
    }

    #[test]
    fn concurrent_verified_rename_and_scan_patch_preserve_all_changes() {
        let (_tmp, table) = wallet_table();
        let stored = wallet("original");
        table.save_all_wallets(stored.network, stored.wallet_mode, vec![stored.clone()]).unwrap();
        let network = stored.network;
        let mode = stored.wallet_mode;
        let stored_id = stored.id.clone();

        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            let verified_table = table.clone();
            let verified_barrier = barrier.clone();
            let id = stored_id.clone();
            scope.spawn(move || {
                verified_barrier.wait();
                verified_table
                    .patch_wallet_metadata(&id, network, mode, WalletMetadataPatch::Verified(true))
                    .unwrap();
            });

            let rename_table = table.clone();
            let rename_barrier = barrier.clone();
            let id = stored_id.clone();
            scope.spawn(move || {
                rename_barrier.wait();
                rename_table
                    .patch_wallet_metadata(
                        &id,
                        network,
                        mode,
                        WalletMetadataPatch::UserFacing(WalletUserMetadataPatch {
                            name: Some("renamed".to_string()),
                            ..Default::default()
                        }),
                    )
                    .unwrap();
            });

            let scan_table = table.clone();
            let scan_barrier = barrier.clone();
            let id = stored_id;
            scope.spawn(move || {
                scan_barrier.wait();
                scan_table
                    .patch_wallet_metadata(
                        &id,
                        network,
                        mode,
                        WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
                            last_scan_finished: Some(Some(Duration::from_secs(7))),
                            ..Default::default()
                        }),
                    )
                    .unwrap();
            });

            barrier.wait();
        });

        let persisted = table.get(&stored.id, stored.network, stored.wallet_mode).unwrap().unwrap();
        assert!(persisted.verified);
        assert_eq!(persisted.name, "renamed");
        assert_eq!(persisted.internal.last_scan_finished, Some(Duration::from_secs(7)));
    }

    #[test]
    fn address_type_patch_preserves_user_fields_and_resets_only_local_scan_state() {
        let (_tmp, table) = wallet_table();
        let mut stored = wallet("switch");
        stored.name = "renamed before switch".to_string();
        stored.origin = Some("wpkh([817e7be0/84'/0'/0'])".to_string());
        stored.internal.address_index =
            Some(cove_types::AddressIndex { last_seen_index: 4, address_list_hash: 7 });
        stored.internal.last_scan_finished = Some(Duration::from_secs(10));
        stored.internal.last_height_fetched = Some(cove_types::BlockSizeLast {
            block_height: 100,
            last_seen: Duration::from_secs(20),
        });
        stored.internal.performed_full_scan_at = Some(30);
        stored.internal.store_type = crate::wallet::metadata::StoreType::FileStore;
        table.save_all_wallets(stored.network, stored.wallet_mode, vec![stored.clone()]).unwrap();

        let master_fingerprint = Arc::new(Fingerprint::from(
            "deadbeef".parse::<bdk_wallet::bitcoin::bip32::Fingerprint>().unwrap(),
        ));
        let result = table
            .patch_wallet_metadata(
                &stored.id,
                stored.network,
                stored.wallet_mode,
                WalletMetadataPatch::AddressType(WalletAddressTypePatch {
                    address_type: WalletAddressType::Legacy,
                    discovery_state: DiscoveryState::ChoseAdressType,
                    origin: Some("pkh([deadbeef/44'/0'/0'])".to_string()),
                    master_fingerprint: Some(master_fingerprint.clone()),
                }),
            )
            .unwrap();

        assert_eq!(result.before, stored);
        assert_eq!(result.after.name, stored.name);
        assert_eq!(result.after.address_type, WalletAddressType::Legacy);
        assert_eq!(result.after.discovery_state, DiscoveryState::ChoseAdressType);
        assert_eq!(result.after.origin, Some("pkh([deadbeef/44'/0'/0'])".to_string()));
        assert_eq!(result.after.master_fingerprint, Some(master_fingerprint));
        assert_eq!(result.after.internal.address_index, None);
        assert_eq!(result.after.internal.last_scan_finished, None);
        assert_eq!(result.after.internal.last_height_fetched, None);
        assert_eq!(result.after.internal.performed_full_scan_at, None);
        assert_eq!(result.after.internal.store_type, stored.internal.store_type);
        assert_eq!(table.get_all(stored.network, stored.wallet_mode).unwrap(), vec![result.after]);
    }

    #[test]
    fn address_index_patch_preserves_other_internal_fields() {
        let (_tmp, table) = wallet_table();
        let mut stored = wallet("address-index");
        stored.internal.address_index =
            Some(cove_types::AddressIndex { last_seen_index: 1, address_list_hash: 2 });
        stored.internal.last_scan_finished = Some(Duration::from_secs(3));
        stored.internal.last_height_fetched =
            Some(cove_types::BlockSizeLast { block_height: 4, last_seen: Duration::from_secs(5) });
        stored.internal.performed_full_scan_at = Some(6);
        table.save_all_wallets(stored.network, stored.wallet_mode, vec![stored.clone()]).unwrap();

        let address_index = cove_types::AddressIndex { last_seen_index: 7, address_list_hash: 8 };
        table
            .patch_wallet_metadata(
                &stored.id,
                stored.network,
                stored.wallet_mode,
                WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
                    address_index: Some(Some(address_index.clone())),
                    ..Default::default()
                }),
            )
            .unwrap();

        let persisted = table.get(&stored.id, stored.network, stored.wallet_mode).unwrap().unwrap();
        assert_eq!(persisted.internal.address_index, Some(address_index));
        assert_eq!(persisted.internal.last_scan_finished, stored.internal.last_scan_finished);
        assert_eq!(persisted.internal.last_height_fetched, stored.internal.last_height_fetched);
        assert_eq!(
            persisted.internal.performed_full_scan_at,
            stored.internal.performed_full_scan_at
        );
    }

    #[test]
    fn patch_returns_error_when_wallet_id_is_missing() {
        let (_tmp, table) = wallet_table();
        let missing = wallet("missing");

        let result = table.patch_wallet_metadata(
            &missing.id,
            missing.network,
            missing.wallet_mode,
            WalletMetadataPatch::Verified(true),
        );

        assert!(matches!(result, Err(Error::Wallets(WalletTableError::WalletNotFound))));
    }
}
