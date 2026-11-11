//! What Cloud Backup knows about this device's wallets and the records in the cloud
//!
//! The screen projection reads the inventory only through [`InventoryFacts`]. Until the inventory
//! owner holds these facts itself, they are read from the reducer's detail state, which is where
//! today's screen gets them

use super::admission::DetailInventory;
use super::screen::{
    CloudBackupListedRecord, CloudBackupOwnRecord, CloudBackupOwnRow, CloudBackupProgressSection,
};
use super::{
    CloudBackupDetailState, CloudBackupStateReducer, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudOnlyState, LoadedCloudBackupDetail, RecordId,
};

/// The inventory facts the screen projection reads
#[derive(Debug, Clone)]
pub(crate) struct InventoryFacts {
    readiness: DetailInventory,
    detail: CloudBackupDetailState,
}

/// The inventory as the screen draws it
#[derive(Debug, Clone)]
pub(crate) struct InventoryView {
    pub(crate) readiness: DetailInventory,
    /// Shown while the inventory loads or after a load failed
    pub(crate) progress: Option<CloudBackupProgressSection>,
    /// The rows of the last finished load, kept through a refresh or a failure
    pub(crate) rows: Option<InventoryRows>,
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
    /// The facts as the reducer holds them today
    pub(crate) fn from_reducer(reducer: &CloudBackupStateReducer) -> Self {
        let readiness = if reducer.detail_inventory_is_complete() {
            DetailInventory::Complete
        } else if reducer.detail_inventory_is_ready() {
            DetailInventory::Ready
        } else {
            DetailInventory::NotLoaded
        };

        let detail = reducer
            .configured_state()
            .map_or(CloudBackupDetailState::NotLoaded, |configured| configured.detail);

        Self { readiness, detail }
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
