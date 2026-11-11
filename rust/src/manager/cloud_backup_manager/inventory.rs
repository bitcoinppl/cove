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
    CloudOnlyState, InventoryOwner, LoadedCloudBackupDetail, OtherBackupsOperation, RecordId,
};

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

/// One cloud record this device doesn't back up
#[derive(Debug, Clone)]
pub(crate) struct ListedRecord {
    pub(crate) record_id: RecordId,
    pub(crate) record: CloudBackupListedRecord,
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
    let listed = match &loaded.cloud_only {
        CloudOnlyState::Loaded { wallets, .. } => wallets.iter().map(listed_record).collect(),
        _ => Vec::new(),
    };

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

fn listed_record(wallet: &CloudBackupWalletItem) -> ListedRecord {
    let record = match wallet.sync_status {
        CloudBackupWalletStatus::AlreadyOnDevice => CloudBackupListedRecord::ExtraCopy {
            wallet: wallet.clone(),
            covered_by: wallet.name.clone(),
        },
        CloudBackupWalletStatus::UnsupportedVersion => CloudBackupListedRecord::Unsupported,
        CloudBackupWalletStatus::RemoteStateUnknown => CloudBackupListedRecord::Unchecked,
        _ => CloudBackupListedRecord::NotOnDevice { wallet: wallet.clone() },
    };

    ListedRecord { record_id: RecordId::from(wallet.record_id.clone()), record }
}
