use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use tracing::info;
use zeroize::Zeroizing;

use super::{
    RECREATE_MANIFEST_RECOVERY_MESSAGE, blocking_cloud_error, load_master_key_for_cloud_action,
};
use crate::manager::cloud_backup_manager::cloud_inventory::CloudWalletInventory;
use crate::manager::cloud_backup_manager::workers::{
    CloudBackupUploadedWallet, CloudBackupWriteClient,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupStore, RustCloudBackupManager,
};

#[derive(Debug)]
pub(crate) struct CloudBackupReuploadedWallets {
    pub(crate) namespace_id: String,
    pub(crate) uploaded_wallets: Vec<CloudBackupUploadedWallet>,
}

impl RustCloudBackupManager {
    pub(crate) async fn do_sync_unsynced_wallets(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Sync)?;
        let namespace = self.current_namespace_id()?;
        info!("Sync: listing cloud wallet backups for namespace {namespace}");
        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids = cloud.list_wallet_backups(namespace).await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::Sync,
                CloudBackupError::cloud_storage_context("list wallet backups", error),
            )
        })?;
        let remote_wallet_truth =
            self.load_remote_wallet_truth(&wallet_record_ids, cloud.clone()).await?;
        let inventory =
            CloudWalletInventory::load_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
                .await?;

        info!("Sync: found {} wallet(s) in cloud", inventory.cloud_wallet_count());
        let unsynced = inventory.upload_candidate_wallets();

        if unsynced.is_empty() {
            info!("Sync: all wallets already synced");
            return Ok(());
        }

        info!("Sync: {} wallet(s) need backup", unsynced.len());
        self.do_backup_wallets(&unsynced)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Sync, error))
    }

    /// Re-upload all local wallets to cloud
    ///
    /// Reuses the master key from keychain (no passkey interaction needed)
    pub(crate) async fn prepare_reupload_all_wallets(
        &self,
        writes: CloudBackupWriteClient,
    ) -> Result<CloudBackupReuploadedWallets, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RecreateManifest)?;
        info!("Re-uploading all wallets to cloud");

        let namespace = self.current_namespace_id()?;
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                RECREATE_MANIFEST_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let cloud = CloudStorage::global_explicit_client();
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&writes, &cloud, &namespace, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;
        let uploaded_wallets = uploaded_wallets
            .into_iter()
            .map(|wallet| {
                CloudBackupUploadedWallet::new(
                    wallet.metadata.id,
                    wallet.record_id,
                    wallet.revision_hash,
                )
            })
            .collect();

        Ok(CloudBackupReuploadedWallets { namespace_id: namespace, uploaded_wallets })
    }
}
