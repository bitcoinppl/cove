use std::collections::{HashMap, HashSet};

use cove_util::ResultExt as _;

use super::wallets::{RemoteWalletBackupSummary, all_local_wallets, prepare_wallet_backup};
use super::{CloudBackupDetail, CloudBackupError, CloudBackupWalletItem, CloudBackupWalletStatus};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobFailedState, PersistedCloudBackupStatus, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::wallet::metadata::WalletMetadata;

#[derive(Debug, Clone, Default)]
pub(crate) struct RemoteWalletTruth {
    pub(super) summaries_by_record_id: HashMap<String, RemoteWalletBackupSummary>,
    pub(super) unsupported_record_ids: HashSet<String>,
    pub(super) unknown_record_ids: HashSet<String>,
}

#[derive(Debug, Clone)]
struct LocalWalletSnapshot {
    metadata: WalletMetadata,
    record_id: String,
    revision_hash: String,
    local_label_count: u32,
}

enum WalletItemBucket {
    UpToDate,
    NeedsSync,
}

enum RemoteWalletState {
    Unknown,
    Unsupported,
    Missing,
    Matching(RemoteWalletBackupSummary),
    Stale(RemoteWalletBackupSummary),
}

pub(super) struct CloudWalletInventory {
    last_sync: Option<u64>,
    local_wallets: Vec<LocalWalletSnapshot>,
    cloud_wallet_record_ids: HashSet<String>,
    sync_states_by_record_id: HashMap<String, PersistedCloudBlobSyncState>,
    remote_wallet_truth: RemoteWalletTruth,
    strict_cloud_presence: bool,
}

impl CloudWalletInventory {
    pub(super) async fn load_with_remote_truth(
        wallet_record_ids: &[String],
        remote_wallet_truth: RemoteWalletTruth,
    ) -> Result<Self, CloudBackupError> {
        let db = Database::global();
        let local_wallets = all_local_wallet_snapshots(&db).await?;
        let last_sync = last_sync(&db);
        let sync_states_by_record_id = sync_states_by_record_id(&db)?;

        Ok(Self {
            last_sync,
            local_wallets,
            cloud_wallet_record_ids: wallet_record_ids.iter().cloned().collect(),
            sync_states_by_record_id,
            remote_wallet_truth,
            strict_cloud_presence: false,
        })
    }

    pub(super) fn cloud_wallet_count(&self) -> usize {
        self.cloud_wallet_record_ids.len()
    }

    pub(super) fn upload_candidate_wallets(&self) -> Vec<WalletMetadata> {
        if self.strict_cloud_presence {
            return self
                .local_wallets
                .iter()
                .filter(|wallet| !self.cloud_wallet_record_ids.contains(&wallet.record_id))
                .map(|wallet| wallet.metadata.clone())
                .collect();
        }

        self.local_wallets
            .iter()
            .filter(|wallet| self.is_upload_candidate_wallet(wallet))
            .map(|wallet| wallet.metadata.clone())
            .collect()
    }

    pub(super) fn build_detail(&self) -> CloudBackupDetail {
        let local_record_ids: HashSet<_> =
            self.local_wallets.iter().map(|wallet| wallet.record_id.clone()).collect();

        let mut up_to_date = Vec::new();
        let mut needs_sync = Vec::new();

        for wallet in &self.local_wallets {
            let item = self.local_wallet_item(wallet);

            match wallet_item_bucket(&item) {
                Some(WalletItemBucket::UpToDate) => up_to_date.push(item),
                Some(WalletItemBucket::NeedsSync) => needs_sync.push(item),
                None => {}
            }
        }

        let cloud_only_count = self
            .cloud_wallet_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .count() as u32;

        CloudBackupDetail { last_sync: self.last_sync, up_to_date, needs_sync, cloud_only_count }
    }

    pub(super) fn has_unknown_remote_wallets(&self) -> bool {
        self.local_wallets.iter().any(|wallet| {
            matches!(
                self.sync_status_for_wallet(wallet),
                CloudBackupWalletStatus::RemoteStateUnknown
            )
        })
    }

    fn local_wallet_item(&self, wallet: &LocalWalletSnapshot) -> CloudBackupWalletItem {
        let remote_summary = self.remote_wallet_truth.summaries_by_record_id.get(&wallet.record_id);

        CloudBackupWalletItem {
            name: wallet.metadata.name.clone(),
            network: Some(wallet.metadata.network),
            wallet_mode: Some(wallet.metadata.wallet_mode),
            wallet_type: Some(wallet.metadata.wallet_type),
            fingerprint: wallet.metadata.master_fingerprint.as_ref().map(|fp| fp.as_uppercase()),
            label_count: remote_summary
                .map(|summary| summary.label_count)
                .or(Some(wallet.local_label_count)),
            backup_updated_at: self.backup_updated_at_for_wallet(wallet),
            sync_status: self.sync_status_for_wallet(wallet),
            record_id: wallet.record_id.clone(),
        }
    }

    fn is_upload_candidate_wallet(&self, wallet: &LocalWalletSnapshot) -> bool {
        let sync_state = self.sync_states_by_record_id.get(&wallet.record_id);

        match self.remote_wallet_state(wallet) {
            RemoteWalletState::Unsupported | RemoteWalletState::Matching(_) => false,
            RemoteWalletState::Unknown => has_local_upload_candidate(sync_state),
            RemoteWalletState::Missing | RemoteWalletState::Stale(_) => !matches!(
                sync_state.map(|state| &state.state),
                Some(PersistedCloudBlobState::Uploading(_))
                    | Some(PersistedCloudBlobState::UploadedPendingConfirmation(_))
            ),
        }
    }

    fn sync_status_for_wallet(&self, wallet: &LocalWalletSnapshot) -> CloudBackupWalletStatus {
        let sync_state = self.sync_states_by_record_id.get(&wallet.record_id);
        match self.remote_wallet_state(wallet) {
            RemoteWalletState::Matching(_) => CloudBackupWalletStatus::Confirmed,
            RemoteWalletState::Unsupported => {
                sync_status_from_state(sync_state, CloudBackupWalletStatus::UnsupportedVersion)
            }
            RemoteWalletState::Unknown => sync_status_for_unknown_remote(sync_state),
            RemoteWalletState::Missing | RemoteWalletState::Stale(_) => {
                sync_status_from_state(sync_state, CloudBackupWalletStatus::Dirty)
            }
        }
    }

    fn backup_updated_at_for_wallet(&self, wallet: &LocalWalletSnapshot) -> Option<u64> {
        match self.remote_wallet_state(wallet) {
            RemoteWalletState::Matching(remote_summary)
            | RemoteWalletState::Stale(remote_summary) => {
                return Some(remote_summary.updated_at);
            }
            RemoteWalletState::Unknown
            | RemoteWalletState::Unsupported
            | RemoteWalletState::Missing => {}
        }

        let sync_state = self.sync_states_by_record_id.get(&wallet.record_id)?;

        match &sync_state.state {
            PersistedCloudBlobState::UploadedPendingConfirmation(state) => Some(state.uploaded_at),
            PersistedCloudBlobState::Confirmed(state) => Some(state.confirmed_at),
            PersistedCloudBlobState::Dirty(_)
            | PersistedCloudBlobState::Uploading(_)
            | PersistedCloudBlobState::Failed(_) => None,
        }
    }

    fn remote_wallet_state(&self, wallet: &LocalWalletSnapshot) -> RemoteWalletState {
        if self.remote_wallet_truth.unsupported_record_ids.contains(&wallet.record_id) {
            return RemoteWalletState::Unsupported;
        }

        if self.remote_wallet_truth.unknown_record_ids.contains(&wallet.record_id) {
            return RemoteWalletState::Unknown;
        }

        let Some(remote_summary) =
            self.remote_wallet_truth.summaries_by_record_id.get(&wallet.record_id)
        else {
            return RemoteWalletState::Missing;
        };

        if remote_summary.revision_hash == wallet.revision_hash {
            return RemoteWalletState::Matching(remote_summary.clone());
        }

        RemoteWalletState::Stale(remote_summary.clone())
    }
}

fn wallet_item_bucket(item: &CloudBackupWalletItem) -> Option<WalletItemBucket> {
    match item.sync_status {
        CloudBackupWalletStatus::Confirmed => Some(WalletItemBucket::UpToDate),
        CloudBackupWalletStatus::DeletedFromDevice => None,
        CloudBackupWalletStatus::Dirty
        | CloudBackupWalletStatus::Uploading
        | CloudBackupWalletStatus::UploadedPendingConfirmation
        | CloudBackupWalletStatus::Failed
        | CloudBackupWalletStatus::UnsupportedVersion
        | CloudBackupWalletStatus::RemoteStateUnknown => Some(WalletItemBucket::NeedsSync),
    }
}

async fn all_local_wallet_snapshots(
    db: &Database,
) -> Result<Vec<LocalWalletSnapshot>, CloudBackupError> {
    let mut snapshots = Vec::new();

    for wallet in all_local_wallets(db)? {
        let prepared = prepare_wallet_backup(&wallet, wallet.wallet_mode).await?;
        snapshots.push(LocalWalletSnapshot {
            metadata: wallet,
            record_id: prepared.record_id,
            revision_hash: prepared.revision_hash,
            local_label_count: prepared.entry.labels_count,
        });
    }

    Ok(snapshots)
}

fn sync_status_from_state(
    sync_state: Option<&PersistedCloudBlobSyncState>,
    fallback_status: CloudBackupWalletStatus,
) -> CloudBackupWalletStatus {
    match sync_state.map(|state| &state.state) {
        Some(PersistedCloudBlobState::Uploading(_)) => CloudBackupWalletStatus::Uploading,
        Some(PersistedCloudBlobState::UploadedPendingConfirmation(_)) => {
            CloudBackupWalletStatus::UploadedPendingConfirmation
        }
        Some(PersistedCloudBlobState::Failed(_)) => CloudBackupWalletStatus::Failed,
        Some(PersistedCloudBlobState::Dirty(_))
        | Some(PersistedCloudBlobState::Confirmed(_))
        | None => fallback_status,
    }
}

fn sync_status_for_unknown_remote(
    sync_state: Option<&PersistedCloudBlobSyncState>,
) -> CloudBackupWalletStatus {
    match sync_state.map(|state| &state.state) {
        Some(PersistedCloudBlobState::Dirty(_)) => CloudBackupWalletStatus::Dirty,
        Some(PersistedCloudBlobState::Uploading(_)) => CloudBackupWalletStatus::Uploading,
        Some(PersistedCloudBlobState::UploadedPendingConfirmation(_)) => {
            CloudBackupWalletStatus::UploadedPendingConfirmation
        }
        Some(PersistedCloudBlobState::Failed(_)) => CloudBackupWalletStatus::Failed,
        Some(PersistedCloudBlobState::Confirmed(_)) | None => {
            CloudBackupWalletStatus::RemoteStateUnknown
        }
    }
}

fn has_local_upload_candidate(sync_state: Option<&PersistedCloudBlobSyncState>) -> bool {
    match sync_state.map(|state| &state.state) {
        Some(PersistedCloudBlobState::Dirty(_)) => true,
        Some(PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable, .. })) => *retryable,
        Some(PersistedCloudBlobState::Uploading(_))
        | Some(PersistedCloudBlobState::UploadedPendingConfirmation(_))
        | Some(PersistedCloudBlobState::Confirmed(_))
        | None => false,
    }
}

fn last_sync(db: &Database) -> Option<u64> {
    let state = db.cloud_backup_state.get().ok()?;
    match state.status {
        PersistedCloudBackupStatus::Disabled => None,
        PersistedCloudBackupStatus::Enabled
        | PersistedCloudBackupStatus::Unverified
        | PersistedCloudBackupStatus::PasskeyMissing => state.last_sync,
    }
}

fn sync_states_by_record_id(
    db: &Database,
) -> Result<HashMap<String, PersistedCloudBlobSyncState>, CloudBackupError> {
    db.cloud_blob_sync_states
        .list()
        .map_err_prefix("list cloud blob sync states", CloudBackupError::Internal)
        .map(|states| {
            states
                .into_iter()
                .map(|state| (state.record_id.clone(), state))
                .collect::<HashMap<_, _>>()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{
        CloudBlobConfirmedState, CloudBlobDirtyState, CloudBlobFailedState,
        CloudBlobUploadedPendingConfirmationState, CloudBlobUploadingState, CloudUploadKind,
        PersistedCloudBlobState,
    };

    fn sync_state(state: PersistedCloudBlobState) -> PersistedCloudBlobSyncState {
        PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id: "ns-1".into(),
            wallet_id: None,
            record_id: "record-1".into(),
            state,
        }
    }

    #[test]
    fn status_is_confirmed_when_remote_revision_matches() {
        let state = sync_state(PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: "old".into(),
            confirmed_at: 10,
        }));
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-1".into(),
            local_label_count: 3,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::from([(wallet.record_id.clone(), state)]),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::from([(
                    wallet.record_id.clone(),
                    RemoteWalletBackupSummary {
                        revision_hash: "rev-1".into(),
                        label_count: 2,
                        updated_at: 50,
                    },
                )]),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::new(),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(inventory.sync_status_for_wallet(&wallet), CloudBackupWalletStatus::Confirmed);
        assert_eq!(inventory.backup_updated_at_for_wallet(&wallet), Some(50));
    }

    #[test]
    fn status_stays_uploaded_pending_confirmation_when_remote_is_stale() {
        let state = sync_state(PersistedCloudBlobState::UploadedPendingConfirmation(
            CloudBlobUploadedPendingConfirmationState {
                revision_hash: "rev-2".into(),
                uploaded_at: 20,
                attempt_count: 0,
                last_checked_at: None,
            },
        ));
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-2".into(),
            local_label_count: 1,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::from([(wallet.record_id.clone(), state)]),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::from([(
                    wallet.record_id.clone(),
                    RemoteWalletBackupSummary {
                        revision_hash: "rev-1".into(),
                        label_count: 1,
                        updated_at: 10,
                    },
                )]),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::new(),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(
            inventory.sync_status_for_wallet(&wallet),
            CloudBackupWalletStatus::UploadedPendingConfirmation
        );
    }

    #[test]
    fn status_is_unknown_when_remote_truth_failed_without_active_upload() {
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-1".into(),
            local_label_count: 0,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::new(),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::new(),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::from([wallet.record_id.clone()]),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(
            inventory.sync_status_for_wallet(&wallet),
            CloudBackupWalletStatus::RemoteStateUnknown
        );
        assert!(inventory.has_unknown_remote_wallets());
        assert!(inventory.upload_candidate_wallets().is_empty());
    }

    #[test]
    fn status_stays_dirty_when_remote_truth_failed_for_dirty_wallet() {
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-1".into(),
            local_label_count: 0,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::from([(
                wallet.record_id.clone(),
                sync_state(PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 10 })),
            )]),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::new(),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::from([wallet.record_id.clone()]),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(inventory.sync_status_for_wallet(&wallet), CloudBackupWalletStatus::Dirty);
        assert!(!inventory.has_unknown_remote_wallets());
        assert_eq!(inventory.upload_candidate_wallets(), vec![wallet.metadata.clone()]);
    }

    #[test]
    fn retryable_failed_unknown_wallet_stays_upload_candidate() {
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-1".into(),
            local_label_count: 0,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::from([(
                wallet.record_id.clone(),
                sync_state(PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: Some("rev-1".into()),
                    retryable: true,
                    error: "offline".into(),
                    failed_at: 10,
                })),
            )]),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::new(),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::from([wallet.record_id.clone()]),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(inventory.sync_status_for_wallet(&wallet), CloudBackupWalletStatus::Failed);
        assert_eq!(inventory.upload_candidate_wallets(), vec![wallet.metadata.clone()]);
    }

    #[test]
    fn strict_inventory_only_treats_listed_wallets_as_synced() {
        let wallet_a = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-a".into(),
            revision_hash: "rev-a".into(),
            local_label_count: 0,
        };
        let wallet_b = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-b".into(),
            revision_hash: "rev-b".into(),
            local_label_count: 0,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet_a.clone(), wallet_b.clone()],
            cloud_wallet_record_ids: HashSet::from([wallet_a.record_id.clone()]),
            sync_states_by_record_id: HashMap::new(),
            remote_wallet_truth: RemoteWalletTruth::default(),
            strict_cloud_presence: true,
        };

        let unsynced = inventory.upload_candidate_wallets();

        assert_eq!(unsynced.len(), 1);
        assert_eq!(unsynced[0].id, wallet_b.metadata.id);
    }

    #[test]
    fn upload_candidates_exclude_pending_states_but_detail_keeps_them_visible() {
        let uploading_wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-uploading".into(),
            revision_hash: "rev-uploading".into(),
            local_label_count: 0,
        };
        let pending_wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-pending".into(),
            revision_hash: "rev-pending".into(),
            local_label_count: 0,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![uploading_wallet.clone(), pending_wallet.clone()],
            cloud_wallet_record_ids: HashSet::new(),
            sync_states_by_record_id: HashMap::from([
                (
                    uploading_wallet.record_id.clone(),
                    PersistedCloudBlobSyncState {
                        kind: CloudUploadKind::BackupBlob,
                        namespace_id: "ns-1".into(),
                        wallet_id: None,
                        record_id: uploading_wallet.record_id.clone(),
                        state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState {
                            revision_hash: "rev-uploading".into(),
                            started_at: 10,
                        }),
                    },
                ),
                (
                    pending_wallet.record_id.clone(),
                    PersistedCloudBlobSyncState {
                        kind: CloudUploadKind::BackupBlob,
                        namespace_id: "ns-1".into(),
                        wallet_id: None,
                        record_id: pending_wallet.record_id.clone(),
                        state: PersistedCloudBlobState::UploadedPendingConfirmation(
                            CloudBlobUploadedPendingConfirmationState {
                                revision_hash: "rev-pending".into(),
                                uploaded_at: 20,
                                attempt_count: 0,
                                last_checked_at: None,
                            },
                        ),
                    },
                ),
            ]),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::from([
                    (
                        uploading_wallet.record_id.clone(),
                        RemoteWalletBackupSummary {
                            revision_hash: "stale-uploading".into(),
                            label_count: 0,
                            updated_at: 5,
                        },
                    ),
                    (
                        pending_wallet.record_id.clone(),
                        RemoteWalletBackupSummary {
                            revision_hash: "stale-pending".into(),
                            label_count: 0,
                            updated_at: 6,
                        },
                    ),
                ]),
                unsupported_record_ids: HashSet::new(),
                unknown_record_ids: HashSet::new(),
            },
            strict_cloud_presence: false,
        };

        let upload_candidates = inventory.upload_candidate_wallets();

        assert!(upload_candidates.is_empty());

        let detail = inventory.build_detail();

        assert_eq!(detail.needs_sync.len(), 2);
        assert!(detail.needs_sync.iter().any(|item| {
            item.record_id == uploading_wallet.record_id
                && item.sync_status == CloudBackupWalletStatus::Uploading
        }));
        assert!(detail.needs_sync.iter().any(|item| {
            item.record_id == pending_wallet.record_id
                && item.sync_status == CloudBackupWalletStatus::UploadedPendingConfirmation
        }));
    }

    #[test]
    fn unsupported_remote_backups_are_visible_but_not_upload_candidates() {
        let wallet = LocalWalletSnapshot {
            metadata: WalletMetadata::preview_new(),
            record_id: "record-1".into(),
            revision_hash: "rev-1".into(),
            local_label_count: 3,
        };
        let inventory = CloudWalletInventory {
            last_sync: None,
            local_wallets: vec![wallet.clone()],
            cloud_wallet_record_ids: HashSet::from([wallet.record_id.clone()]),
            sync_states_by_record_id: HashMap::new(),
            remote_wallet_truth: RemoteWalletTruth {
                summaries_by_record_id: HashMap::new(),
                unsupported_record_ids: HashSet::from([wallet.record_id.clone()]),
                unknown_record_ids: HashSet::new(),
            },
            strict_cloud_presence: false,
        };

        assert_eq!(
            inventory.sync_status_for_wallet(&wallet),
            CloudBackupWalletStatus::UnsupportedVersion
        );
        assert!(inventory.upload_candidate_wallets().is_empty());

        let detail = inventory.build_detail();

        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, wallet.record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
    }
}
