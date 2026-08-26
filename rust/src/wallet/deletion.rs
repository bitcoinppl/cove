use std::collections::{HashMap, HashSet};

use cove_util::ResultExt as _;

use crate::{
    database::Database,
    keychain::Keychain,
    manager::cloud_backup_manager::{
        CloudBackupError, CloudBackupRecoveryCleanupPermit, CloudBackupResetPermit,
        CloudBackupResetRecovery, begin_recovery_cleanup as begin_cloud_backup_recovery_cleanup,
    },
    network::Network,
    wallet::metadata::{WalletId, WalletMetadata, WalletMode},
    wallet_lifecycle::{
        PreparedWalletLifecycle, RecoveryCleanupPermit, WalletLifecycleCoordinator,
        WalletLifecycleFailure,
    },
};

/// Exact database bucket that contains one wallet metadata row
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletLocation {
    /// Bitcoin network stored in the wallet-table key
    pub network: Network,
    /// Main or decoy wallet table
    pub wallet_mode: WalletMode,
}

/// Complete durable identity for deletion of one registered wallet
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct RegisteredWalletDeletionTarget {
    pub(crate) wallet_id: WalletId,
    pub(crate) locations: Vec<WalletLocation>,
}

/// Resolution of a fresh deletion intent against authoritative inventory
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum WalletDeletionIntent {
    AlreadyAbsent,
    Registered(RegisteredWalletDeletionTarget),
}

/// Capability for one exact registered-wallet deletion
#[derive(Debug)]
pub(crate) struct PreparedWalletDeletion {
    _lifecycle: PreparedWalletLifecycle,
    target: RegisteredWalletDeletionTarget,
}

impl PreparedWalletDeletion {
    pub(crate) fn new(
        lifecycle: PreparedWalletLifecycle,
        target: RegisteredWalletDeletionTarget,
    ) -> Self {
        Self { _lifecycle: lifecycle, target }
    }

    pub(crate) fn delete(self) -> Result<(), WalletDeletionFailure> {
        delete_registered_wallet(&self.target)
    }
}

/// Capability for deletion after wallet and Cloud Backup writers have stopped
#[derive(Debug)]
pub(crate) struct PreparedFullWipe {
    _lifecycle: PreparedWalletLifecycle,
    cloud_reset: CloudBackupResetPermit,
    recovery_cleanup: RecoveryCleanup,
}

impl PreparedFullWipe {
    pub(crate) fn new(
        lifecycle: PreparedWalletLifecycle,
        cloud_reset: CloudBackupResetPermit,
    ) -> Self {
        Self {
            _lifecycle: lifecycle,
            cloud_reset,
            recovery_cleanup: RecoveryCleanup { proof: RecoveryCleanupProof::FullWipe },
        }
    }

    pub(crate) fn delete_wallet(
        &self,
        target: &RegisteredWalletDeletionTarget,
    ) -> Result<(), WalletDeletionFailure> {
        delete_registered_wallet(target)
    }

    pub(crate) fn prevent_cloud_resume(&mut self) {
        self.cloud_reset.prevent_resume();
    }

    pub(crate) fn delete_all_wallet_items(&self) -> Result<(), String> {
        self.recovery_cleanup.delete_all_wallet_items()
    }

    pub(crate) fn purge_orphan_wallet_artifacts(&self) -> std::io::Result<()> {
        self.recovery_cleanup.purge_orphan_wallet_artifacts()
    }

    pub(crate) async fn complete_after_database_reset(self) -> Result<(), String> {
        self.cloud_reset.complete_after_database_reset().await.map_err(|error| error.to_string())
    }

    pub(crate) async fn resume_after_failure(
        self,
    ) -> Result<CloudBackupResetRecovery, CloudBackupError> {
        self.cloud_reset.resume_after_failed_reset().await
    }
}

/// Failure to reserve the database-unavailable cleanup owners
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecoveryCleanupAcquireError {
    #[error(transparent)]
    WalletLifecycle(#[from] WalletLifecycleFailure),

    #[error("{0}")]
    CloudBackup(String),
}

#[derive(Debug)]
enum RecoveryCleanupProof {
    FullWipe,
    DatabaseUnavailable {
        _wallet: RecoveryCleanupPermit,
        _cloud_backup: CloudBackupRecoveryCleanupPermit,
    },
}

/// Capability proving wallet and Cloud Backup writers cannot race namespace cleanup
#[derive(Debug)]
pub(crate) struct RecoveryCleanup {
    proof: RecoveryCleanupProof,
}

impl RecoveryCleanup {
    /// Reserve cleanup for startup recovery when the main database cannot be read
    pub(crate) fn prepare_database_unavailable() -> Result<Self, RecoveryCleanupAcquireError> {
        let wallet = WalletLifecycleCoordinator::global().begin_recovery_cleanup()?;
        let cloud_backup = begin_cloud_backup_recovery_cleanup()
            .map_err_str(RecoveryCleanupAcquireError::CloudBackup)?;

        Ok(Self {
            proof: RecoveryCleanupProof::DatabaseUnavailable {
                _wallet: wallet,
                _cloud_backup: cloud_backup,
            },
        })
    }

    pub(crate) fn delete_all_wallet_items(&self) -> Result<(), String> {
        Keychain::global().delete_all_wallet_items().map_err_str(std::convert::identity)
    }

    pub(crate) fn purge_orphan_wallet_artifacts(&self) -> std::io::Result<()> {
        crate::app::purge_orphan_wallet_artifacts(self)
    }

    pub(crate) fn is_authorized(&self) -> bool {
        matches!(
            &self.proof,
            RecoveryCleanupProof::FullWipe | RecoveryCleanupProof::DatabaseUnavailable { .. }
        )
    }
}

/// Phase of a failed registered-wallet deletion
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletDeletionStage {
    /// Wallet keychain entries
    Keychain,
    /// Temporary address-switch store
    AddressSwitchStore,
    /// BDK SQLite and legacy stores
    BdkArtifacts,
    /// Per-wallet application data directory
    WalletData,
    /// Parent-directory durability synchronization
    DirectorySync,
    /// Exact durable metadata rows
    Metadata,
}

/// Typed context for a failed wallet deletion phase
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletDeletionFailure {
    /// Wallet being deleted
    pub wallet_id: WalletId,
    /// Cleanup phase that failed
    pub stage: WalletDeletionStage,
    /// Underlying source error without duplicated phase context
    pub source_detail: String,
}

/// Typed context for a failed authoritative wallet inventory bucket
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletInventoryFailure {
    /// Network bucket, or `None` for a global inventory failure
    pub network: Option<Network>,
    /// Wallet mode bucket, or `None` for a global inventory failure
    pub wallet_mode: Option<WalletMode>,
    /// Underlying source error without duplicated inventory context
    pub source_detail: String,
}

impl std::fmt::Display for WalletInventoryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.source_detail)
    }
}

impl std::error::Error for WalletInventoryFailure {}

impl std::fmt::Display for WalletDeletionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.wallet_id, self.source_detail)
    }
}

impl std::error::Error for WalletDeletionFailure {}

/// Build exact deletion targets from a complete cross-network inventory
pub(crate) fn targets_from_inventory(
    inventory: Vec<WalletMetadata>,
) -> Vec<RegisteredWalletDeletionTarget> {
    let mut locations_by_id = HashMap::<WalletId, HashSet<WalletLocation>>::new();
    for wallet in inventory {
        locations_by_id
            .entry(wallet.id)
            .or_default()
            .insert(WalletLocation { network: wallet.network, wallet_mode: wallet.wallet_mode });
    }

    let mut targets = locations_by_id
        .into_iter()
        .map(|(wallet_id, locations)| {
            let mut locations = locations.into_iter().collect::<Vec<_>>();
            locations.sort_unstable_by_key(|location| {
                (location.network.to_string(), location.wallet_mode.to_string())
            });
            RegisteredWalletDeletionTarget { wallet_id, locations }
        })
        .collect::<Vec<_>>();
    targets.sort_unstable_by(|left, right| left.wallet_id.as_str().cmp(right.wallet_id.as_str()));
    targets
}

/// Resolve one wallet ID from a fresh complete inventory
pub(crate) fn resolve_intent(
    wallet_id: &WalletId,
    inventory: Vec<WalletMetadata>,
) -> WalletDeletionIntent {
    let target =
        targets_from_inventory(inventory).into_iter().find(|target| &target.wallet_id == wallet_id);

    match target {
        Some(target) => WalletDeletionIntent::Registered(target),
        None => WalletDeletionIntent::AlreadyAbsent,
    }
}

/// Delete one prepared wallet's secrets, artifacts, and exact metadata rows
///
/// The lifecycle coordinator is the only caller. Secrets go first and every
/// canonical metadata row goes last, so a partial failure remains retryable
fn delete_registered_wallet(
    target: &RegisteredWalletDeletionTarget,
) -> Result<(), WalletDeletionFailure> {
    let wallet_id = &target.wallet_id;
    if !Keychain::global().delete_wallet_items(wallet_id) {
        return Err(failure(wallet_id, WalletDeletionStage::Keychain, "unable to delete"));
    }

    crate::bdk_store::BdkStore::delete_address_switch_store(wallet_id).map_err(|source| {
        failure(wallet_id, WalletDeletionStage::AddressSwitchStore, source.to_string())
    })?;

    crate::bdk_store::BdkStore::delete_wallet_store_artifacts(wallet_id).map_err(|source| {
        failure(wallet_id, WalletDeletionStage::BdkArtifacts, source.to_string())
    })?;

    crate::database::wallet_data::delete_wallet_data_directory(wallet_id).map_err(|source| {
        failure(wallet_id, WalletDeletionStage::WalletData, source.to_string())
    })?;

    crate::bdk_store::sync_wallet_store_directory().map_err(|source| {
        failure(wallet_id, WalletDeletionStage::DirectorySync, source.to_string())
    })?;

    for location in &target.locations {
        let removed = Database::global()
            .wallets
            .remove_prepared_wallet_metadata(location.network, location.wallet_mode, wallet_id)
            .map_err(|source| {
                failure(wallet_id, WalletDeletionStage::Metadata, source.to_string())
            })?;

        if !removed {
            return Err(failure(
                wallet_id,
                WalletDeletionStage::Metadata,
                format!(
                    "prepared metadata row is missing for {} {}",
                    location.network, location.wallet_mode
                ),
            ));
        }
    }

    crate::app::reconcile::Updater::send_update(
        crate::app::reconcile::AppStateReconcileMessage::WalletsChanged,
    );
    Ok(())
}

fn failure(
    wallet_id: &WalletId,
    stage: WalletDeletionStage,
    source_detail: impl Into<String>,
) -> WalletDeletionFailure {
    WalletDeletionFailure {
        wallet_id: wallet_id.clone(),
        stage,
        source_detail: source_detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_groups_repeated_wallet_ids_by_exact_location() {
        let id = WalletId::preview_new_random();
        let mut main = WalletMetadata::preview_new();
        main.id.clone_from(&id);
        let mut decoy = main.clone();
        decoy.wallet_mode = WalletMode::Decoy;

        let targets = targets_from_inventory(vec![main, decoy]);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].wallet_id, id);
        assert_eq!(targets[0].locations.len(), 2);
    }

    #[test]
    fn deletion_intent_is_absent_when_no_exact_row_exists() {
        let id = WalletId::preview_new_random();

        assert_eq!(resolve_intent(&id, Vec::new()), WalletDeletionIntent::AlreadyAbsent);
    }

    #[test]
    fn deletion_target_preserves_exact_network_and_mode() {
        let mut wallet = WalletMetadata::preview_new();
        wallet.network = Network::Signet;
        wallet.wallet_mode = WalletMode::Decoy;
        let id = wallet.id.clone();

        let WalletDeletionIntent::Registered(target) = resolve_intent(&id, vec![wallet]) else {
            panic!("expected registered deletion target");
        };

        assert_eq!(
            target.locations,
            vec![WalletLocation { network: Network::Signet, wallet_mode: WalletMode::Decoy }]
        );
    }
}
