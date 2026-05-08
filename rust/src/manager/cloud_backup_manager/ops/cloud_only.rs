use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CLOUD_ONLY_FETCH_RECOVERY_MESSAGE,
    CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE, UNSUPPORTED_CLOUD_ONLY_WALLET_NAME, blocking_cloud_error,
    load_master_key_for_cloud_action,
};
use crate::database::Database;
use crate::manager::cloud_backup_manager::wallets::{
    WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupStore, CloudBackupWalletItem,
    CloudBackupWalletStatus, CloudStorageIssue, RustCloudBackupManager,
    is_connectivity_related_issue,
};

impl RustCloudBackupManager {
    pub(crate) async fn do_fetch_cloud_only_wallets(
        &self,
    ) -> Result<Vec<CloudBackupWalletItem>, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::FetchCloudOnly)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids =
            cloud.list_wallet_backups(namespace.clone()).await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::FetchCloudOnly,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                )
            })?;

        let db = Database::global();
        let local_record_ids: std::collections::HashSet<_> = CloudBackupStore::new(&db)
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
            return Ok(Vec::new());
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                CLOUD_ONLY_FETCH_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error))?;

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut items = Vec::new();
        let mut lookups = stream::iter(
            orphan_ids
                .into_iter()
                .map(|record_id| Self::lookup_wallet_backup(reader.clone(), record_id)),
        )
        .buffered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, lookup)) = lookups.next().await {
            let wallet = match lookup {
                Ok(WalletBackupLookup::Found(wallet)) => wallet,
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!(
                        "Cloud-only wallet {record_id} uses unsupported wallet backup version {version}"
                    );
                    items.push(CloudBackupWalletItem {
                        name: UNSUPPORTED_CLOUD_ONLY_WALLET_NAME.into(),
                        network: None,
                        wallet_mode: None,
                        wallet_type: None,
                        fingerprint: None,
                        label_count: None,
                        backup_updated_at: None,
                        sync_status: CloudBackupWalletStatus::UnsupportedVersion,
                        record_id: record_id.clone(),
                    });
                    continue;
                }
                Ok(WalletBackupLookup::NotFound) => {
                    warn!("Failed to load cloud-only wallet {record_id}: not found");
                    continue;
                }
                Err(error) => {
                    if is_connectivity_related_issue(CloudStorageIssue::from(&error)) {
                        return Err(blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error));
                    }
                    warn!("Failed to load cloud-only wallet {record_id}: {error}");
                    continue;
                }
            };
            let metadata = wallet.metadata;

            items.push(CloudBackupWalletItem {
                name: metadata.name,
                network: Some(metadata.network),
                wallet_mode: Some(metadata.wallet_mode),
                wallet_type: Some(metadata.wallet_type),
                fingerprint: metadata
                    .master_fingerprint
                    .as_ref()
                    .map(|fingerprint| fingerprint.as_ref().as_uppercase()),
                label_count: Some(wallet.entry.labels_count),
                backup_updated_at: Some(wallet.entry.updated_at),
                sync_status: CloudBackupWalletStatus::DeletedFromDevice,
                record_id: record_id.clone(),
            });
        }

        Ok(items)
    }

    pub(crate) async fn do_restore_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<WalletRestoreOutcome, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RestoreCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud(
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

        let db = Database::global();
        let existing_fingerprints: Vec<_> = CloudBackupStore::new(&db)
            .all_wallets()?
            .iter()
            .filter_map(|wallet| {
                wallet
                    .master_fingerprint
                    .as_ref()
                    .map(|fp| (**fp, wallet.network, wallet.wallet_mode))
            })
            .collect();
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);

        let outcome = restore_session
            .restore_record(&reader, record_id)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;
        info!("Restored cloud wallet {record_id}");
        Ok(outcome)
    }

    pub(crate) async fn do_delete_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();

        cloud.delete_wallet_backup(namespace.clone(), record_id.to_string()).await.map_err(
            |error| {
                blocking_cloud_error(
                    BlockingCloudStep::DeleteCloudWallet,
                    CloudBackupError::cloud_storage_context("delete wallet backup", error),
                )
            },
        )?;
        self.remove_blob_sync_states(std::iter::once(record_id.to_string()))
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error))?;

        let wallet_record_ids = cloud.list_wallet_backups(namespace).await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::DeleteCloudWallet,
                CloudBackupError::cloud_storage_context("list wallet backups", error),
            )
        })?;
        let wallet_count = wallet_record_ids.len() as u32;
        let db = Database::global();
        if let Ok(mut current) = db.cloud_backup_state.get() {
            current.set_wallet_count(Some(wallet_count));
            let _ = self.persist_cloud_backup_state(
                &current,
                "persist cloud backup state after deleting cloud wallet",
            );
        }

        info!("Deleted cloud wallet {record_id}");
        Ok(())
    }
}
