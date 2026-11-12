use std::collections::HashSet;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE, blocking_cloud_error,
    load_master_key_for_cloud_action,
};
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::screen::CloudBackupUnreadableKind;
use crate::manager::cloud_backup_manager::wallets::{
    CloudOnlyRestoreOutcome, DownloadedWalletBackup, WalletBackupLookup, WalletBackupReadError,
    WalletBackupReader,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupStore, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudOnlyFetch, CloudOnlyRow, RustCloudBackupManager,
    is_provider_wide_interruption,
};
use crate::wallet_identity::{LocalWalletIdentities, NamedWalletCoverage};

pub(crate) struct CloudBackupPreparedCloudWalletDelete {
    pub(crate) cloud: CloudStorageClient,
    pub(crate) namespace: String,
    pub(crate) record_id: String,
}

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
        let local_wallets =
            crate::wallet_identity::collect_local_wallet_identities().map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;

        wallet.restore_cloud_only(&local_wallets)
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
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        let prepared = self.prepare_active_namespace_wallet_restore(writes).await?;
        let result = prepared.restore_record(record_id).await;
        self.handle_cloud_only_restore_committed(&result);

        result
    }

    /// Queues the upload of a local wallet a restore just added a secret to, straight after the
    /// upgrade commits, so the wallet's own backup gains the secret even if the restore result is
    /// never applied
    pub(crate) fn handle_cloud_only_restore_committed(
        &self,
        result: &Result<CloudOnlyRestoreOutcome, CloudBackupError>,
    ) {
        if let Ok(CloudOnlyRestoreOutcome::SecretAdded { wallet_id, .. }) = result {
            self.handle_restored_secret_added(wallet_id.clone());
        }
    }

    pub(crate) async fn prepare_restore_all_cloud_wallets(
        &self,
        writes: &CloudBackupWriteClient,
        frozen_wallets: Vec<CloudBackupWalletItem>,
        listing_generation: u64,
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
            Some(self.prepare_active_namespace_wallet_restore(writes).await?)
        };

        Ok(CloudBackupPreparedRestoreAll { authoritative_fetch, ordered_queue, wallet_restore })
    }

    pub(crate) async fn prepare_active_namespace_wallet_restore(
        &self,
        writes: &CloudBackupWriteClient,
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

    pub(crate) async fn prepare_delete_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<CloudBackupPreparedCloudWalletDelete, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();

        Ok(CloudBackupPreparedCloudWalletDelete {
            cloud,
            namespace,
            record_id: record_id.to_string(),
        })
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
