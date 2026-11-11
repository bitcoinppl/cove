//! The inventory owner: the one writer of what Cloud Backup knows about this device's wallets,
//! the records in the active namespace, and the backups in other namespaces
//!
//! Producers (detail refreshes, cloud-only fetches, restore and delete results, local wallet
//! changes, the other-backups scan) hand the owner typed [`InventoryInput`]s. Only
//! [`InventoryOwner::apply`] changes the inventory, and every change that lands bumps the
//! generation the screen and Turn Off read the inventory at

mod listing;

use std::collections::{HashMap, HashSet};

pub(crate) use self::listing::{
    CloudOnlyFetch, CloudOnlyInventory, CloudOnlyListingUpdate, CloudOnlyRow,
};
use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;
use crate::manager::cloud_backup_manager::admission::DetailInventory;
use crate::manager::cloud_backup_manager::model::CloudBackupRestoreAllRuntimeState;
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
    CloudBackupRestoreAllState, CloudBackupWalletItem, CloudBackupWalletRestoreFailure,
    CloudOnlyOperation, OtherBackupsOperation, OwnSecretFacts,
};
use crate::wallet_identity::CoveringWallets;
use tracing::debug;

/// The loaded inventory of the active namespace
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedCloudBackupDetail {
    pub(crate) detail: CloudBackupDetail,
    /// Evidence used to accept the active namespace inventory
    pub(crate) inventory_authority: CloudBackupInventoryAuthority,
    pub(crate) cloud_only: CloudOnlyState,
    pub(crate) cloud_only_operation: CloudOnlyOperation,
    pub(crate) other_backups_operation: OtherBackupsOperation,
    pub(crate) own_secrets: OwnSecretFacts,
}

/// What a detail result says about the own records' secrets
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnSecretUpdate {
    /// The result read the own records, so these facts replace the loaded ones
    Read(OwnSecretFacts),
    /// The result carries no own-record reads, so the loaded facts stay
    Unread,
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
        rows: Vec<CloudOnlyRow>,
        /// The rows are not a complete read of the latest listing, so a record may be unread or
        /// missing
        incomplete: bool,
    },
    Failed {
        error: String,
    },
}

impl CloudOnlyState {
    /// The loaded rows, or `None` while none are loaded
    pub(crate) fn loaded_rows(&self) -> Option<&[CloudOnlyRow]> {
        match self {
            Self::Loaded { rows, .. } => Some(rows),
            Self::NotFetched | Self::Loading | Self::Failed { .. } => None,
        }
    }
}

/// Which configuration an inventory request was made under
///
/// Every producer reads it when its request starts and hands it back with the result, so a result
/// from before a configuration change (a reinitialize, a Drive account switch, a reset or a Turn
/// Off) is dropped instead of describing the configuration that replaced it. `None` is the scope
/// before the first configuration is published
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct InventoryScope(Option<ConfigGeneration>);

impl InventoryScope {
    pub(crate) fn of(generation: ConfigGeneration) -> Self {
        Self(Some(generation))
    }
}

/// A typed input to the inventory
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InventoryInput {
    /// The configuration changed, so everything the owner knows belongs to the old one
    ScopeEntered(InventoryScope),
    /// A producer's result, for the request it started under `scope`
    Result {
        scope: InventoryScope,
        result: InventoryResult,
    },

    // the inputs below come from a running row operation, whose claim keeps the configuration it
    // read until it ends, or from this device's own wallets, so they carry no scope
    /// A listed record was restored or deleted, so it is no longer cloud-only
    CloudOnlyRowRemoved(String),
    /// A restore found the local wallets `covered_by` holding everything the record can
    /// recover, applied only if no local wallet or listing change happened since the restore
    /// read the local wallets
    CloudOnlyRowAlreadyOnDevice {
        record_id: String,
        covered_by: CoveringWallets,
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
}

/// What a producer observed, applied only in the scope it was requested under
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InventoryResult {
    DetailRefreshStarted,
    DetailRefreshProvisional(CloudBackupDetail),
    DetailCleared,
    DetailRefreshApplied {
        detail: CloudBackupDetail,
        authority: CloudBackupInventoryAuthority,
        cloud_only_listing: CloudOnlyListingUpdate,
        own_secrets: OwnSecretUpdate,
    },
    /// A detail a verification built from the local wallets it read at `read_at`; applied like a
    /// refresh only if no local wallet change landed since
    VerifiedDetailApplied {
        read_at: InventoryReadAt,
        detail: CloudBackupDetail,
        cloud_only_listing: CloudOnlyListingUpdate,
        own_secrets: OwnSecretUpdate,
    },
    DetailRefreshFailed {
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
    },
    CloudOnlyStateResolved(CloudOnlyState),
    /// A cloud-only fetch finished, applied only if its listing generation is still current
    CloudOnlyFetchLoaded(CloudOnlyFetch),
    /// The other-backups scan, a supplemental read of the other namespaces
    OtherBackupsStateResolved(CloudBackupOtherBackupsState),
}

/// What applying an input asks of the caller
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InventoryEffects {
    /// The inventory changed, so the screen is projected again
    pub(crate) changed: bool,
    /// Cloud-only rows were invalidated or a fetch was stale, so a fetch has to repopulate them
    pub(crate) refetch_needed: bool,
    /// The result was requested under an earlier configuration and dropped, so its producer
    /// asks again under the current one
    pub(crate) dropped: bool,
}

/// How many local wallet changes the owner has applied
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LocalWalletsEpoch(u64);

/// What evidence about to read the cloud and the local wallets was read under: the configuration,
/// and how many local wallet changes the owner had applied
///
/// Evidence built under an older one is told apart from the inventory that replaced it
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InventoryReadAt {
    scope: InventoryScope,
    local_wallets: LocalWalletsEpoch,
}

impl InventoryReadAt {
    /// The configuration the evidence was read under
    pub(crate) fn scope(self) -> InventoryScope {
        self.scope
    }
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
    scope: InventoryScope,
    generation: InventoryGeneration,
    local_wallets_epoch: LocalWalletsEpoch,
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
            scope: InventoryScope::default(),
            generation: InventoryGeneration::default(),
            local_wallets_epoch: LocalWalletsEpoch::default(),
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
        let (refetch_needed, dropped) = match input {
            InventoryInput::Result { scope, .. } if scope != self.scope => {
                debug!("Cloud Backup dropped an inventory result from an earlier configuration");
                (false, true)
            }
            input => (self.apply_input(input), false),
        };

        let changed = *self != before;
        if changed {
            self.generation.0 += 1;
        }

        InventoryEffects { changed, refetch_needed, dropped }
    }

    fn apply_input(&mut self, input: InventoryInput) -> bool {
        match input {
            InventoryInput::ScopeEntered(scope) => self.enter_scope(scope),
            InventoryInput::Result { scope: _, result } => return self.apply_result(result),
            InventoryInput::CloudOnlyRowRemoved(record_id) => {
                self.remove_cloud_only_row(&record_id);
            }
            InventoryInput::CloudOnlyRowAlreadyOnDevice {
                record_id,
                covered_by,
                listing_generation,
            } => {
                self.mark_cloud_only_row_already_on_device(
                    &record_id,
                    covered_by,
                    listing_generation,
                );
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
        }

        false
    }

    /// Applies a result of the current scope, returning whether the cloud-only rows need a fetch
    fn apply_result(&mut self, result: InventoryResult) -> bool {
        match result {
            InventoryResult::DetailRefreshStarted => self.apply_detail_refresh_started(),
            InventoryResult::DetailRefreshProvisional(detail) => {
                self.apply_detail_refresh_provisional(detail);
            }
            InventoryResult::DetailRefreshApplied {
                detail,
                authority,
                cloud_only_listing,
                own_secrets,
            } => {
                return self.apply_detail_refresh(
                    detail,
                    authority,
                    cloud_only_listing,
                    own_secrets,
                );
            }
            InventoryResult::VerifiedDetailApplied {
                read_at,
                detail,
                cloud_only_listing,
                own_secrets,
            } => {
                // the detail lacks or still lists a wallet the change added or removed, and the
                // change already asked for the refresh that replaces it
                if read_at.local_wallets != self.local_wallets_epoch {
                    debug!("Cloud Backup dropped a verified detail read before a wallet change");
                    return false;
                }

                let authority = CloudBackupInventoryAuthority::ProviderConfirmed;
                return self.apply_detail_refresh(
                    detail,
                    authority,
                    cloud_only_listing,
                    own_secrets,
                );
            }
            InventoryResult::DetailCleared => self.apply_detail_cleared(),
            InventoryResult::DetailRefreshFailed { reason, error } => {
                self.apply_detail_refresh_failure(reason, error);
            }
            InventoryResult::CloudOnlyStateResolved(cloud_only) => {
                self.resolve_cloud_only_state(cloud_only);
            }
            InventoryResult::CloudOnlyFetchLoaded(fetch) => {
                return !self.apply_cloud_only_fetch_loaded(fetch);
            }
            InventoryResult::OtherBackupsStateResolved(other_backups) => {
                self.other_backups = other_backups;
            }
        }

        false
    }

    /// Starts the inventory of a new configuration from nothing
    ///
    /// The local-wallet epoch and the inventory generation carry on, and the cloud-only listing
    /// is invalidated rather than replaced, so its generation keeps rising past every fetch the
    /// old configuration started
    fn enter_scope(&mut self, scope: InventoryScope) {
        if scope == self.scope {
            return;
        }

        let mut cloud_only_inventory = std::mem::take(&mut self.cloud_only_inventory);
        cloud_only_inventory.invalidate();

        *self = Self {
            scope,
            generation: self.generation,
            local_wallets_epoch: self.local_wallets_epoch,
            cloud_only_inventory,
            ..Self::default()
        };
    }

    /// The configuration the inventory describes, which a producer reads when its request starts
    pub(crate) fn scope(&self) -> InventoryScope {
        self.scope
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

    /// What evidence about to read the cloud and the local wallets is read under
    pub(crate) fn read_at(&self) -> InventoryReadAt {
        InventoryReadAt { scope: self.scope, local_wallets: self.local_wallets_epoch }
    }

    pub(crate) fn cloud_only_listing_generation(&self) -> u64 {
        self.cloud_only_inventory.generation()
    }

    /// Whether the provider confirmed the listing the detail was built from
    ///
    /// Listing authority only: it says which records exist, not that every listed record was
    /// read, which [`Self::classified`] adds
    pub(crate) fn listing_is_provider_confirmed(&self) -> bool {
        matches!(
            &self.detail,
            CloudBackupDetailState::Complete { state }
                if state.inventory_authority == CloudBackupInventoryAuthority::ProviderConfirmed
        )
    }

    /// The inventory as proof that it is complete, current and classified, or `None` while it
    /// isn't
    ///
    /// Two facts make it: the provider confirmed the listing, and a fetch read every cloud-only
    /// record of exactly the latest listing
    pub(crate) fn classified(&self) -> Option<ClassifiedInventory<'_>> {
        if !self.listing_is_provider_confirmed() {
            return None;
        }

        let rows = self.loaded_detail()?.cloud_only.loaded_rows()?;
        let classified = self.cloud_only_inventory.loaded_is_complete_and_current(&row_ids(rows));
        let listing_generation = self.cloud_only_inventory.generation();

        classified.then_some(ClassifiedInventory { scope: self.scope, rows, listing_generation })
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
        if self.listing_is_provider_confirmed() {
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

    /// The run Restore All would start, or `None` while the inventory is not classified
    ///
    /// Restore All start and retry, and clearing its marker, all take the batch from here, so
    /// none of them can act on a subset that left a listed record unread
    pub(crate) fn restore_all_batch(&self) -> Option<RestoreAllBatch> {
        self.classified().map(ClassifiedInventory::restore_all_batch)
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
        let classified = self.classified();
        let wallet_count = self.restore_all_eligible_wallet_count();
        let available = classified.is_some() && !operation_active;

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
                // the marker clears only once the classified inventory shows nothing is left, so
                // loaded rows that can't show that keep the retry, disabled until they can; with
                // no rows loaded there is nothing to retry yet
                let rows_loaded = matches!(self.cloud_only(), CloudOnlyState::Loaded { .. });

                if wallet_count == 0 && (classified.is_some() || !rows_loaded) {
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
            cloud_only: CloudOnlyState::Loaded { rows, .. }, ..
        }) = self.loaded_detail()
        else {
            return 0;
        };

        rows.iter()
            .filter_map(CloudOnlyRow::restorable_wallet)
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
        own_secrets: OwnSecretUpdate,
    ) -> bool {
        let previous_loaded = self.loaded_detail().cloned();
        self.detail_refresh = DetailRefreshActivity::Idle;

        let own_secrets = match own_secrets {
            OwnSecretUpdate::Read(facts) => facts,
            OwnSecretUpdate::Unread => {
                previous_loaded.as_ref().map(|state| state.own_secrets.clone()).unwrap_or_default()
            }
        };

        if let CloudOnlyListingUpdate::Listed(record_ids) = &cloud_only_listing {
            self.cloud_only_inventory.record_listing(record_ids);
        }

        let (cloud_only, invalidated) = match previous_loaded.as_ref().map(|s| &s.cloud_only) {
            Some(CloudOnlyState::Loaded { rows, .. }) => {
                let keep = match &cloud_only_listing {
                    CloudOnlyListingUpdate::Listed(_) => {
                        self.cloud_only_inventory.keep_loaded_for_listing(&row_ids(rows))
                    }
                    CloudOnlyListingUpdate::Unlisted => false,
                };

                if keep {
                    (self.loaded_cloud_only(rows.clone()), false)
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
                own_secrets,
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
        for wallet in fetch.rows.iter_mut().filter_map(CloudOnlyRow::restorable_wallet_mut) {
            if wallet.restore_failure.is_none() {
                wallet.restore_failure = retained_failures.get(&wallet.record_id).cloned();
            }
        }

        self.cloud_only_inventory.record_loaded(&fetch);
        let cloud_only = self.loaded_cloud_only(fetch.rows);
        self.resolve_cloud_only_state(cloud_only);

        true
    }

    fn loaded_cloud_only_failures(&self) -> HashMap<String, CloudBackupWalletRestoreFailure> {
        let CloudOnlyState::Loaded { rows, .. } = self.cloud_only() else {
            return HashMap::new();
        };

        rows.iter()
            .filter_map(CloudOnlyRow::restorable_wallet)
            .filter_map(|wallet| {
                let failure = wallet.restore_failure.clone()?;
                Some((wallet.record_id.clone(), failure))
            })
            .collect()
    }

    /// Loaded rows marked incomplete by the same check the Restore All marker proof uses, so the
    /// retry stays visible whenever the marker cannot be cleared
    fn loaded_cloud_only(&self, rows: Vec<CloudOnlyRow>) -> CloudOnlyState {
        let incomplete = !self.cloud_only_inventory.loaded_is_complete_and_current(&row_ids(&rows));

        CloudOnlyState::Loaded { rows, incomplete }
    }

    /// Keeps a row whose recovery material a local wallet holds visible and deletable, without
    /// offering it for restore again
    fn mark_cloud_only_row_already_on_device(
        &mut self,
        record_id: &str,
        covered_by: CoveringWallets,
        listing_generation: u64,
    ) {
        // a local wallet change since the restore read the local wallets bumped the generation
        // and asked for a refetch, which classifies the row against the wallets that exist now
        if listing_generation != self.cloud_only_inventory.generation() {
            return;
        }

        let CloudOnlyState::Loaded { mut rows, incomplete } = self.cloud_only() else {
            return;
        };

        if let Some(row) = rows.iter_mut().find(|row| row.record_id == record_id)
            && let Some(wallet) = row.restorable_wallet()
        {
            *row = CloudOnlyRow::extra_copy(wallet.clone(), covered_by);
        }

        self.resolve_cloud_only_state(CloudOnlyState::Loaded { rows, incomplete });
    }

    fn mark_cloud_only_restore_failure(&mut self, record_id: &str, message: String) {
        let mut cloud_only = self.cloud_only();
        if let CloudOnlyState::Loaded { rows, .. } = &mut cloud_only
            && let Some(wallet) = rows
                .iter_mut()
                .filter_map(CloudOnlyRow::restorable_wallet_mut)
                .find(|wallet| wallet.record_id == record_id)
        {
            wallet.restore_failure = Some(CloudBackupWalletRestoreFailure { message });
        }

        self.resolve_cloud_only_state(cloud_only);
    }

    fn clear_cloud_only_restore_failures(&mut self, record_ids: &[String]) {
        let mut cloud_only = self.cloud_only();
        if let CloudOnlyState::Loaded { rows, .. } = &mut cloud_only {
            for wallet in rows.iter_mut().filter_map(CloudOnlyRow::restorable_wallet_mut) {
                if record_ids.contains(&wallet.record_id) {
                    wallet.restore_failure = None;
                }
            }
        }

        self.resolve_cloud_only_state(cloud_only);
    }

    /// Removes a row whose record stopped being cloud-only, keeping the listing in step with it
    fn remove_cloud_only_row(&mut self, record_id: &str) {
        let CloudOnlyState::Loaded { mut rows, incomplete } = self.cloud_only() else {
            return;
        };

        rows.retain(|row| row.record_id != record_id);
        self.cloud_only_inventory.forget_record(record_id);
        self.resolve_cloud_only_state(CloudOnlyState::Loaded { rows, incomplete });
    }

    /// Drops the local rows of wallets that are gone, and resets the cloud-only rows so a fetch
    /// classifies them against the wallets that exist now, returning whether one is needed
    ///
    /// A deleted wallet keeps its backup, so it moves to Not on This Device once the cloud-only
    /// rows are read again. Waiting for the throttled detail refresh, or for one that fails, left
    /// it listed as local as well
    fn apply_local_wallets_changed(&mut self, local_record_ids: Option<&HashSet<String>>) -> bool {
        self.local_wallets_epoch.0 += 1;

        if let Some(local_record_ids) = local_record_ids
            && let Some(state) = self.loaded_detail_mut()
        {
            state.detail.retain_local_wallets(local_record_ids);
            state.own_secrets.retain_local_wallets(local_record_ids);
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
                own_secrets: OwnSecretFacts::default(),
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

/// Proof that the inventory is complete, current and classified
///
/// Only [`InventoryOwner::classified`] builds one, so an action that needs every listed record
/// can only run on an inventory whose listing the provider confirmed and whose every record was
/// read, all within the configuration it names
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClassifiedInventory<'a> {
    scope: InventoryScope,
    rows: &'a [CloudOnlyRow],
    listing_generation: u64,
}

impl ClassifiedInventory<'_> {
    /// Every wallet a Restore All run restores, frozen with the listing it was classified under
    pub(crate) fn restore_all_batch(self) -> RestoreAllBatch {
        let wallets =
            self.rows.iter().filter_map(CloudOnlyRow::restorable_wallet).cloned().collect();

        RestoreAllBatch { scope: self.scope, wallets, listing_generation: self.listing_generation }
    }
}

/// The wallets of one Restore All run, taken from a [`ClassifiedInventory`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreAllBatch {
    scope: InventoryScope,
    wallets: Vec<CloudBackupWalletItem>,
    listing_generation: u64,
}

impl RestoreAllBatch {
    pub(crate) fn wallets(&self) -> &[CloudBackupWalletItem] {
        &self.wallets
    }

    /// The configuration the batch was classified under, which the run's own fetch reports to
    pub(crate) fn scope(&self) -> InventoryScope {
        self.scope
    }

    /// The wallets, and the listing generation the run's fetch is checked against
    pub(crate) fn into_parts(self) -> (Vec<CloudBackupWalletItem>, u64) {
        (self.wallets, self.listing_generation)
    }
}

fn row_ids(rows: &[CloudOnlyRow]) -> HashSet<String> {
    rows.iter().map(|row| row.record_id.clone()).collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::listing::CloudOnlyRecord;
    use super::{
        CloudOnlyRow, CloudOnlyState, InventoryEffects, InventoryInput, InventoryOwner,
        InventoryResult,
    };
    use crate::manager::cloud_backup_manager::{CloudBackupWalletItem, CloudBackupWalletStatus};
    use crate::wallet::metadata::WalletId;
    use crate::wallet_identity::{CoveringWallet, CoveringWallets};

    pub(crate) trait InventoryOwnerTestExt {
        /// Applies a result requested under the owner's current configuration
        fn apply_current(&mut self, result: InventoryResult) -> InventoryEffects;
    }

    impl InventoryOwnerTestExt for InventoryOwner {
        fn apply_current(&mut self, result: InventoryResult) -> InventoryEffects {
            self.apply(InventoryInput::Result { scope: self.scope(), result })
        }
    }

    /// The local wallet tests use to cover an extra copy
    pub(crate) fn test_covering_wallets() -> CoveringWallets {
        CoveringWallets::one(CoveringWallet {
            id: WalletId::from("local-wallet".to_string()),
            name: "Local wallet".to_string(),
        })
    }

    /// The row a fetch makes for a wallet item, read from the item's status
    pub(crate) fn listed_row(wallet: &CloudBackupWalletItem) -> CloudOnlyRow {
        let record_id = wallet.record_id.clone();

        match wallet.sync_status {
            CloudBackupWalletStatus::AlreadyOnDevice => {
                CloudOnlyRow::extra_copy(wallet.clone(), test_covering_wallets())
            }
            CloudBackupWalletStatus::UnsupportedVersion => CloudOnlyRow::unsupported(record_id),
            CloudBackupWalletStatus::RemoteStateUnknown => CloudOnlyRow::unchecked(record_id),
            _ => CloudOnlyRow::not_on_device(wallet.clone()),
        }
    }

    pub(crate) fn listed_rows(wallets: &[CloudBackupWalletItem]) -> Vec<CloudOnlyRow> {
        wallets.iter().map(listed_row).collect()
    }

    /// The wallet a read row holds
    pub(crate) fn row_wallet(row: &CloudOnlyRow) -> Option<&CloudBackupWalletItem> {
        match &row.record {
            CloudOnlyRecord::NotOnDevice(wallet) | CloudOnlyRecord::ExtraCopy { wallet, .. } => {
                Some(wallet)
            }
            CloudOnlyRecord::Unsupported
            | CloudOnlyRecord::Unreadable { .. }
            | CloudOnlyRecord::Unchecked => None,
        }
    }

    pub(crate) trait CloudOnlyStateTestExt {
        /// The wallets of the loaded rows Cove could read, or `None` while none are loaded
        fn loaded_wallets(&self) -> Option<Vec<CloudBackupWalletItem>>;
    }

    impl CloudOnlyStateTestExt for CloudOnlyState {
        fn loaded_wallets(&self) -> Option<Vec<CloudBackupWalletItem>> {
            let rows = self.loaded_rows()?;

            Some(rows.iter().filter_map(row_wallet).cloned().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        CloudOnlyStateTestExt as _, InventoryOwnerTestExt as _, listed_rows, test_covering_wallets,
    };
    use super::*;
    use crate::manager::cloud_backup_manager::inventory::{InventoryFacts, InventoryRows};
    use crate::manager::cloud_backup_manager::screen::{
        CloudBackupListedRecord, CloudBackupUnreadableKind,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupOtherBackupsSummary, CloudBackupStateReducer, CloudBackupWalletStatus,
    };
    use crate::wallet::metadata::WalletId;
    use crate::wallet_identity::{CoveringWallet, CoveringWallets};

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

    /// An owner whose provider-confirmed listing found these wallets, read by a fetch
    fn owner_with_cloud_only(wallets: Vec<CloudBackupWalletItem>) -> InventoryOwner {
        let mut owner = InventoryOwner::default();
        load_listed_cloud_only(&mut owner, &test_detail(), &wallets);
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
        load_listed_rows(owner, detail, listed_rows(wallets));
    }

    // records a provider-confirmed listing of these rows and applies the fetch that read them
    fn load_listed_rows(
        owner: &mut InventoryOwner,
        detail: &CloudBackupDetail,
        rows: Vec<CloudOnlyRow>,
    ) {
        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(row_ids(&rows)),
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows,
            listing_generation: owner.cloud_only_listing_generation(),
        }));
    }

    /// The rows of a provider-confirmed listing whose fetch read two restorable wallets and an
    /// unsupported record, and could not read one more record
    fn mixed_fetch_rows() -> Vec<CloudOnlyRow> {
        let mut rows = listed_rows(&[
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("unsupported", CloudBackupWalletStatus::UnsupportedVersion),
        ]);
        rows.push(CloudOnlyRow::unreadable("unread".into(), CloudBackupUnreadableKind::Decrypt));
        rows
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
        let cloud_only = loaded.cloud_only.loaded_wallets().unwrap_or_default();

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
    fn a_new_configuration_drops_every_result_its_predecessor_started() {
        let wallets = vec![
            cloud_only_wallet("a", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("b", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut owner = owner_with_cloud_only(wallets.clone());
        let old_scope = owner.scope();
        assert!(owner.restore_all_batch().is_some());

        let entered =
            owner.apply(InventoryInput::ScopeEntered(InventoryScope::of(ConfigGeneration::next())));

        // the proof names its configuration, so the old inventory proves nothing in the new one
        assert!(entered.changed);
        assert!(owner.classified().is_none());
        assert_eq!(owner.detail_state(), &CloudBackupDetailState::NotLoaded);

        // results the old configuration started, provisional and supplemental ones included
        let entered_state = owner.clone();
        let stale_results = [
            InventoryResult::DetailRefreshProvisional(test_detail()),
            InventoryResult::DetailRefreshApplied {
                detail: test_detail(),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
                cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
                own_secrets: OwnSecretUpdate::Unread,
            },
            InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
                rows: listed_rows(&wallets),
                listing_generation: owner.cloud_only_listing_generation(),
            }),
            InventoryResult::OtherBackupsStateResolved(CloudBackupOtherBackupsState::Checking),
        ];
        for result in stale_results {
            let effects = owner.apply(InventoryInput::Result { scope: old_scope, result });

            assert!(effects.dropped);
            assert!(!effects.changed);
            assert_eq!(owner, entered_state);
        }

        // the new configuration's own load is applied
        load_listed_cloud_only(&mut owner, &test_detail(), &wallets);
        let batch = owner.restore_all_batch().expect("the new configuration's inventory");
        assert_eq!(batch.scope(), owner.scope());
        assert_eq!(batch.wallets(), wallets.as_slice());
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
        owner.apply_current(InventoryResult::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "provider unavailable".into(),
        });

        assert_eq!(
            visible_rows(&owner),
            vec![(kept.record_id.clone(), CloudBackupWalletStatus::Dirty)]
        );

        // the refetch lists the deleted wallet under Not on This Device, and only there
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: listed_rows(&[cloud_only_wallet(
                "deleted",
                CloudBackupWalletStatus::DeletedFromDevice,
            )]),
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
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: listed_rows(std::slice::from_ref(&wallet)),
            listing_generation: owner.cloud_only_listing_generation(),
        }));
        owner.apply(InventoryInput::CloudOnlyRowAlreadyOnDevice {
            record_id: wallet.record_id.clone(),
            covered_by: test_covering_wallets(),
            listing_generation: restore_generation,
        });

        let Some(wallets) = owner.cloud_only().loaded_wallets() else {
            panic!("expected the refetched rows");
        };
        assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::DeletedFromDevice);

        // a result read under the current generation still marks the row
        owner.apply(InventoryInput::CloudOnlyRowAlreadyOnDevice {
            record_id: wallet.record_id,
            covered_by: test_covering_wallets(),
            listing_generation: owner.cloud_only_listing_generation(),
        });

        let Some(wallets) = owner.cloud_only().loaded_wallets() else {
            panic!("expected the refetched rows");
        };
        assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::AlreadyOnDevice);
    }

    #[test]
    fn restore_all_start_requires_two_restorable_rows_of_a_classified_inventory() {
        let one_wallet = owner_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        assert_eq!(restore_all_state(&one_wallet), CloudBackupRestoreAllState::NotShown);

        // an unsupported record was read far enough to classify, so it holds nothing back
        let mut classified = InventoryOwner::default();
        let mut read_rows = mixed_fetch_rows();
        read_rows.retain(CloudOnlyRow::is_read);
        load_listed_rows(&mut classified, &test_detail(), read_rows);

        assert_eq!(
            restore_all_state(&classified),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
        let batch = classified.restore_all_batch().expect("a classified inventory has a batch");
        let batch_ids: Vec<_> = batch.wallets().iter().map(|w| w.record_id.as_str()).collect();
        assert_eq!(batch_ids, ["wallet-1", "wallet-2"]);
    }

    #[test]
    fn restore_all_waits_for_every_listed_record_of_a_confirmed_listing_to_be_read() {
        let mut owner = InventoryOwner::default();
        load_listed_rows(&mut owner, &test_detail(), mixed_fetch_rows());

        // the provider confirmed the listing, but the unread record may be one more wallet
        assert!(owner.listing_is_provider_confirmed());
        assert!(matches!(owner.cloud_only(), CloudOnlyState::Loaded { incomplete: true, .. }));
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
        assert_eq!(owner.restore_all_batch(), None);
    }

    #[test]
    fn restore_all_retry_of_an_unread_record_stays_disabled_with_no_restorable_row() {
        let retry_state = |owner: &InventoryOwner| {
            owner.restore_all(&CloudBackupRestoreAllRuntimeState::RetryRemaining, false)
        };
        let unread = CloudOnlyRow::unreadable("unread".into(), CloudBackupUnreadableKind::Decrypt);
        let mut owner = InventoryOwner::default();
        load_listed_rows(&mut owner, &test_detail(), vec![unread]);

        // the record could still need restoring, so the retry stays, but nothing proves what
        // a retry would restore
        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryDisabled { wallet_count: 0 }
        );
        assert_eq!(owner.restore_all_batch(), None);

        // once every record is read and none is left, the retry is gone and the batch is empty
        load_listed_rows(&mut owner, &test_detail(), Vec::new());
        assert_eq!(retry_state(&owner), CloudBackupRestoreAllState::NotShown);
        assert_eq!(owner.restore_all_batch().map(|batch| batch.wallets().len()), Some(0));
    }

    #[test]
    fn restore_all_retry_waits_for_the_provider_to_confirm_a_read_listing() {
        let retry_state = |owner: &InventoryOwner| {
            owner.restore_all(&CloudBackupRestoreAllRuntimeState::RetryRemaining, false)
        };
        let mut owner = InventoryOwner::default();
        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: test_detail(),
            authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(HashSet::new()),
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: Vec::new(),
            listing_generation: owner.cloud_only_listing_generation(),
        }));

        // every listed record was read, but only the provider can say none is missing
        assert!(matches!(owner.cloud_only(), CloudOnlyState::Loaded { incomplete: false, .. }));
        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryDisabled { wallet_count: 0 }
        );
        assert_eq!(owner.restore_all_batch(), None);
    }

    #[test]
    fn retained_restore_all_count_stays_enabled_until_inventory_becomes_incomplete() {
        let wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut checking = owner_with_cloud_only(wallets.clone());

        checking.apply_current(InventoryResult::DetailRefreshStarted);
        assert_eq!(
            restore_all_state(&checking),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );

        let mut failed = owner_with_cloud_only(wallets);
        failed.apply_current(InventoryResult::DetailRefreshFailed {
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
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut owner = owner_with_cloud_only(loaded_wallets.clone());

        owner.apply_current(InventoryResult::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: listed_rows(&loaded_wallets),
            listing_generation: owner.cloud_only_listing_generation(),
        }));
        let CloudBackupDetailState::Complete { state } = owner.detail_state().clone() else {
            panic!("expected complete detail after cloud-only fetch");
        };

        assert_eq!(
            state.cloud_only,
            CloudOnlyState::Loaded { rows: listed_rows(&loaded_wallets), incomplete: false }
        );
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn cloud_only_completion_does_not_finish_authoritative_detail_refresh() {
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut owner = owner_with_cloud_only(loaded_wallets.clone());

        owner.apply_current(InventoryResult::DetailRefreshStarted);
        owner.apply_current(InventoryResult::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: listed_rows(&loaded_wallets),
            listing_generation: owner.cloud_only_listing_generation(),
        }));
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected authoritative detail to remain available during refresh");
        };
        assert_eq!(
            retained.cloud_only,
            CloudOnlyState::Loaded { rows: listed_rows(&loaded_wallets), incomplete: false }
        );
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn supplemental_cloud_only_failure_does_not_fail_primary_inventory() {
        let mut owner = owner_with_cloud_only(Vec::new());

        owner.apply_current(InventoryResult::CloudOnlyStateResolved(CloudOnlyState::Loading));
        owner.apply_current(InventoryResult::CloudOnlyStateResolved(CloudOnlyState::Failed {
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
        owner.apply_current(InventoryResult::DetailRefreshStarted);
        owner.apply(InventoryInput::CloudOnlyOperationResolved(cloud_only_failure.clone()));
        owner.apply(InventoryInput::OtherBackupsOperationResolved(other_backups_failure.clone()));
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected available detail with retained operation states");
        };
        assert_eq!(retained.cloud_only_operation, cloud_only_failure);
        assert_eq!(retained.other_backups_operation, other_backups_failure);

        owner.apply_current(InventoryResult::DetailRefreshFailed {
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
        owner.apply_current(InventoryResult::OtherBackupsStateResolved(
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

        owner.apply_current(InventoryResult::DetailRefreshStarted);
        assert_eq!(
            retry_state(&owner),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail,
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
            own_secrets: OwnSecretUpdate::Unread,
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

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::DetailRefreshFailed {
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

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });
        assert!(owner.listing_is_provider_confirmed());

        owner.apply_current(InventoryResult::DetailRefreshStarted);
        let CloudBackupDetailState::Complete { state: retained } = owner.detail_state().clone()
        else {
            panic!("expected complete detail with retained rows");
        };

        assert_eq!(retained.detail, detail);
        assert_eq!(owner.detail(), Some(detail));
        assert!(owner.listing_is_provider_confirmed());
        assert!(owner.detail_inventory_is_ready());
    }

    #[test]
    fn provisional_detail_is_visible_without_becoming_complete() {
        let mut owner = InventoryOwner::default();
        let provisional = test_detail();

        owner.apply_current(InventoryResult::DetailRefreshProvisional(provisional.clone()));
        let CloudBackupDetailState::Checking { retained: Some(retained) } =
            owner.detail_state().clone()
        else {
            panic!("expected provisional checking detail");
        };

        assert_eq!(retained.detail, provisional);
        assert!(!owner.listing_is_provider_confirmed());
        assert!(!owner.detail_inventory_is_ready());
    }

    #[test]
    fn offline_failure_keeps_confirmed_rows_without_individual_wallet_actions() {
        let mut owner = InventoryOwner::default();
        let detail = test_detail();

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::Offline,
            error: "This device is offline".into(),
        });

        assert_eq!(owner.detail(), Some(detail));
        assert!(!owner.listing_is_provider_confirmed());
        assert!(!owner.detail_inventory_is_ready());

        // a provider failure that isn't the network keeps the confirmed rows actionable
        owner.apply_current(InventoryResult::DetailRefreshFailed {
            reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
            error: "iCloud unavailable".into(),
        });

        assert!(!owner.listing_is_provider_confirmed());
        assert!(owner.detail_inventory_is_ready());
    }

    #[test]
    fn trusted_local_snapshot_allows_individual_wallet_actions() {
        let mut owner = InventoryOwner::default();

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: test_detail(),
            authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });

        assert!(!owner.listing_is_provider_confirmed());
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

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: detail.clone(),
            authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::CloudOnlyFetchLoaded(CloudOnlyFetch {
            rows: listed_rows(&wallets),
            listing_generation: owner.cloud_only_listing_generation(),
        }));

        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail,
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(listed_record_ids(&wallets)),
            own_secrets: OwnSecretUpdate::Unread,
        });
        let CloudBackupDetailState::Complete { state } = owner.detail_state().clone() else {
            panic!("expected complete detail");
        };

        assert_eq!(
            state.cloud_only,
            CloudOnlyState::Loaded { rows: listed_rows(&wallets), incomplete: false }
        );
        assert_eq!(
            restore_all_state(&owner),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn smaller_provisional_snapshot_does_not_drop_retained_rows() {
        let mut owner = InventoryOwner::default();
        let retained = test_detail();

        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: retained.clone(),
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });
        owner.apply_current(InventoryResult::DetailRefreshProvisional(test_detail()));

        assert_eq!(owner.detail(), Some(retained));
        assert!(!owner.listing_is_provider_confirmed());
    }

    #[test]
    fn initial_detail_refresh_failure_never_projects_confirmed_zero() {
        let mut owner = InventoryOwner::default();

        owner.apply_current(InventoryResult::DetailRefreshFailed {
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

    /// A local wallet's own backup, named as shown and keyed by its wallet ID
    fn own_wallet(
        id: &str,
        name: &str,
        sync_status: CloudBackupWalletStatus,
    ) -> (CoveringWallet, CloudBackupWalletItem) {
        let covering = CoveringWallet { id: WalletId::from(id.to_string()), name: name.into() };
        let item = CloudBackupWalletItem {
            name: name.into(),
            record_id: cove_cspp::backup_data::wallet_record_id(id),
            ..cloud_only_wallet(id, sync_status)
        };

        (covering, item)
    }

    /// The listed rows as the screen reads them, for an owner holding these own backups and
    /// extra copies
    fn rows_with_extra_copies(
        own: Vec<CloudBackupWalletItem>,
        copies: Vec<(CloudBackupWalletItem, CoveringWallet)>,
    ) -> InventoryRows {
        let (up_to_date, needs_sync) = own
            .into_iter()
            .partition(|wallet| wallet.sync_status == CloudBackupWalletStatus::Confirmed);

        let mut owner = InventoryOwner::default();
        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: CloudBackupDetail { last_sync: None, up_to_date, needs_sync },
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Unlisted,
            own_secrets: OwnSecretUpdate::Unread,
        });

        let rows = copies
            .into_iter()
            .map(|(copy, covering)| CloudOnlyRow::extra_copy(copy, CoveringWallets::one(covering)))
            .collect();

        owner.apply_current(InventoryResult::CloudOnlyStateResolved(CloudOnlyState::Loaded {
            rows,
            incomplete: false,
        }));

        let facts = InventoryFacts::new(&owner, &CloudBackupStateReducer::default());
        facts.view().rows.expect("expected loaded rows")
    }

    #[test]
    fn extra_copy_is_replaced_only_by_its_own_covering_wallet() {
        // two local wallets share a name; only the first has a current backup
        let (backed_up, backed_up_item) =
            own_wallet("wallet-a", "Test Wallet", CloudBackupWalletStatus::Confirmed);
        let (not_backed_up, not_backed_up_item) =
            own_wallet("wallet-b", "Test Wallet", CloudBackupWalletStatus::Dirty);

        let copy_of_a = cloud_only_wallet("old-a", CloudBackupWalletStatus::AlreadyOnDevice);
        let copy_of_b = CloudBackupWalletItem {
            name: "Test Wallet".into(),
            ..cloud_only_wallet("old-b", CloudBackupWalletStatus::AlreadyOnDevice)
        };

        let rows = rows_with_extra_copies(
            vec![backed_up_item, not_backed_up_item],
            vec![(copy_of_a, backed_up), (copy_of_b, not_backed_up)],
        );

        let [copy_of_a, copy_of_b] = rows.listed.as_slice() else {
            panic!("expected two extra copies, got {:?}", rows.listed);
        };

        assert!(rows.copy_has_current_backup(copy_of_a));
        assert!(
            !rows.copy_has_current_backup(copy_of_b),
            "a same-name wallet's current backup must not replace another wallet's copy"
        );
    }

    #[test]
    fn extra_copy_of_a_renamed_wallet_is_replaced_by_its_current_backup() {
        let (renamed, renamed_item) =
            own_wallet("wallet-a", "Test Wallet++", CloudBackupWalletStatus::Confirmed);
        let old_copy = CloudBackupWalletItem {
            name: "Test Wallet".into(),
            ..cloud_only_wallet("old-a", CloudBackupWalletStatus::AlreadyOnDevice)
        };

        let rows = rows_with_extra_copies(vec![renamed_item], vec![(old_copy, renamed)]);

        let [old_copy] = rows.listed.as_slice() else {
            panic!("expected one extra copy, got {:?}", rows.listed);
        };

        let CloudBackupListedRecord::ExtraCopy { covered_by, .. } = &old_copy.record else {
            panic!("expected an extra copy, got {:?}", old_copy.record);
        };

        assert_eq!(covered_by, "Test Wallet++");
        assert!(rows.copy_has_current_backup(old_copy));
    }

    #[test]
    fn verification_evidence_read_before_a_wallet_change_is_discarded() {
        let (_, kept) = own_wallet("wallet-a", "Kept", CloudBackupWalletStatus::Confirmed);
        let (_, added) = own_wallet("wallet-b", "Added", CloudBackupWalletStatus::Dirty);
        let verified =
            |wallets: Vec<CloudBackupWalletItem>, read_at| InventoryResult::VerifiedDetailApplied {
                read_at,
                detail: CloudBackupDetail {
                    last_sync: None,
                    up_to_date: wallets,
                    needs_sync: Vec::new(),
                },
                cloud_only_listing: CloudOnlyListingUpdate::Listed(HashSet::new()),
                own_secrets: OwnSecretUpdate::Unread,
            };

        let mut owner = InventoryOwner::default();
        let read_before_change = owner.read_at();

        // a wallet is added while the verification reads the cloud, and its refresh lands first
        owner.apply(InventoryInput::LocalWalletsChanged { local_record_ids: None });
        owner.apply_current(InventoryResult::DetailRefreshApplied {
            detail: CloudBackupDetail {
                last_sync: None,
                up_to_date: vec![kept.clone()],
                needs_sync: vec![added.clone()],
            },
            authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            cloud_only_listing: CloudOnlyListingUpdate::Listed(HashSet::new()),
            own_secrets: OwnSecretUpdate::Unread,
        });

        let effects = owner.apply_current(verified(vec![kept.clone()], read_before_change));
        assert!(!effects.changed, "evidence from before the change must not replace the refresh");
        assert_eq!(owner.detail().map(|detail| detail.needs_sync), Some(vec![added]));

        // evidence read after the change still applies
        let effects = owner.apply_current(verified(vec![kept.clone()], owner.read_at()));
        assert!(effects.changed);
        assert_eq!(owner.detail().map(|detail| detail.up_to_date), Some(vec![kept]));
    }
}
