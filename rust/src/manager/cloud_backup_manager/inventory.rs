//! What Cloud Backup knows about this device's wallets and the records in the cloud
//!
//! The screen projection reads the inventory only through [`InventoryFacts`], built from the
//! inventory owner and, for Restore All's progress, the reducer

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::admission::DetailInventory;
use super::screen::{
    CloudBackupInventoryAssessment, CloudBackupListedRecord, CloudBackupOwnRecord,
    CloudBackupOwnRow, CloudBackupProgressSection,
};
use super::{
    CloudBackupDetailState, CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
    CloudBackupRestoreAllState, CloudBackupStateReducer, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudOnlyOperation, InventoryOwner, LoadedCloudBackupDetail,
    OtherBackupsOperation, OwnSecretFacts, RecordId,
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
    /// Own record IDs of wallets holding no secret here whose cloud copy couldn't be read, so
    /// whether only the cloud holds their secret is unchecked
    pub(crate) secret_unchecked: HashSet<String>,
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

/// What the inventory knows that a Turn Off impact classifies a fresh listing with
///
/// Built from the view alone, so the impact reads the same rows the screen draws
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the Turn Off impact reads these, and only tests until it does"
    )
)]
pub(crate) struct InventoryImpactInputs {
    /// How complete the inventory behind these classes is
    pub(crate) assessment: CloudBackupInventoryAssessment,
    /// Own record IDs of this device's wallets the loaded rows show
    pub(crate) own: BTreeSet<String>,
    /// Own records whose cloud copy holds a secret this device doesn't, by record ID, with names
    pub(crate) secret_missing_locally: BTreeMap<String, String>,
    /// Own records whose cloud copy couldn't be read to tell, by record ID, with names
    pub(crate) secret_unchecked: BTreeMap<String, String>,
    /// What each listed record that isn't a local wallet's own backup is, by record ID
    pub(crate) listed: BTreeMap<String, InventoryListedClass>,
}

/// What a listed record that isn't a local wallet's own backup is
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the Turn Off impact reads these, and only tests until it does"
    )
)]
pub(crate) enum InventoryListedClass {
    NotOnDevice { name: String },
    ExtraCopy,
    Unsupported,
    Unreadable,
    Unchecked,
}

impl From<&CloudBackupListedRecord> for InventoryListedClass {
    fn from(record: &CloudBackupListedRecord) -> Self {
        match record {
            CloudBackupListedRecord::NotOnDevice { wallet } => {
                Self::NotOnDevice { name: wallet.name.clone() }
            }
            CloudBackupListedRecord::ExtraCopy { .. } => Self::ExtraCopy,
            CloudBackupListedRecord::Unsupported => Self::Unsupported,
            CloudBackupListedRecord::Unreadable { .. } => Self::Unreadable,
            CloudBackupListedRecord::Unchecked => Self::Unchecked,
        }
    }
}

impl InventoryView {
    /// The inventory's facts for a Turn Off impact
    ///
    /// Complete only when every listed record was read and every own record's secret was
    /// checked. With no loaded rows the inventory knows nothing, so it's unknown
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the Turn Off impact reads these, and only tests until it does"
        )
    )]
    pub(crate) fn impact_inputs(&self) -> InventoryImpactInputs {
        let reason = match self.progress {
            Some(CloudBackupProgressSection::InventoryIncomplete { reason }) => reason,
            _ => CloudBackupInventoryIncompleteReason::Unknown,
        };

        let Some(rows) = &self.rows else {
            return InventoryImpactInputs {
                assessment: CloudBackupInventoryAssessment::Unknown { reason },
                own: BTreeSet::new(),
                secret_missing_locally: BTreeMap::new(),
                secret_unchecked: BTreeMap::new(),
                listed: BTreeMap::new(),
            };
        };

        let mut own = BTreeSet::new();
        let mut secret_missing_locally = BTreeMap::new();
        let mut secret_unchecked = BTreeMap::new();
        for row in rows.up_to_date.iter().chain(&rows.needs_sync) {
            let wallet = &row.wallet;
            own.insert(wallet.record_id.clone());

            if row.record == CloudBackupOwnRecord::SecretMissingLocally {
                secret_missing_locally.insert(wallet.record_id.clone(), wallet.name.clone());
            }

            if rows.secret_unchecked.contains(&wallet.record_id) {
                secret_unchecked.insert(wallet.record_id.clone(), wallet.name.clone());
            }
        }

        let listed = rows
            .listed
            .iter()
            .map(|listed| {
                (listed.record_id.clone().into(), InventoryListedClass::from(&listed.record))
            })
            .collect();

        let assessment = match self.readiness {
            DetailInventory::Complete if secret_unchecked.is_empty() => {
                CloudBackupInventoryAssessment::Complete
            }
            DetailInventory::Complete | DetailInventory::Ready | DetailInventory::NotLoaded => {
                CloudBackupInventoryAssessment::Partial { reason }
            }
        };

        InventoryImpactInputs { assessment, own, secret_missing_locally, secret_unchecked, listed }
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

    let own_row = |wallet| own_row(wallet, &loaded.own_secrets);

    InventoryRows {
        up_to_date: loaded.detail.up_to_date.iter().map(own_row).collect(),
        needs_sync: loaded.detail.needs_sync.iter().map(own_row).collect(),
        listed,
        last_sync: loaded.detail.last_sync,
        secret_unchecked: loaded.own_secrets.unchecked.clone(),
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
