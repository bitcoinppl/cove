use std::collections::HashSet;
use std::sync::Arc;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE, blocking_cloud_error,
    load_master_key_for_cloud_action,
};
use crate::backup::import::CloudRestorePlacement;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobConfirmedState, PersistedCloudBlobState, PersistedCloudBlobSyncState,
};
use crate::manager::cloud_backup_manager::actors::{CloudBackupWriteClient, RowDeleteReport};
use crate::manager::cloud_backup_manager::delete_consent::CloudBackupDeleteCategory;
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteAuthorization, StaleTarget,
};
use crate::manager::cloud_backup_manager::owner::inventory::RecordRead;
use crate::manager::cloud_backup_manager::screen::CloudBackupUnreadableKind;
use crate::manager::cloud_backup_manager::wallets::{
    CloudOnlyRestoreOutcome, DownloadedWalletBackup, PasskeyRequestAuthority, WalletBackupLookup,
    WalletBackupReadError, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupStore, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudOnlyFetch, CloudOnlyRow, RustCloudBackupManager,
    is_provider_wide_interruption,
};
use crate::wallet_identity::{LocalWalletIdentities, NamedWalletCoverage};

pub(crate) struct CloudBackupPreparedWalletRestore {
    reader: WalletBackupReader,
}

impl CloudBackupPreparedWalletRestore {
    pub(crate) async fn restore_record(
        &self,
        record_id: &str,
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        let outcome = self
            .restore_downloaded_record(record_id)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;

        info!("Restored cloud wallet");
        Ok(outcome)
    }

    async fn restore_downloaded_record(
        &self,
        record_id: &str,
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        let wallet = self.reader.download(record_id).await?;

        // read after the download, so a local wallet deleted while it waited is not counted and
        // a wallet this run restored earlier is
        let local_wallets = crate::wallet_identity::collect_local_wallet_identities()?;

        let outcome = wallet.restore_cloud_only(&local_wallets)?;

        // the record is the restored wallet's own backup only under the backup's wallet id
        if let CloudOnlyRestoreOutcome::Restored { wallet: restored, .. } = &outcome
            && restored.placement() == CloudRestorePlacement::BackupWalletId
        {
            self.record_restored_backup(&wallet);
        }

        Ok(outcome)
    }

    /// Records the backup this restore just read and decrypted as confirmed
    ///
    /// A restored wallet is already in the cloud, so it uploads nothing and would otherwise have
    /// no row, which leaves it counted as unbacked, for example by the Wipe Data PIN check. A
    /// row another writer holds is left alone
    fn record_restored_backup(&self, wallet: &DownloadedWalletBackup) {
        let wallet_id = wallet.metadata.id.clone();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        let confirmed = PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: wallet.entry.content_revision_hash.clone(),
            confirmed_at: cove_util::time::unix_timestamp_secs_or_zero(),
        });
        let row = PersistedCloudBlobSyncState::wallet(
            self.reader.namespace().to_string(),
            wallet_id,
            record_id,
            confirmed,
        );

        if let Err(error) = Database::global().cloud_blob_sync_states.set_if_absent(&row) {
            warn!("Failed to record the restored wallet's backup as confirmed: {error}");
        }
    }
}

pub(crate) struct CloudBackupPreparedRestoreAll {
    authoritative_fetch: CloudOnlyFetch,
    ordered_queue: Vec<CloudBackupWalletItem>,
    wallet_restore: Option<CloudBackupPreparedWalletRestore>,
}

impl CloudBackupPreparedRestoreAll {
    pub(crate) fn authoritative_fetch(&self) -> &CloudOnlyFetch {
        &self.authoritative_fetch
    }

    pub(crate) fn ordered_queue(&self) -> &[CloudBackupWalletItem] {
        &self.ordered_queue
    }

    pub(crate) async fn restore_record(
        &self,
        record_id: &str,
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        if !self.ordered_queue.iter().any(|wallet| wallet.record_id == record_id) {
            return Err(CloudBackupError::Internal(
                "restore all record is not in the prepared queue".into(),
            ));
        }

        let Some(wallet_restore) = &self.wallet_restore else {
            return Err(CloudBackupError::Internal(
                "restore all queue is missing its wallet restore session".into(),
            ));
        };

        wallet_restore.restore_record(record_id).await
    }
}

impl RustCloudBackupManager {
    pub(crate) async fn do_fetch_cloud_only_wallets(
        &self,
        listing_generation: u64,
    ) -> Result<CloudOnlyFetch, CloudBackupError> {
        self.do_fetch_cloud_only_wallets_with_client(
            CloudStorage::global_explicit_client(),
            listing_generation,
        )
        .await
    }

    /// Reads every listed cloud-only record, reporting the ones it could not read
    ///
    /// `listing_generation` is the listing generation the caller observed when it started, so the
    /// result can be rejected if the listing changed while the downloads were in flight
    pub(crate) async fn do_fetch_cloud_only_wallets_with_client(
        &self,
        cloud: CloudStorageClient,
        listing_generation: u64,
    ) -> Result<CloudOnlyFetch, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::FetchCloudOnly)?;
        let namespace = self.current_namespace_id()?;
        let wallet_record_ids =
            cloud.list_wallet_backups(namespace.clone()).await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::FetchCloudOnly,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                )
            })?;

        let db = Database::global();
        let local_record_ids: HashSet<_> = CloudBackupStore::new(&db)
            .all_wallets()?
            .iter()
            .map(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()))
            .collect();

        let orphan_ids: Vec<_> = wallet_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .cloned()
            .collect();

        if orphan_ids.is_empty() {
            return Ok(CloudOnlyFetch {
                rows: Vec::new(),
                listing_generation,
                namespace_id: namespace,
            });
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let local_master_key = cspp
            .load_master_key_from_store()
            .map_err(|source| CloudBackupError::internal_context("load local master key", source))?
            .filter(|master_key| master_key.namespace_id() == namespace);

        // loading never asks for the passkey, so without this namespace's key on the device the
        // records stay unchecked until a verification the user starts recovers it
        let Some(master_key) = local_master_key else {
            info!(
                "No local master key for the active namespace, listing cloud-only records unchecked"
            );
            let rows = orphan_ids.into_iter().map(CloudOnlyRow::unchecked).collect();

            return Ok(CloudOnlyFetch { rows, listing_generation, namespace_id: namespace });
        };

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );

        // collected once per fetch, so a record whose recovery material a local wallet already
        // holds under another record ID shows as an extra copy instead of restorable
        let local_wallets =
            crate::wallet_identity::collect_local_wallet_identities().map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;

        let mut rows = Vec::with_capacity(orphan_ids.len());
        let mut reads = stream::iter(
            orphan_ids.into_iter().map(|record_id| read_listed_record(reader.clone(), record_id)),
        )
        .buffered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, read)) = reads.next().await {
            let wallet = match read {
                Ok(WalletBackupLookup::Found(wallet)) => wallet,
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!(
                        "Cloud-only wallet backup uses unsupported wallet backup version {version}"
                    );

                    rows.push(CloudOnlyRow::unsupported(record_id));
                    continue;
                }

                // listed but gone when downloaded, so the provider cannot serve it right now
                Ok(WalletBackupLookup::NotFound) => {
                    warn!("Failed to load cloud-only wallet backup: not found");
                    rows.push(CloudOnlyRow::unreadable(
                        record_id,
                        CloudBackupUnreadableKind::NotDownloadable,
                    ));
                    continue;
                }
                Err(error) => {
                    if error.download_error().is_some_and(is_provider_wide_interruption) {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::FetchCloudOnly,
                            error.into(),
                        ));
                    }

                    let kind = CloudBackupUnreadableKind::from(&error);
                    warn!(
                        "Failed to load cloud-only wallet backup: {}",
                        CloudBackupError::from(error)
                    );
                    rows.push(CloudOnlyRow::unreadable(record_id, kind));
                    continue;
                }
            };

            rows.push(listed_wallet_row(wallet, record_id, &local_wallets));
        }

        Ok(CloudOnlyFetch { rows, listing_generation, namespace_id: namespace })
    }

    pub(crate) async fn do_restore_cloud_wallet(
        &self,
        writes: &CloudBackupWriteClient,
        record_id: &str,
        authority: PasskeyRequestAuthority,
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        let prepared = self.prepare_active_namespace_wallet_restore(writes, authority).await?;
        let result = prepared.restore_record(record_id).await;
        self.handle_cloud_only_restore_committed(&result);

        result
    }

    /// Queues the upload of a local wallet whose own backup a restore just made stale or left
    /// missing, straight after the restore commits, so it is uploaded even if the restore result
    /// is never applied
    ///
    /// That is a wallet a restore added a secret to, and a wallet restored under a new wallet id,
    /// which the backup's record does not describe
    pub(crate) fn handle_cloud_only_restore_committed(
        &self,
        result: &Result<CloudOnlyRestoreOutcome, CloudBackupError>,
    ) {
        match result {
            Ok(CloudOnlyRestoreOutcome::SecretAdded { wallet, .. }) => {
                self.handle_restored_wallet_needs_backup(wallet.id.clone());
            }
            Ok(CloudOnlyRestoreOutcome::Restored { wallet, .. })
                if wallet.placement() == CloudRestorePlacement::NewWalletId =>
            {
                self.handle_restored_wallet_needs_backup(wallet.metadata().id.clone());
            }
            _ => {}
        }
    }

    /// Restores one downloaded backup in a batch's `session`, queueing the upload of the wallet it
    /// restored, or added a secret to, as soon as that commits
    ///
    /// The active backup holds none of that until the wallet uploads, so a batch that is
    /// cancelled, returns early or fails its own upload still leaves it queued, and a retry that
    /// now finds the wallet covered doesn't leave the device without a backup of it
    pub(crate) fn restore_in_session(
        &self,
        session: &mut WalletRestoreSession,
        wallet: &DownloadedWalletBackup,
    ) -> Result<WalletRestoreOutcome, CloudBackupError> {
        let result = session.restore_downloaded(wallet);
        match &result {
            Ok(WalletRestoreOutcome::SecretAdded { wallet }) => {
                self.handle_restored_wallet_needs_backup(wallet.id.clone());
            }

            // the batch may read the record from another namespace, or under an id the record
            // doesn't describe, so nothing else uploads this wallet if the run's own upload fails
            Ok(WalletRestoreOutcome::Restored { wallet, .. }) => {
                self.handle_restored_wallet_needs_backup(wallet.metadata().id.clone());
            }
            Ok(WalletRestoreOutcome::SkippedDuplicate) | Err(_) => {}
        }

        result
    }

    /// Uses `authority` only when the local master key is missing
    pub(crate) async fn prepare_restore_all_cloud_wallets(
        &self,
        writes: &CloudBackupWriteClient,
        frozen_wallets: Vec<CloudBackupWalletItem>,
        listing_generation: u64,
        authority: PasskeyRequestAuthority,
    ) -> Result<CloudBackupPreparedRestoreAll, CloudBackupError> {
        let authoritative_fetch = self.do_fetch_cloud_only_wallets(listing_generation).await?;
        let eligible_record_ids: HashSet<_> = authoritative_fetch
            .rows
            .iter()
            .filter_map(CloudOnlyRow::restorable_wallet)
            .map(|wallet| wallet.record_id.as_str())
            .collect();
        let ordered_queue = frozen_wallets
            .into_iter()
            .filter(|wallet| {
                wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice
                    && eligible_record_ids.contains(wallet.record_id.as_str())
            })
            .collect::<Vec<_>>();

        let wallet_restore = if ordered_queue.is_empty() {
            None
        } else {
            Some(self.prepare_active_namespace_wallet_restore(writes, authority).await?)
        };

        Ok(CloudBackupPreparedRestoreAll { authoritative_fetch, ordered_queue, wallet_restore })
    }

    /// Uses `authority` only when the local master key is missing and must be recovered
    pub(crate) async fn prepare_active_namespace_wallet_restore(
        &self,
        writes: &CloudBackupWriteClient,
        authority: PasskeyRequestAuthority,
    ) -> Result<CloudBackupPreparedWalletRestore, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RestoreCloudWallet)?;

        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, &namespace, || {
            self.recover_local_master_key_from_cloud(
                writes,
                &namespace,
                CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE,
                authority,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );

        Ok(CloudBackupPreparedWalletRestore { reader })
    }

    /// Deletes one listed record the user confirmed, through the write lane
    ///
    /// The lane checks the authorization again just before the remote call, so a row that
    /// changed while this waited reports a stale target instead of deleting. The lane reads an
    /// unreadable row's record again with the verified key first, and keeps one that no longer
    /// fails to decrypt
    pub(crate) async fn delete_authorized_cloud_wallet(
        &self,
        writes: CloudBackupWriteClient,
        authorization: Arc<DeleteAuthorization>,
        record_id: String,
    ) -> Result<RowDeleteReport, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteCloudWallet)?;

        let cloud = CloudStorage::global_explicit_client();
        let report = match authorization.category() {
            CloudBackupDeleteCategory::Unreadable { .. } => {
                delete_unreadable_row(&writes, cloud, authorization, record_id).await
            }
            _ => writes.delete_authorized_record(cloud, authorization, record_id).await,
        };

        let report = report.map_err(|error| {
            let error = match error {
                CloudBackupError::CloudStorage(source) => {
                    CloudBackupError::cloud_storage_context("delete wallet backup", source)
                }
                error => error,
            };

            blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error)
        })?;

        if let RowDeleteReport::Removed { .. } = report {
            info!("Deleted cloud wallet");
        }

        Ok(report)
    }
}

/// Deletes an unreadable row the way the unreadable batch deletes its records
async fn delete_unreadable_row(
    writes: &CloudBackupWriteClient,
    cloud: CloudStorageClient,
    authorization: Arc<DeleteAuthorization>,
    record_id: String,
) -> Result<RowDeleteReport, CloudBackupError> {
    let reader = verified_reader(cloud.clone(), authorization.namespace())?;
    let deleted = writes.delete_unreadable_record(cloud, reader, authorization, record_id).await?;

    // the lane's read kept the record, so the category the user agreed to no longer holds; the
    // inventory owner already has the read
    Ok(deleted.unwrap_or(RowDeleteReport::StaleTarget(StaleTarget::CategoryChanged)))
}

/// A reader of `namespace` with this device's key, the key an unreadable row's Delete trusts
///
/// A row delete holds no passkey permit, so it never recovers a missing key; without the key
/// nothing is read and nothing is deleted
fn verified_reader(
    cloud: CloudStorageClient,
    namespace: &str,
) -> Result<WalletBackupReader, CloudBackupError> {
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .map_err(|source| CloudBackupError::internal_context("load local master key", source))?
        .filter(|master_key| master_key.namespace_id() == namespace)
        .ok_or_else(|| CloudBackupError::RecoveryRequired(UNREADABLE_ROW_KEY_MESSAGE.into()))?;

    Ok(WalletBackupReader::new(
        cloud,
        namespace.to_owned(),
        Zeroizing::new(master_key.critical_data_key()),
    ))
}

const UNREADABLE_ROW_KEY_MESSAGE: &str =
    "Cloud backup needs verification before this wallet backup can be deleted";

/// What `record_id` reads as with `reader`, which holds the key the configuration verified, for
/// the inventory owner; a readable record is classified the way a fetch classifies it
///
/// A provider-wide interruption says nothing about the record, so it is the error that stops the
/// caller instead of a read
pub(crate) async fn read_record(
    reader: &WalletBackupReader,
    record_id: &str,
    local_wallets: &LocalWalletIdentities,
) -> Result<RecordRead, CloudStorageError> {
    match reader.read(record_id).await {
        Ok(WalletBackupLookup::Found(wallet)) => {
            let row = listed_wallet_row(wallet, record_id.to_owned(), local_wallets);
            Ok(RecordRead::Readable(Box::new(row)))
        }
        Ok(WalletBackupLookup::NotFound) => Ok(RecordRead::Absent),
        Ok(WalletBackupLookup::UnsupportedVersion(_)) => Ok(RecordRead::Unsupported),
        Err(WalletBackupReadError::Decrypt(_)) => Ok(RecordRead::Undecryptable),
        Err(WalletBackupReadError::Download(source)) if is_provider_wide_interruption(&source) => {
            Err(source)
        }

        // a parse failure may come before decryption, so it says nothing about the key
        Err(error) => {
            let kind = CloudBackupUnreadableKind::from(&error);
            warn!("Read an unreadable wallet backup: {}", CloudBackupError::from(error));
            Ok(RecordRead::Unreadable(kind))
        }
    }
}

async fn read_listed_record(
    reader: WalletBackupReader,
    record_id: String,
) -> (String, Result<WalletBackupLookup<DownloadedWalletBackup>, WalletBackupReadError>) {
    let read = reader.read(&record_id).await;
    (record_id, read)
}

/// The row of a read record, an extra copy when a local wallet holds all it can recover
///
/// A secret backup that a local public wallet only matches stays restorable, since restoring it
/// adds the secret to that wallet and deleting it could lose the secret
fn listed_wallet_row(
    wallet: DownloadedWalletBackup,
    record_id: String,
    local_wallets: &LocalWalletIdentities,
) -> CloudOnlyRow {
    let coverage = wallet.local_coverage(local_wallets);
    let metadata = wallet.metadata;
    let item = CloudBackupWalletItem {
        name: metadata.name,
        network: Some(metadata.network),
        wallet_mode: Some(metadata.wallet_mode),
        wallet_type: Some(metadata.wallet_type),
        fingerprint: metadata
            .master_fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint.as_ref().as_uppercase()),
        label_count: Some(wallet.entry.labels_count),
        backup_updated_at: (wallet.entry.updated_at != 0).then_some(wallet.entry.updated_at),
        sync_status: CloudBackupWalletStatus::DeletedFromDevice,
        restore_failure: None,
        record_id,
    };

    match coverage {
        NamedWalletCoverage::Covered(covered_by) => CloudOnlyRow::extra_copy(item, covered_by),
        NamedWalletCoverage::SecretMissing | NamedWalletCoverage::Absent => {
            CloudOnlyRow::not_on_device(item)
        }
    }
}

impl From<&WalletBackupReadError> for CloudBackupUnreadableKind {
    fn from(error: &WalletBackupReadError) -> Self {
        match error {
            WalletBackupReadError::Download(
                CloudStorageError::DownloadFailed(_) | CloudStorageError::NotFound(_),
            ) => Self::NotDownloadable,
            WalletBackupReadError::Download(_) => Self::Provider,
            WalletBackupReadError::Parse(_) => Self::Parse,
            WalletBackupReadError::Decrypt(_) => Self::Decrypt,
        }
    }
}
