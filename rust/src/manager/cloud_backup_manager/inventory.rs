//! What Cloud Backup knows about this device's wallets and the records in the cloud
//!
//! The screen projection reads the inventory only through [`InventoryFacts`], built from the
//! inventory owner and, for Restore All's progress, the reducer

use std::collections::{BTreeMap, HashMap};

use super::admission::DetailInventory;
use super::owner::inventory::UnreadableRead;
use super::screen::{
    CloudBackupInventoryAssessment, CloudBackupListedRecord, CloudBackupOwnRecord,
    CloudBackupOwnRow,
};
use super::{
    CloudBackupDetailState, CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
    CloudBackupRestoreAllState, CloudBackupStateReducer, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudCopySecret, CloudOnlyOperation, CloudOnlyState, InventoryOwner,
    InventoryReadAt, LoadedCloudBackupDetail, OtherBackupsOperation, OwnSecretFacts, RecordId,
    RustCloudBackupManager,
};
use crate::wallet_identity::{BackupRecoveryMaterial, CoveringWallets};

/// The inventory facts the screen projection reads
#[derive(Debug, Clone)]
pub(crate) struct InventoryFacts {
    readiness: DetailInventory,
    classified: bool,
    read_at: InventoryReadAt,
    listing_generation: u64,
    rows_current: bool,
    verified_unreadable: Option<UnreadableRead>,
    detail: CloudBackupDetailState,
    restore_all: CloudBackupRestoreAllState,
    other_backups: CloudBackupOtherBackupsState,
    cloud_only_operation: CloudOnlyOperation,
    other_backups_operation: OtherBackupsOperation,
}

/// The inventory as the screen draws it
#[derive(Debug, Clone)]
pub(crate) struct InventoryView {
    /// The owner proved the inventory complete, current and classified
    pub(crate) classified: bool,
    /// The configuration and the local wallet changes the inventory reflects
    pub(crate) read_at: InventoryReadAt,
    /// The cloud-only listing the rows were classified under
    pub(crate) listing_generation: u64,
    /// The loaded rows were read under the latest listing, so a delete can rest on them
    pub(crate) rows_current: bool,
    /// The records a key the configuration verified failed to decrypt, read under the latest
    /// listing
    pub(crate) verified_unreadable: Option<UnreadableRead>,
    /// Shown while the inventory loads or after a load failed
    pub(crate) progress: Option<InventoryProgress>,
    /// The rows of the last finished load, kept through a refresh or a failure
    pub(crate) rows: Option<InventoryRows>,
    pub(crate) restore_all: CloudBackupRestoreAllState,
    pub(crate) other_backups: CloudBackupOtherBackupsState,
    /// The restore or delete running on a listed record
    pub(crate) cloud_only_operation: CloudOnlyOperation,
    pub(crate) other_backups_operation: OtherBackupsOperation,
}

/// A load still running, or one that failed, which the screen draws as its progress section
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InventoryProgress {
    Checking,
    Incomplete(CloudBackupInventoryIncompleteReason),
}

/// The rows of a loaded inventory
#[derive(Debug, Clone)]
pub(crate) struct InventoryRows {
    pub(crate) up_to_date: Vec<CloudBackupOwnRow>,
    pub(crate) needs_sync: Vec<CloudBackupOwnRow>,
    /// Cloud records this device doesn't back up, in listing order
    pub(crate) listed: Vec<ListedRecord>,
    /// The namespace `listed` was listed in, or `None` while no listing is loaded
    pub(crate) listed_namespace: Option<String>,
    pub(crate) last_sync: Option<u64>,
    /// What each own record's cloud copy holds, by record ID
    pub(crate) own_cloud_copies: HashMap<String, CloudCopySecret>,
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
            .any(|own_record_id| listed.covered_by().contains(&own_record_id))
    }
}

/// What the inventory knows that a Turn Off impact classifies a fresh listing and a fresh read
/// of this device's wallets with
///
/// Built from the view alone, so the impact reads the same rows the screen draws. It carries
/// what each own record's cloud copy holds, never whether this device held the secret when the
/// inventory read it: Turn Off reads that itself when it asks, since a wallet can lose its
/// secret without the inventory hearing of it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InventoryImpactInputs {
    /// How complete the inventory behind these classes is
    pub(crate) assessment: CloudBackupInventoryAssessment,
    /// What each own record's cloud copy holds, by record ID
    pub(crate) own_cloud_copies: BTreeMap<String, CloudCopySecret>,
    /// What each listed record that isn't a local wallet's own backup is, by record ID
    pub(crate) listed: BTreeMap<String, InventoryListedClass>,
    /// The configuration and the local wallet changes these facts reflect
    pub(crate) read_at: InventoryReadAt,
}

/// What a listed record that isn't a local wallet's own backup is
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InventoryListedClass {
    NotOnDevice { name: String },
    ExtraCopy(CopyCover),
    Unsupported,
    Unreadable,
    Unchecked,
}

/// Which local wallets an extra copy was read as covered by, and what it holds
///
/// The inventory read the coverage, but a wallet can lose its secret without the inventory
/// hearing of it, so Turn Off checks it again against its own read of this device's wallets
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyCover {
    /// The covering wallets' own-backup record IDs, which key this device's wallets
    pub(crate) covered_by: Vec<RecordId>,
    /// What the copy can recover, which a covering wallet must still hold
    pub(crate) material: BackupRecoveryMaterial,
}

impl From<&CoveringWallets> for CopyCover {
    fn from(covering: &CoveringWallets) -> Self {
        Self { covered_by: own_record_ids(covering), material: covering.material() }
    }
}

impl InventoryView {
    /// The inventory's facts for a Turn Off impact
    ///
    /// Complete only when the owner proves the inventory classified: the provider confirmed the
    /// listing and every listed record was read. Whether each own record's risk could be told is
    /// the impact's to tell. With no loaded rows the inventory knows nothing, so it's unknown
    pub(crate) fn impact_inputs(&self) -> InventoryImpactInputs {
        let reason = match self.progress {
            Some(InventoryProgress::Incomplete(reason)) => reason,
            _ => CloudBackupInventoryIncompleteReason::Unknown,
        };

        let Some(rows) = &self.rows else {
            return InventoryImpactInputs {
                assessment: CloudBackupInventoryAssessment::Unknown { reason },
                own_cloud_copies: BTreeMap::new(),
                listed: BTreeMap::new(),
                read_at: self.read_at,
            };
        };

        let listed = rows
            .listed
            .iter()
            .map(|listed| (listed.record_id.clone().into(), listed.class.clone()))
            .collect();

        let assessment = if self.classified {
            CloudBackupInventoryAssessment::Complete
        } else {
            CloudBackupInventoryAssessment::Partial { reason }
        };

        InventoryImpactInputs {
            assessment,
            own_cloud_copies: rows.own_cloud_copies.clone().into_iter().collect(),
            listed,
            read_at: self.read_at,
        }
    }
}

/// One cloud record this device doesn't back up
#[derive(Debug, Clone)]
pub(crate) struct ListedRecord {
    pub(crate) record_id: RecordId,
    pub(crate) record: CloudBackupListedRecord,
    /// What the record is, with an extra copy's covering wallets
    pub(crate) class: InventoryListedClass,
}

impl ListedRecord {
    /// For an extra copy, the own-backup record IDs of the local wallets that cover it, so the
    /// copy is matched to their own rows by wallet ID and never by name; empty for other rows
    pub(crate) fn covered_by(&self) -> &[RecordId] {
        match &self.class {
            InventoryListedClass::ExtraCopy(cover) => &cover.covered_by,
            InventoryListedClass::NotOnDevice { .. }
            | InventoryListedClass::Unsupported
            | InventoryListedClass::Unreadable
            | InventoryListedClass::Unchecked => &[],
        }
    }
}

impl InventoryRows {
    /// Any row is drawn, so progress over them shows inline rather than in their place
    pub(crate) fn has_rows(&self) -> bool {
        !self.up_to_date.is_empty() || !self.needs_sync.is_empty() || !self.listed.is_empty()
    }
}

impl RustCloudBackupManager {
    /// The inventory facts as the owner and the reducer hold them now
    pub(crate) fn inventory_facts(&self) -> InventoryFacts {
        // the reducer's lock is taken first, as every reader holding both does
        let reducer = self.state.read();
        InventoryFacts::new(&self.inventory.read(), &reducer)
    }
}

impl InventoryFacts {
    /// The facts as the owner holds them, with Restore All's run read from the reducer
    pub(crate) fn new(owner: &InventoryOwner, reducer: &CloudBackupStateReducer) -> Self {
        Self {
            readiness: owner.readiness(),
            classified: owner.classified().is_some(),
            read_at: owner.read_at(),
            listing_generation: owner.cloud_only_listing_generation(),
            rows_current: owner.loaded_rows_are_current(),
            verified_unreadable: owner.verified_unreadable().cloned(),
            detail: owner.detail_state().clone(),
            restore_all: owner
                .restore_all(reducer.restore_all_runtime(), reducer.active_operation().is_some()),
            other_backups: owner.other_backups_state(),
            cloud_only_operation: owner.cloud_only_operation(),
            other_backups_operation: owner.other_backups_operation(),
        }
    }

    /// How far the inventory has loaded, which admission reads
    pub(crate) fn readiness(&self) -> DetailInventory {
        self.readiness
    }

    /// Whether an inventory load is running, which a repeated Check Again joins
    pub(crate) fn checking(&self) -> bool {
        matches!(self.detail, CloudBackupDetailState::Checking { .. })
    }

    pub(crate) fn view(&self) -> InventoryView {
        let progress = match &self.detail {
            CloudBackupDetailState::Checking { .. } => Some(InventoryProgress::Checking),
            CloudBackupDetailState::Failed { reason, .. } => {
                Some(InventoryProgress::Incomplete(*reason))
            }
            // the backups not on this device load after the detail, so their failed fetch leaves
            // the detail complete; without this the screen would draw them as absent
            CloudBackupDetailState::Complete { state } => match state.cloud_only {
                CloudOnlyState::Failed { reason } => Some(InventoryProgress::Incomplete(reason)),
                CloudOnlyState::NotFetched
                | CloudOnlyState::Loading
                | CloudOnlyState::Loaded { .. } => None,
            },
            CloudBackupDetailState::NotLoaded => None,
        };

        InventoryView {
            classified: self.classified,
            read_at: self.read_at,
            listing_generation: self.listing_generation,
            rows_current: self.rows_current,
            verified_unreadable: self.verified_unreadable.clone(),
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
            class: row.impact_class(),
        })
        .collect();

    let own_row = |wallet| own_row(wallet, &loaded.own_secrets);

    InventoryRows {
        up_to_date: loaded.detail.up_to_date.iter().map(own_row).collect(),
        needs_sync: loaded.detail.needs_sync.iter().map(own_row).collect(),
        listed,
        listed_namespace: loaded.cloud_only.listed_namespace().map(str::to_owned),
        last_sync: loaded.detail.last_sync,
        own_cloud_copies: loaded.own_secrets.cloud_copies.clone(),
    }
}

fn own_row(wallet: &CloudBackupWalletItem, own_secrets: &OwnSecretFacts) -> CloudBackupOwnRow {
    // the cloud copy may be the only place the secret is, whatever state the upload is in
    if own_secrets.missing_locally.contains(&wallet.record_id) {
        return CloudBackupOwnRow {
            wallet: wallet.clone(),
            record: CloudBackupOwnRecord::SecretMissingLocally,
        };
    }

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
