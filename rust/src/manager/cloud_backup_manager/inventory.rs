//! What Cloud Backup knows about this device's wallets and the records in the cloud
//!
//! The screen projection reads the inventory only through [`InventoryFacts`], built from the
//! inventory owner and, for Restore All's progress, the reducer

use super::admission::DetailInventory;
use super::screen::{
    CloudBackupListedRecord, CloudBackupOwnRecord, CloudBackupOwnRow, CloudBackupProgressSection,
};
use super::{
    CloudBackupDetailState, CloudBackupOtherBackupsState, CloudBackupRestoreAllState,
    CloudBackupStateReducer, CloudBackupWalletItem, CloudBackupWalletStatus, CloudOnlyOperation,
    InventoryOwner, LoadedCloudBackupDetail, OtherBackupsOperation, RecordId,
};
use crate::wallet_identity::CoveringWallets;

/// The inventory facts the screen projection reads
#[derive(Debug, Clone)]
pub(crate) struct InventoryFacts {
    readiness: DetailInventory,
    detail: CloudBackupDetailState,
    restore_all: CloudBackupRestoreAllState,
    other_backups: CloudBackupOtherBackupsState,
    cloud_only_operation: CloudOnlyOperation,
    other_backups_operation: OtherBackupsOperation,
}

/// The inventory as the screen draws it
#[derive(Debug, Clone)]
pub(crate) struct InventoryView {
    pub(crate) readiness: DetailInventory,
    /// Shown while the inventory loads or after a load failed
    pub(crate) progress: Option<CloudBackupProgressSection>,
    /// The rows of the last finished load, kept through a refresh or a failure
    pub(crate) rows: Option<InventoryRows>,
    pub(crate) restore_all: CloudBackupRestoreAllState,
    pub(crate) other_backups: CloudBackupOtherBackupsState,
    /// The restore or delete running on a listed record
    pub(crate) cloud_only_operation: CloudOnlyOperation,
    pub(crate) other_backups_operation: OtherBackupsOperation,
}

/// The rows of a loaded inventory
#[derive(Debug, Clone)]
pub(crate) struct InventoryRows {
    pub(crate) up_to_date: Vec<CloudBackupOwnRow>,
    pub(crate) needs_sync: Vec<CloudBackupOwnRow>,
    /// Cloud records this device doesn't back up, in listing order
    pub(crate) listed: Vec<ListedRecord>,
    pub(crate) last_sync: Option<u64>,
}

impl InventoryRows {
    /// Whether a wallet covering this extra copy has its own current backup, so the copy can go
    ///
    /// Matched by the covering wallets' own record IDs, so a renamed wallet still matches its
    /// copy and another wallet with the same name never does
    pub(crate) fn copy_has_current_backup(&self, listed: &ListedRecord) -> bool {
        self.up_to_date
            .iter()
            .chain(&self.needs_sync)
            .filter(|row| row.record == CloudBackupOwnRecord::Current)
            .map(|row| RecordId::from(row.wallet.record_id.clone()))
            .any(|own_record_id| listed.covered_by.contains(&own_record_id))
    }
}

/// One cloud record this device doesn't back up
#[derive(Debug, Clone)]
pub(crate) struct ListedRecord {
    pub(crate) record_id: RecordId,
    pub(crate) record: CloudBackupListedRecord,
    /// For an extra copy, the own-backup record IDs of the local wallets that cover it, so the
    /// copy is matched to their own rows by wallet ID and never by name; empty for other rows
    pub(crate) covered_by: Vec<RecordId>,
}

impl InventoryFacts {
    /// The facts as the owner holds them, with Restore All's run read from the reducer
    pub(crate) fn new(owner: &InventoryOwner, reducer: &CloudBackupStateReducer) -> Self {
        Self {
            readiness: owner.readiness(),
            detail: owner.detail_state().clone(),
            restore_all: owner
                .restore_all(reducer.restore_all_runtime(), reducer.active_operation().is_some()),
            other_backups: owner.other_backups_state(),
            cloud_only_operation: owner.cloud_only_operation(),
            other_backups_operation: owner.other_backups_operation(),
        }
    }

    pub(crate) fn view(&self) -> InventoryView {
        let progress = match &self.detail {
            CloudBackupDetailState::Checking { .. } => {
                Some(CloudBackupProgressSection::InventoryChecking)
            }
            CloudBackupDetailState::Failed { reason, .. } => {
                Some(CloudBackupProgressSection::InventoryIncomplete { reason: *reason })
            }
            CloudBackupDetailState::NotLoaded | CloudBackupDetailState::Complete { .. } => None,
        };

        InventoryView {
            readiness: self.readiness,
            progress,
            rows: loaded_detail(&self.detail).map(inventory_rows),
            restore_all: self.restore_all.clone(),
            other_backups: self.other_backups.clone(),
            cloud_only_operation: self.cloud_only_operation.clone(),
            other_backups_operation: self.other_backups_operation.clone(),
        }
    }
}

/// The detail rows to draw: a finished load, or the rows a refresh or failure kept
fn loaded_detail(detail: &CloudBackupDetailState) -> Option<&LoadedCloudBackupDetail> {
    match detail {
        CloudBackupDetailState::Complete { state } => Some(state),
        CloudBackupDetailState::Checking { retained }
        | CloudBackupDetailState::Failed { retained, .. } => retained.as_ref(),
        CloudBackupDetailState::NotLoaded => None,
    }
}

fn inventory_rows(loaded: &LoadedCloudBackupDetail) -> InventoryRows {
    let listed = loaded
        .cloud_only
        .loaded_rows()
        .unwrap_or_default()
        .iter()
        .map(|row| ListedRecord {
            record_id: RecordId::from(row.record_id.clone()),
            record: row.listed_record(),
            covered_by: row.covered_by().map(own_record_ids).unwrap_or_default(),
        })
        .collect();

    InventoryRows {
        up_to_date: loaded.detail.up_to_date.iter().map(own_row).collect(),
        needs_sync: loaded.detail.needs_sync.iter().map(own_row).collect(),
        listed,
        last_sync: loaded.detail.last_sync,
    }
}

fn own_row(wallet: &CloudBackupWalletItem) -> CloudBackupOwnRow {
    let record = match wallet.sync_status {
        CloudBackupWalletStatus::Confirmed | CloudBackupWalletStatus::AlreadyOnDevice => {
            CloudBackupOwnRecord::Current
        }
        CloudBackupWalletStatus::Dirty
        | CloudBackupWalletStatus::Uploading
        | CloudBackupWalletStatus::UploadedPendingConfirmation => CloudBackupOwnRecord::Pending,
        CloudBackupWalletStatus::Failed => CloudBackupOwnRecord::Stale,
        CloudBackupWalletStatus::DeletedFromDevice => CloudBackupOwnRecord::SecretMissingLocally,
        CloudBackupWalletStatus::UnsupportedVersion
        | CloudBackupWalletStatus::RemoteStateUnknown => CloudBackupOwnRecord::Missing,
    };

    CloudBackupOwnRow { wallet: wallet.clone(), record }
}

/// The record IDs of the covering wallets' own backups
fn own_record_ids(covered_by: &CoveringWallets) -> Vec<RecordId> {
    covered_by
        .iter()
        .map(|wallet| RecordId::from(cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref())))
        .collect()
}
