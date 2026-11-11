//! The inventory owner: the one writer of what Cloud Backup knows about this device's wallets,
//! the records in the active namespace, and the backups in other namespaces
//!
//! Producers (detail refreshes, cloud-only fetches, restore and delete results, local wallet
//! changes, the other-backups scan) hand the owner typed [`InventoryInput`]s. Only
//! [`InventoryOwner::apply`] changes the inventory, and every change that lands bumps the
//! generation the screen and Turn Off read the inventory at

mod listing;

use std::collections::{HashMap, HashSet};

pub(crate) use self::listing::{CloudOnlyFetch, CloudOnlyInventory, CloudOnlyListingUpdate};
use crate::manager::cloud_backup_manager::admission::DetailInventory;
use crate::manager::cloud_backup_manager::model::CloudBackupRestoreAllRuntimeState;
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
    CloudBackupRestoreAllState, CloudBackupWalletItem, CloudBackupWalletRestoreFailure,
    CloudBackupWalletStatus, CloudOnlyOperation, OtherBackupsOperation,
};

/// The loaded inventory of the active namespace
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedCloudBackupDetail {
    pub(crate) detail: CloudBackupDetail,
    /// Evidence used to accept the active namespace inventory
    pub(crate) inventory_authority: CloudBackupInventoryAuthority,
    pub(crate) cloud_only: CloudOnlyState,
    pub(crate) cloud_only_operation: CloudOnlyOperation,
    pub(crate) other_backups_operation: OtherBackupsOperation,
}

/// Evidence used to accept wallet inventory for the active backup namespace
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum CloudBackupInventoryAuthority {
    /// The provider returned an authoritative inventory
    ProviderConfirmed,
    /// The local provider snapshot contains the last known wallet count
    LocalSnapshotMatchesKnownCount,
    /// Rows are useful for display but cannot authorize wallet actions
    Provisional,
}

/// Where loading the active namespace's inventory is
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupDetailState {
    NotLoaded,
    Checking {
        retained: Option<LoadedCloudBackupDetail>,
    },
    Complete {
        state: LoadedCloudBackupDetail,
    },
    Failed {
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
        retained: Option<LoadedCloudBackupDetail>,
    },
}

/// The records of the active namespace that are not a local wallet's own backup
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudOnlyState {
    NotFetched,
    Loading,
    Loaded {
        wallets: Vec<CloudBackupWalletItem>,
        /// Some listed backups could not be read, so `wallets` may be missing cloud-only wallets
        incomplete: bool,
    },
    Failed {
        error: String,
    },
}

/// A typed input to the inventory, from the producer that observed it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InventoryInput {
    DetailRefreshStarted,
    DetailRefreshProvisional(CloudBackupDetail),
    DetailCleared,
    DetailRefreshApplied {
        detail: CloudBackupDetail,
        authority: CloudBackupInventoryAuthority,
        cloud_only_listing: CloudOnlyListingUpdate,
    },
    DetailRefreshFailed {
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
    },
    CloudOnlyStateResolved(CloudOnlyState),
    /// A cloud-only fetch finished, applied only if its listing generation is still current
    CloudOnlyFetchLoaded(CloudOnlyFetch),
    /// A listed record was restored or deleted, so it is no longer cloud-only
    CloudOnlyRowRemoved(String),
    /// A restore found a local wallet holding everything the record can recover, applied only if
    /// no local wallet or listing change happened since the restore read the local wallets
    CloudOnlyRowAlreadyOnDevice {
        record_id: String,
        listing_generation: u64,
    },
    /// A restore of a listed record failed, so its row is marked until a restore starts again
    CloudOnlyRestoreFailed {
        record_id: String,
        error: String,
    },
    /// A restore started for these records, so their earlier failures are gone
    CloudOnlyRestoreFailuresCleared(Vec<String>),
    /// Cloud-only rows no longer belong to any known listing, such as after disable or reset
    CloudOnlyInventoryInvalidated,
    /// The local wallet set changed, so the listing and any loaded rows may now count a record as
    /// cloud-only, restorable, or a duplicate when it no longer is, and the detail may list a
    /// wallet as local that is gone
    ///
    /// `local_record_ids` holds the wallets on this device now, or `None` when they could not be
    /// read, in which case the detail's local rows wait for the next refresh
    LocalWalletsChanged {
        local_record_ids: Option<HashSet<String>>,
    },
    CloudOnlyOperationResolved(CloudOnlyOperation),
    OtherBackupsOperationResolved(OtherBackupsOperation),
    OtherBackupsStateResolved(CloudBackupOtherBackupsState),
}

/// What applying an input asks of the caller
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InventoryEffects {
    /// The inventory changed, so the screen is projected again
    pub(crate) changed: bool,
    /// Cloud-only rows were invalidated or a fetch was stale, so a fetch has to repopulate them
    pub(crate) refetch_needed: bool,
}

/// Identity of one inventory state; it changes with every input that changes the inventory
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InventoryGeneration(u64);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DetailRefreshActivity {
    #[default]
    Idle,
    InFlight,
}

/// The single writer of the Cloud Backup inventory
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InventoryOwner {
    generation: InventoryGeneration,
    detail: CloudBackupDetailState,
    detail_refresh: DetailRefreshActivity,
    cloud_only_inventory: CloudOnlyInventory,
    /// Restore failures shown on rows that a local wallet change reset, kept for the refetch
    /// that replaces them so a failed restore stays marked
    cloud_only_failures_awaiting_refetch: HashMap<String, CloudBackupWalletRestoreFailure>,
    other_backups: CloudBackupOtherBackupsState,
}

impl Default for InventoryOwner {
    fn default() -> Self {
        Self {
            generation: InventoryGeneration::default(),
            detail: CloudBackupDetailState::NotLoaded,
            detail_refresh: DetailRefreshActivity::Idle,
            cloud_only_inventory: CloudOnlyInventory::default(),
            cloud_only_failures_awaiting_refetch: HashMap::new(),
            other_backups: CloudBackupOtherBackupsState::NotChecked,
        }
    }
}

impl InventoryOwner {
    /// Applies one input; the only way the inventory changes
    pub(crate) fn apply(&mut self, input: InventoryInput) -> InventoryEffects {
        let before = self.clone();
        let refetch_needed = self.apply_input(input);

        let changed = *self != before;
        if changed {
            self.generation.0 += 1;
        }

        InventoryEffects { changed, refetch_needed }
    }

    fn apply_input(&mut self, input: InventoryInput) -> bool {
        match input {
            InventoryInput::DetailRefreshStarted => self.apply_detail_refresh_started(),
            InventoryInput::DetailRefreshProvisional(detail) => {
                self.apply_detail_refresh_provisional(detail);
            }
            InventoryInput::DetailRefreshApplied { detail, authority, cloud_only_listing } => {
                return self.apply_detail_refresh(detail, authority, cloud_only_listing);
            }
            InventoryInput::DetailCleared => self.apply_detail_cleared(),
            InventoryInput::DetailRefreshFailed { reason, error } => {
                self.apply_detail_refresh_failure(reason, error);
            }
            InventoryInput::CloudOnlyStateResolved(cloud_only) => {
                self.resolve_cloud_only_state(cloud_only);
            }
            InventoryInput::CloudOnlyFetchLoaded(fetch) => {
                return !self.apply_cloud_only_fetch_loaded(fetch);
            }
            InventoryInput::CloudOnlyRowRemoved(record_id) => {
                self.remove_cloud_only_row(&record_id);
            }
            InventoryInput::CloudOnlyRowAlreadyOnDevice { record_id, listing_generation } => {
                self.mark_cloud_only_row_already_on_device(&record_id, listing_generation);
            }
            InventoryInput::CloudOnlyRestoreFailed { record_id, error } => {
                self.mark_cloud_only_restore_failure(&record_id, error);
            }
            InventoryInput::CloudOnlyRestoreFailuresCleared(record_ids) => {
                self.clear_cloud_only_restore_failures(&record_ids);
            }
            InventoryInput::CloudOnlyInventoryInvalidated => {
                self.cloud_only_inventory.invalidate();
                self.cloud_only_failures_awaiting_refetch.clear();
            }
            InventoryInput::LocalWalletsChanged { local_record_ids } => {
                return self.apply_local_wallets_changed(local_record_ids.as_ref());
            }
            InventoryInput::CloudOnlyOperationResolved(cloud_only_operation) => {
                if let Some(state) = self.loaded_detail_mut() {
                    state.cloud_only_operation = cloud_only_operation;
                }
            }
            InventoryInput::OtherBackupsOperationResolved(other_backups_operation) => {
                self.resolve_other_backups_operation(other_backups_operation);
            }
            InventoryInput::OtherBackupsStateResolved(other_backups) => {
                self.other_backups = other_backups;
            }
        }

        false
    }

    pub(crate) fn detail_state(&self) -> &CloudBackupDetailState {
        &self.detail
    }

    pub(crate) fn other_backups_state(&self) -> CloudBackupOtherBackupsState {
        self.other_backups.clone()
    }

    pub(crate) fn loaded_detail(&self) -> Option<&LoadedCloudBackupDetail> {
        match &self.detail {
            CloudBackupDetailState::Complete { state } => Some(state),
            CloudBackupDetailState::Checking { retained }
            | CloudBackupDetailState::Failed { retained, .. } => retained.as_ref(),
            CloudBackupDetailState::NotLoaded => None,
        }
    }

    fn loaded_detail_mut(&mut self) -> Option<&mut LoadedCloudBackupDetail> {
        match &mut self.detail {
            CloudBackupDetailState::Complete { state } => Some(state),
            CloudBackupDetailState::Checking { retained }
            | CloudBackupDetailState::Failed { retained, .. } => retained.as_mut(),
            CloudBackupDetailState::NotLoaded => None,
        }
    }

    pub(crate) fn cloud_only(&self) -> CloudOnlyState {
        self.loaded_detail()
            .map(|state| state.cloud_only.clone())
            .unwrap_or(CloudOnlyState::NotFetched)
    }

    pub(crate) fn cloud_only_operation(&self) -> CloudOnlyOperation {
        self.loaded_detail()
            .map(|state| state.cloud_only_operation.clone())
            .unwrap_or(CloudOnlyOperation::Idle)
    }

    pub(crate) fn other_backups_operation(&self) -> OtherBackupsOperation {
        self.loaded_detail()
            .map(|state| state.other_backups_operation.clone())
            .unwrap_or(OtherBackupsOperation::Idle)
    }

    pub(crate) fn cloud_only_listing_generation(&self) -> u64 {
        self.cloud_only_inventory.generation()
    }

    /// Whether the loaded cloud-only rows are a complete read of the latest provider listing
    pub(crate) fn cloud_only_is_complete_and_current(&self) -> bool {
        let CloudOnlyState::Loaded { wallets, .. } = self.cloud_only() else { return false };
        let loaded_ids: HashSet<_> = wallets.into_iter().map(|wallet| wallet.record_id).collect();

        self.cloud_only_inventory.loaded_is_complete_and_current(&loaded_ids)
    }

    /// Whether the provider confirmed the inventory, so every action that needs all of it can run
    pub(crate) fn detail_inventory_is_complete(&self) -> bool {
        matches!(
            &self.detail,
            CloudBackupDetailState::Complete { state }
                if state.inventory_authority == CloudBackupInventoryAuthority::ProviderConfirmed
        )
    }

    /// Whether the inventory is trusted enough for a row's own action
    pub(crate) fn detail_inventory_is_ready(&self) -> bool {
        let loaded = match &self.detail {
            CloudBackupDetailState::Complete { state } => Some(state),

            // an offline device can't reach the backups a row would restore or delete, so the
            // known rows stay listed while their actions wait for a check that succeeds
            CloudBackupDetailState::Failed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
                ..
            } => None,
            CloudBackupDetailState::Failed { retained, .. } => retained.as_ref(),
            CloudBackupDetailState::NotLoaded | CloudBackupDetailState::Checking { .. } => None,
        };

        loaded.is_some_and(|detail| {
            matches!(
                detail.inventory_authority,
                CloudBackupInventoryAuthority::ProviderConfirmed
                    | CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount
            )
        })
    }

    /// How far the inventory can be trusted, as admission reads it
    pub(crate) fn readiness(&self) -> DetailInventory {
        if self.detail_inventory_is_complete() {
            DetailInventory::Complete
        } else if self.detail_inventory_is_ready() {
            DetailInventory::Ready
        } else {
            DetailInventory::NotLoaded
        }
    }

    pub(crate) fn other_backups_is_loaded(&self) -> bool {
        matches!(self.other_backups, CloudBackupOtherBackupsState::Loaded { .. })
    }

    /// Wallets Restore All can start with, or `None` while the cloud-only rows are not loaded
    pub(crate) fn restore_all_eligible_wallets(&self) -> Option<Vec<CloudBackupWalletItem>> {
        let CloudOnlyState::Loaded { wallets, .. } = self.cloud_only() else { return None };

        Some(eligible_restore_all_wallets(wallets))
    }

    /// Wallets Restore All still has to restore, or `None` when the rows do not prove it
    ///
    /// Proof means the rows are loaded, every listed record was read, and the rows are current
    /// for the latest listing. Anything less is unknown rather than empty, so the Restore All
    /// marker must stay
    pub(crate) fn restore_all_proven_eligible_wallets(&self) -> Option<Vec<CloudBackupWalletItem>> {
        let CloudOnlyState::Loaded { wallets, .. } = self.cloud_only() else { return None };

        self.cloud_only_is_complete_and_current().then(|| eligible_restore_all_wallets(wallets))
    }

    /// Restore All as the cloud-only rows and the running Restore All allow it
    ///
    /// `operation_active` is whether any exclusive operation holds the slot, which disables a
    /// start or a retry without hiding it
    pub(crate) fn restore_all(
        &self,
        runtime: &CloudBackupRestoreAllRuntimeState,
        operation_active: bool,
    ) -> CloudBackupRestoreAllState {
        let wallet_count = self.restore_all_eligible_wallet_count();
        let available = self.detail_inventory_is_complete() && !operation_active;

        match runtime {
            CloudBackupRestoreAllRuntimeState::Running {
                claim: _,
                completed,
                total,
                current_wallet_name,
                cancellation_requested,
            } => CloudBackupRestoreAllState::Running {
                completed: *completed,
                total: *total,
                current_wallet_name: current_wallet_name.clone(),
                cancellation_requested: *cancellation_requested,
            },
            CloudBackupRestoreAllRuntimeState::RetryRemaining => {
                // backups that could not be read may still need restoring, so the retry stays
                // reachable even when no loaded row is eligible
                let incomplete =
                    matches!(self.cloud_only(), CloudOnlyState::Loaded { incomplete: true, .. });

                if wallet_count == 0 && !incomplete {
                    return CloudBackupRestoreAllState::NotShown;
                }

                if available {
                    CloudBackupRestoreAllState::RetryAvailable { wallet_count }
                } else {
                    CloudBackupRestoreAllState::RetryDisabled { wallet_count }
                }
            }
            CloudBackupRestoreAllRuntimeState::Idle => {
                if wallet_count < 2 {
                    return CloudBackupRestoreAllState::NotShown;
                }

                if available {
                    CloudBackupRestoreAllState::StartAvailable { wallet_count }
                } else {
                    CloudBackupRestoreAllState::StartDisabled { wallet_count }
                }
            }
        }
    }

    fn restore_all_eligible_wallet_count(&self) -> u32 {
        let Some(LoadedCloudBackupDetail {
            cloud_only: CloudOnlyState::Loaded { wallets, .. },
            ..
        }) = self.loaded_detail()
        else {
            return 0;
        };

        wallets
            .iter()
            .filter(|wallet| wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Applies a refreshed detail and returns whether it invalidated loaded cloud-only rows
    fn apply_detail_refresh(
        &mut self,
        detail: CloudBackupDetail,
        inventory_authority: CloudBackupInventoryAuthority,
        cloud_only_listing: CloudOnlyListingUpdate,
    ) -> bool {
        let previous_loaded = self.loaded_detail().cloned();
        self.detail_refresh = DetailRefreshActivity::Idle;

        if let CloudOnlyListingUpdate::Listed(record_ids) = &cloud_only_listing {
            self.cloud_only_inventory.record_listing(record_ids);
        }

        let (cloud_only, invalidated) = match previous_loaded.as_ref().map(|s| &s.cloud_only) {
            Some(CloudOnlyState::Loaded { wallets, .. }) => {
                let keep = match &cloud_only_listing {
                    CloudOnlyListingUpdate::Listed(_) => {
                        let loaded_ids: HashSet<_> =
                            wallets.iter().map(|wallet| wallet.record_id.clone()).collect();

                        self.cloud_only_inventory.keep_loaded_for_listing(&loaded_ids)
                    }
                    CloudOnlyListingUpdate::Unlisted => false,
                };

                if keep {
                    (self.loaded_cloud_only(wallets.clone()), false)
                } else {
                    self.cloud_only_inventory.clear_loaded();
                    (CloudOnlyState::NotFetched, true)
                }
            }
            Some(other) => (other.clone(), false),
            None => (CloudOnlyState::NotFetched, false),
        };

        self.detail = CloudBackupDetailState::Complete {
            state: LoadedCloudBackupDetail {
                detail,
                inventory_authority,
                cloud_only,
                cloud_only_operation: previous_loaded
                    .as_ref()
                    .map(|state| state.cloud_only_operation.clone())
                    .unwrap_or(CloudOnlyOperation::Idle),
                other_backups_operation: previous_loaded
                    .as_ref()
                    .map(|state| state.other_backups_operation.clone())
                    .unwrap_or(OtherBackupsOperation::Idle),
            },
        };

        invalidated
    }

    /// Applies a fetch result when it was started under the current listing, returning whether
    /// it was applied
    fn apply_cloud_only_fetch_loaded(&mut self, mut fetch: CloudOnlyFetch) -> bool {
        if fetch.listing_generation != self.cloud_only_inventory.generation() {
            // the rows belong to an older listing, so drop the Loading marker the fetch set
            if matches!(self.cloud_only(), CloudOnlyState::Loading) {
                self.resolve_cloud_only_state(CloudOnlyState::NotFetched);
            }

            return false;
        }

        let mut retained_failures = std::mem::take(&mut self.cloud_only_failures_awaiting_refetch);
        retained_failures.extend(self.loaded_cloud_only_failures());

        // a row a local wallet now covers has nothing left to restore, so its failure is gone
        for wallet in &mut fetch.wallets {
            if wallet.restore_failure.is_none()
                && wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice
            {
                wallet.restore_failure = retained_failures.get(&wallet.record_id).cloned();
            }
        }

        self.cloud_only_inventory.record_loaded(&fetch);
        let cloud_only = self.loaded_cloud_only(fetch.wallets);
        self.resolve_cloud_only_state(cloud_only);

        true
    }

    fn loaded_cloud_only_failures(&self) -> HashMap<String, CloudBackupWalletRestoreFailure> {
        let CloudOnlyState::Loaded { wallets, .. } = self.cloud_only() else {
            return HashMap::new();
        };

        wallets
            .into_iter()
            .filter_map(|wallet| wallet.restore_failure.map(|failure| (wallet.record_id, failure)))
            .collect()
    }

    /// Loaded rows marked incomplete by the same check the Restore All marker proof uses, so the
    /// retry stays visible whenever the marker cannot be cleared
    fn loaded_cloud_only(&self, wallets: Vec<CloudBackupWalletItem>) -> CloudOnlyState {
        let loaded_ids: HashSet<_> =
            wallets.iter().map(|wallet| wallet.record_id.clone()).collect();

        let incomplete = !self.cloud_only_inventory.loaded_is_complete_and_current(&loaded_ids);

        CloudOnlyState::Loaded { wallets, incomplete }
    }

    /// Keeps a row whose recovery material a local wallet holds visible and deletable, without
    /// offering it for restore again
    fn mark_cloud_only_row_already_on_device(&mut self, record_id: &str, listing_generation: u64) {
        // a local wallet change since the restore read the local wallets bumped the generation
        // and asked for a refetch, which classifies the row against the wallets that exist now
        if listing_generation != self.cloud_only_inventory.generation() {
            return;
        }

        let CloudOnlyState::Loaded { mut wallets, incomplete } = self.cloud_only() else {
            return;
        };

        if let Some(wallet) = wallets.iter_mut().find(|wallet| wallet.record_id == record_id) {
            wallet.sync_status = CloudBackupWalletStatus::AlreadyOnDevice;
            wallet.restore_failure = None;
        }

        self.resolve_cloud_only_state(CloudOnlyState::Loaded { wallets, incomplete });
    }

    fn mark_cloud_only_restore_failure(&mut self, record_id: &str, message: String) {
        let mut cloud_only = self.cloud_only();
        if let CloudOnlyState::Loaded { wallets, .. } = &mut cloud_only
            && let Some(wallet) = wallets.iter_mut().find(|wallet| wallet.record_id == record_id)
        {
            wallet.restore_failure = Some(CloudBackupWalletRestoreFailure { message });
        }

        self.resolve_cloud_only_state(cloud_only);
    }

    fn clear_cloud_only_restore_failures(&mut self, record_ids: &[String]) {
        let mut cloud_only = self.cloud_only();
        if let CloudOnlyState::Loaded { wallets, .. } = &mut cloud_only {
            for wallet in wallets {
                if record_ids.contains(&wallet.record_id) {
                    wallet.restore_failure = None;
                }
            }
        }

        self.resolve_cloud_only_state(cloud_only);
    }

    /// Removes a row whose record stopped being cloud-only, keeping the listing in step with it
    fn remove_cloud_only_row(&mut self, record_id: &str) {
        let CloudOnlyState::Loaded { mut wallets, incomplete } = self.cloud_only() else {
            return;
        };

        wallets.retain(|wallet| wallet.record_id != record_id);
        self.cloud_only_inventory.forget_record(record_id);
        self.resolve_cloud_only_state(CloudOnlyState::Loaded { wallets, incomplete });
    }

    /// Drops the local rows of wallets that are gone, and resets the cloud-only rows so a fetch
    /// classifies them against the wallets that exist now, returning whether one is needed
    ///
    /// A deleted wallet keeps its backup, so it moves to Not on This Device once the cloud-only
    /// rows are read again. Waiting for the throttled detail refresh, or for one that fails, left
    /// it listed as local as well
    fn apply_local_wallets_changed(&mut self, local_record_ids: Option<&HashSet<String>>) -> bool {
        if let Some(local_record_ids) = local_record_ids
            && let Some(state) = self.loaded_detail_mut()
        {
            state.detail.retain_local_wallets(local_record_ids);
        }

        self.cloud_only_inventory.invalidate();

        // a fetch still in flight is now stale, so it is discarded and replaced when it finishes
        if !matches!(self.cloud_only(), CloudOnlyState::Loaded { .. }) {
            return false;
        }

        let failures = self.loaded_cloud_only_failures();
        self.cloud_only_failures_awaiting_refetch.extend(failures);
        self.resolve_cloud_only_state(CloudOnlyState::NotFetched);
        true
    }

    fn apply_detail_cleared(&mut self) {
        self.detail_refresh = DetailRefreshActivity::Idle;
        self.detail = CloudBackupDetailState::NotLoaded;
    }

    fn apply_detail_refresh_started(&mut self) {
        self.detail_refresh = DetailRefreshActivity::InFlight;
        if matches!(self.detail, CloudBackupDetailState::Complete { .. }) {
            return;
        }

        let retained = self.loaded_detail().cloned();
        self.detail = CloudBackupDetailState::Checking { retained };
    }

    fn apply_detail_refresh_provisional(&mut self, detail: CloudBackupDetail) {
        self.detail_refresh = DetailRefreshActivity::InFlight;

        if let Some(previous_loaded) = self.loaded_detail().cloned() {
            self.detail = CloudBackupDetailState::Checking { retained: Some(previous_loaded) };
            return;
        }

        self.detail = CloudBackupDetailState::Checking {
            retained: Some(LoadedCloudBackupDetail {
                detail,
                inventory_authority: CloudBackupInventoryAuthority::Provisional,
                cloud_only: CloudOnlyState::NotFetched,
                cloud_only_operation: CloudOnlyOperation::Idle,
                other_backups_operation: OtherBackupsOperation::Idle,
            }),
        };
    }

    fn apply_detail_refresh_failure(
        &mut self,
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
    ) {
        let retained = self.loaded_detail().cloned();
        self.detail_refresh = DetailRefreshActivity::Idle;
        self.detail = CloudBackupDetailState::Failed { reason, error, retained };
    }

    fn resolve_cloud_only_state(&mut self, cloud_only: CloudOnlyState) {
        if !matches!(cloud_only, CloudOnlyState::Loaded { .. }) {
            self.cloud_only_inventory.clear_loaded();
        }

        let detail_refresh_in_flight = self.detail_refresh == DetailRefreshActivity::InFlight;

        match &mut self.detail {
            CloudBackupDetailState::Complete { state } => state.cloud_only = cloud_only,
            CloudBackupDetailState::Checking { retained } => {
                let Some(state) = retained.as_mut() else { return };

                state.cloud_only = cloud_only;
                if !detail_refresh_in_flight && let Some(state) = retained.take() {
                    self.detail = CloudBackupDetailState::Complete { state };
                }
            }
            CloudBackupDetailState::Failed { retained: Some(state), .. } => {
                state.cloud_only = cloud_only;
            }
            CloudBackupDetailState::NotLoaded
            | CloudBackupDetailState::Failed { retained: None, .. } => {}
        }
    }

    fn resolve_other_backups_operation(&mut self, other_backups_operation: OtherBackupsOperation) {
        if matches!(&other_backups_operation, OtherBackupsOperation::Deleted) {
            self.other_backups = CloudBackupOtherBackupsState::NotChecked;
        }

        if let Some(state) = self.loaded_detail_mut() {
            state.other_backups_operation = other_backups_operation;
        }
    }
}

fn eligible_restore_all_wallets(wallets: Vec<CloudBackupWalletItem>) -> Vec<CloudBackupWalletItem> {
    wallets
        .into_iter()
        .filter(|wallet| wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::CloudBackupOtherBackupsSummary;

    impl InventoryOwner {
        fn detail(&self) -> Option<CloudBackupDetail> {
            self.loaded_detail().map(|state| state.detail.clone())
        }
    }

    fn test_detail() -> CloudBackupDetail {
        CloudBackupDetail { last_sync: None, up_to_date: Vec::new(), needs_sync: Vec::new() }
    }

    fn cloud_only_wallet(
        record_id: &str,
        sync_status: CloudBackupWalletStatus,
    ) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: record_id.into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: None,
            sync_status,
            restore_failure: None,
            record_id: record_id.into(),
        }
    }

    fn owner_with_cloud_only(wallets: Vec<CloudBackupWalletItem>) -> InventoryOwner {
        let mut owner = InventoryOwner::default();
        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: test_detail(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });
        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loaded {
            wallets,
            incomplete: false,
        }));
        owner
    }

    fn listed_record_ids(wallets: &[CloudBackupWalletItem]) -> HashSet<String> {
        wallets.iter().map(|wallet| wallet.record_id.clone()).collect()
    }

    // records the listing and loads a complete fetch for it, so a later detail listing the same
    // wallets keeps the rows
    fn load_listed_cloud_only(
        owner: &mut InventoryOwner,
        detail: &CloudBackupDetail,
        wallets: &[CloudBackupWalletItem],
    ) {
        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(wallets)),
        });
        owner.apply(InventoryInput::CloudOnlyFetchLoaded(CloudOnlyFetch {
            wallets: wallets.to_vec(),
            unresolved_record_ids: Vec::new(),
            listing_generation: owner.cloud_only_listing_generation(),
        }));
    }

    /// Restore All with no run in progress and no operation holding the slot
    fn restore_all_state(owner: &InventoryOwner) -> CloudBackupRestoreAllState {
        owner.restore_all(&CloudBackupRestoreAllRuntimeState::Idle, false)
    }

    /// Every row the detail screen lists, local rows first, then loaded cloud-only rows
    fn visible_rows(owner: &InventoryOwner) -> Vec<(String, CloudBackupWalletStatus)> {
        let loaded = match owner.detail_state().clone() {
            CloudBackupDetailState::Complete { state } => state,
            CloudBackupDetailState::Checking { retained: Some(state) }
            | CloudBackupDetailState::Failed { retained: Some(state), .. } => state,
            other => panic!("expected a retained detail, got {other:?}"),
        };
        let cloud_only = match loaded.cloud_only {
            CloudOnlyState::Loaded { wallets, .. } => wallets,
            CloudOnlyState::NotFetched
            | CloudOnlyState::Loading
            | CloudOnlyState::Failed { .. } => Vec::new(),
        };

        loaded
            .detail
            .up_to_date
            .into_iter()
            .chain(loaded.detail.needs_sync)
            .chain(cloud_only)
            .map(|wallet| (wallet.record_id, wallet.sync_status))
            .collect()
    }

    #[test]
    fn deleted_wallet_is_listed_once_when_the_detail_refresh_fails() {
        let deleted = cloud_only_wallet("deleted", CloudBackupWalletStatus::Confirmed);
        let kept = cloud_only_wallet("kept", CloudBackupWalletStatus::Dirty);
        let detail = CloudBackupDetail {
            last_sync: None,
            up_to_date: vec![deleted.clone()],
            needs_sync: vec![kept.clone()],
        };
        let mut owner = owner_with_cloud_only(Vec::new());
        load_listed_cloud_only(&mut owner, &detail, &[]);

        // the wallet is deleted from this device: its local row goes at once, and the cloud-only
        // rows are read again to pick up its backup
        let effects = owner.apply(InventoryInput::LocalWalletsChanged {
            local_record_ids: Some(HashSet::from([kept.record_id.clone()])),
        });

        assert!(effects.refetch_needed);
        assert_eq!(
            visible_rows(&owner),
            vec![(kept.record_id.clone(), CloudBackupWalletStatus::Dirty)]
        );

        // the throttled detail refresh fails, which keeps the retained detail as it now is
        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "provider unavailable".into(),
        });

        assert_eq!(
            visible_rows(&owner),
            vec![(kept.record_id.clone(), CloudBackupWalletStatus::Dirty)]
        );

        // the refetch lists the deleted wallet under Not on This Device, and only there
        owner.apply(InventoryInput::CloudOnlyFetchLoaded(CloudOnlyFetch {
            wallets: vec![cloud_only_wallet("deleted", CloudBackupWalletStatus::DeletedFromDevice)],
            unresolved_record_ids: Vec::new(),
            listing_generation: owner.cloud_only_listing_generation(),
        }));

        assert_eq!(
            visible_rows(&owner),
            vec![
                (kept.record_id, CloudBackupWalletStatus::Dirty),
                (deleted.record_id, CloudBackupWalletStatus::DeletedFromDevice),
            ]
        );
    }

    #[test]
    fn unreadable_local_wallets_leave_the_detail_rows_for_the_refresh() {
        let wallet = cloud_only_wallet("wallet", CloudBackupWalletStatus::Confirmed);
        let detail = CloudBackupDetail {
            last_sync: None,
            up_to_date: vec![wallet.clone()],
            needs_sync: Vec::new(),
        };
        let mut owner = owner_with_cloud_only(Vec::new());
        load_listed_cloud_only(&mut owner, &detail, &[]);

        owner.apply(InventoryInput::LocalWalletsChanged { local_record_ids: None });

        assert_eq!(
            visible_rows(&owner),
            vec![(wallet.record_id, CloudBackupWalletStatus::Confirmed)]
        );
    }

    #[test]
    fn already_on_device_result_read_before_a_local_wallet_change_is_discarded() {
        let wallet = cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice);
        let detail = test_detail();
        let mut owner = owner_with_cloud_only(Vec::new());
        load_listed_cloud_only(&mut owner, &detail, std::slice::from_ref(&wallet));
        let restore_generation = owner.cloud_only_listing_generation();

        // the matching local wallet is deleted and the refetch lands before the restore result
        owner.apply(InventoryInput::LocalWalletsChanged { local_record_ids: None });
        owner.apply(InventoryInput::CloudOnlyFetchLoaded(CloudOnlyFetch {
            wallets: vec![wallet.clone()],
            unresolved_record_ids: Vec::new(),
            listing_generation: owner.cloud_only_listing_generation(),
        }));
        owner.apply(InventoryInput::CloudOnlyRowAlreadyOnDevice {
            record_id: wallet.record_id.clone(),
            listing_generation: restore_generation,
        });

        let CloudOnlyState::Loaded { wallets, .. } = owner.cloud_only() else {
            panic!("expected the refetched rows");
        };
        assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::DeletedFromDevice);

        // a result read under the current generation still marks the row
        owner.apply(InventoryInput::CloudOnlyRowAlreadyOnDevice {
            record_id: wallet.record_id,
            listing_generation: owner.cloud_only_listing_generation(),
        });

        let CloudOnlyState::Loaded { wallets, .. } = owner.cloud_only() else {
            panic!("expected the refetched rows");
        };
        assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::AlreadyOnDevice);
    }

    #[test]
    fn restore_all_start_requires_two_supported_complete_rows() {
        let one_wallet = owner_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        assert_eq!(restore_all_state(&one_wallet), CloudBackupRestoreAllState::NotShown);

        let two_wallets = owner_with_cloud_only(vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("unsupported", CloudBackupWalletStatus::UnsupportedVersion),
            cloud_only_wallet("unknown", CloudBackupWalletStatus::RemoteStateUnknown),
        ]);
        assert_eq!(
            restore_all_state(&two_wallets),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn retained_restore_all_count_stays_enabled_until_inventory_becomes_incomplete() {
        let wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut checking = owner_with_cloud_only(wallets.clone());

        checking.apply(InventoryInput::DetailRefreshStarted);
        assert_eq!(
            restore_all_state(&checking),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );

        let mut failed = owner_with_cloud_only(wallets);
        failed.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "provider unavailable".into(),
        });
        assert_eq!(
            restore_all_state(&failed),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
    }

    #[test]
    fn completed_cloud_only_fetch_restores_complete_detail_with_loaded_wallets() {
        let mut owner = owner_with_cloud_only(vec![cloud_only_wallet(
            "stale-wallet",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];

        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loaded {
            wallets: loaded_wallets.clone(),
            incomplete: false,
        }));
        let CloudBackupDetailState::Complete { state } = owner.detail_state().clone() else {
            panic!("expected complete detail after cloud-only fetch");
        };

        assert_eq!(
            state.cloud_only,
            CloudOnlyState::Loaded { wallets: loaded_wallets, incomplete: false }
        );
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn cloud_only_completion_does_not_finish_authoritative_detail_refresh() {
        let mut owner = owner_with_cloud_only(vec![cloud_only_wallet(
            "stale-wallet",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];

        owner.apply(InventoryInput::DetailRefreshStarted);
        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loaded {
            wallets: loaded_wallets.clone(),
            incomplete: false,
        }));
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected authoritative detail to remain available during refresh");
        };
        assert_eq!(
            retained.cloud_only,
            CloudOnlyState::Loaded { wallets: loaded_wallets, incomplete: false }
        );
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn supplemental_cloud_only_failure_does_not_fail_primary_inventory() {
        let mut owner = owner_with_cloud_only(Vec::new());

        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply(InventoryInput::CloudOnlyStateResolved(CloudOnlyState::Failed {
            error: "provider is still syncing".into(),
        }));
        let CloudBackupDetailState::Complete { state } = owner.detail_state().clone() else {
            panic!("expected primary inventory to remain complete");
        };

        assert_eq!(
            state.cloud_only,
            CloudOnlyState::Failed { error: "provider is still syncing".into() }
        );
    }

    #[test]
    fn retained_detail_receives_terminal_operation_states() {
        let mut owner = owner_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let cloud_only_failure = CloudOnlyOperation::Failed { error: "restore failed".into() };
        let other_backups_failure =
            OtherBackupsOperation::Failed { error: "recovery failed".into() };

        owner.apply(InventoryInput::CloudOnlyOperationResolved(CloudOnlyOperation::Operating {
            record_id: "wallet-1".into(),
        }));
        owner.apply(InventoryInput::OtherBackupsOperationResolved(
            OtherBackupsOperation::Recovering,
        ));
        owner.apply(InventoryInput::DetailRefreshStarted);
        owner.apply(InventoryInput::CloudOnlyOperationResolved(cloud_only_failure.clone()));
        owner.apply(InventoryInput::OtherBackupsOperationResolved(other_backups_failure.clone()));
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected available detail with retained operation states");
        };
        assert_eq!(retained.cloud_only_operation, cloud_only_failure);
        assert_eq!(retained.other_backups_operation, other_backups_failure);

        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::Offline,
            error: "provider offline".into(),
        });
        owner.apply(InventoryInput::CloudOnlyOperationResolved(CloudOnlyOperation::Idle));
        owner.apply(InventoryInput::OtherBackupsOperationResolved(OtherBackupsOperation::Deleted));
        let CloudBackupDetailState::Failed { retained: Some(retained), .. } =
            owner.detail_state().clone()
        else {
            panic!("expected failed detail with retained operation states");
        };
        assert_eq!(retained.cloud_only_operation, CloudOnlyOperation::Idle);
        assert_eq!(retained.other_backups_operation, OtherBackupsOperation::Deleted);
    }

    #[test]
    fn deleted_other_backups_invalidates_loaded_inventory() {
        let mut owner = owner_with_cloud_only(Vec::new());
        owner.apply(InventoryInput::OtherBackupsStateResolved(
            CloudBackupOtherBackupsState::Loaded {
                summary: CloudBackupOtherBackupsSummary {
                    namespace_count: 1,
                    wallet_count: 2,
                    passkey_hints: Vec::new(),
                },
            },
        ));

        owner.apply(InventoryInput::OtherBackupsOperationResolved(OtherBackupsOperation::Deleted));

        assert_eq!(owner.other_backups_state(), CloudBackupOtherBackupsState::NotChecked);
    }

    #[test]
    fn conflicting_exclusive_operation_disables_restore_all_start() {
        let owner = owner_with_cloud_only(vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ]);

        // another exclusive operation holds the slot
        assert_eq!(
            owner.restore_all(&CloudBackupRestoreAllRuntimeState::Idle, true),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
    }

    #[test]
    fn restore_all_retry_allows_one_remaining_wallet_only_when_complete() {
        let retry_state = |owner: &InventoryOwner| {
            owner.restore_all(&CloudBackupRestoreAllRuntimeState::RetryRemaining, false)
        };
        let detail = test_detail();
        let wallets =
            vec![cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice)];
        let mut owner = owner_with_cloud_only(wallets.clone());
        load_listed_cloud_only(&mut owner, &detail, &wallets);

        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );

        owner.apply(InventoryInput::DetailRefreshStarted);
        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail,
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
        });
        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );
    }

    #[test]
    fn detail_refresh_failure_retains_last_known_rows() {
        let mut owner = InventoryOwner::default();
        let detail = test_detail();

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });
        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "iCloud unavailable".into(),
        });
        let CloudBackupDetailState::Failed { reason, error, retained: Some(retained) } =
            owner.detail_state().clone()
        else {
            panic!("expected retained failed detail");
        };

        assert_eq!(reason, CloudBackupInventoryIncompleteReason::ProviderUnavailable);
        assert_eq!(error, "iCloud unavailable");
        assert_eq!(retained.detail, detail);
        assert_eq!(owner.detail(), Some(detail));
    }

    #[test]
    fn refresh_keeps_confirmed_inventory_ready() {
        let mut owner = InventoryOwner::default();
        let detail = test_detail();

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });
        assert!(owner.detail_inventory_is_complete());

        owner.apply(InventoryInput::DetailRefreshStarted);
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected complete detail with retained rows");
        };

        assert_eq!(retained.detail, detail);
        assert_eq!(owner.detail(), Some(detail));
        assert!(owner.detail_inventory_is_complete());
        assert!(owner.detail_inventory_is_ready());
    }

    #[test]
    fn provisional_detail_is_visible_without_becoming_complete() {
        let mut owner = InventoryOwner::default();
        let provisional = test_detail();

        owner.apply(InventoryInput::DetailRefreshProvisional(provisional.clone()));
        let CloudBackupDetailState::Checking { retained: Some(retained) } =
            owner.detail_state().clone()
        else {
            panic!("expected provisional checking detail");
        };

        assert_eq!(retained.detail, provisional);
        assert!(!owner.detail_inventory_is_complete());
        assert!(!owner.detail_inventory_is_ready());
    }

    #[test]
    fn offline_failure_keeps_confirmed_rows_without_individual_wallet_actions() {
        let mut owner = InventoryOwner::default();
        let detail = test_detail();

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });
        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::Offline,
            error: "This device is offline".into(),
        });

        assert_eq!(owner.detail(), Some(detail));
        assert!(!owner.detail_inventory_is_complete());
        assert!(!owner.detail_inventory_is_ready());

        // a provider failure that isn't the network keeps the confirmed rows actionable
        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "iCloud unavailable".into(),
        });

        assert!(!owner.detail_inventory_is_complete());
        assert!(owner.detail_inventory_is_ready());
    }

    #[test]
    fn trusted_local_snapshot_allows_individual_wallet_actions() {
        let mut owner = InventoryOwner::default();

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: test_detail(),
            authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });

        assert!(!owner.detail_inventory_is_complete());
        assert!(owner.detail_inventory_is_ready());
    }

    #[test]
    fn provider_confirmation_upgrades_trusted_local_snapshot_and_keeps_cloud_only_rows() {
        let mut owner = InventoryOwner::default();
        let detail = test_detail();
        let wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
        });
        owner.apply(InventoryInput::CloudOnlyFetchLoaded(CloudOnlyFetch {
            wallets: wallets.clone(),
            unresolved_record_ids: Vec::new(),
            listing_generation: owner.cloud_only_listing_generation(),
        }));

        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail,
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
        });
        let CloudBackupDetailState::Complete { state } = owner.detail_state().clone() else {
            panic!("expected complete detail");
        };

        assert_eq!(state.cloud_only, CloudOnlyState::Loaded { wallets, incomplete: false });
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn smaller_provisional_snapshot_does_not_drop_retained_rows() {
        let mut owner = InventoryOwner::default();
        let retained = test_detail();

        owner.apply(InventoryInput::DetailRefreshApplied {
            detail: retained.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
        });
        owner.apply(InventoryInput::DetailRefreshProvisional(test_detail()));

        assert_eq!(owner.detail(), Some(retained));
        assert!(!owner.detail_inventory_is_complete());
    }

    #[test]
    fn initial_detail_refresh_failure_never_projects_confirmed_zero() {
        let mut owner = InventoryOwner::default();

        owner.apply(InventoryInput::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::Offline,
            error: "Drive unavailable".into(),
        });
        assert_eq!(
            owner.detail_state().clone(),
            CloudBackupDetailState::Failed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
                error: "Drive unavailable".into(),
                retained: None,
            }
        );
        assert_eq!(owner.detail(), None);
    }
}
