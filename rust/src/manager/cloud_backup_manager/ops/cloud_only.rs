use std::collections::HashSet;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_util::ResultExt as _;
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

pub(crate) struct CloudBackupPreparedCloudWalletDelete {
    pub(crate) cloud: CloudStorageClient,
    pub(crate) namespace: String,
    pub(crate) record_id: String,
}

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
            return Ok(Vec::new());
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, &namespace, || {
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
                        "Cloud-only wallet backup uses unsupported wallet backup version {version}"
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
                    warn!("Failed to load cloud-only wallet backup: not found");
                    continue;
                }
                Err(error) => {
                    if is_connectivity_related_issue(CloudStorageIssue::from(&error)) {
                        return Err(blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error));
                    }
                    warn!("Failed to load cloud-only wallet backup: {error}");
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
                backup_updated_at: (wallet.entry.updated_at != 0)
                    .then_some(wallet.entry.updated_at),
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
        let master_key = load_master_key_for_cloud_action(&cspp, &namespace, || {
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

        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()
            .map_err_prefix("collect wallet identities", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_identities);
        let outcome = restore_session
            .restore_record(&reader, record_id)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;

        info!("Restored cloud wallet");
        Ok(outcome)
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
