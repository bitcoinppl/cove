use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use tracing::info;
use zeroize::Zeroizing;

use super::{
    RECREATE_MANIFEST_RECOVERY_MESSAGE, blocking_cloud_error, load_master_key_for_cloud_action,
};
use crate::database::Database;
use crate::database::cloud_backup::CloudBackupRecordKey;
use crate::manager::cloud_backup_manager::cloud_inventory::CloudWalletInventory;
use crate::manager::cloud_backup_manager::wallets::PreparedWalletBackup;
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupStore, RustCloudBackupManager,
};

pub(crate) enum FinalizeUploadStateMode {
    PreserveVerification,
    ResetVerification,
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
    pub(crate) async fn do_reupload_all_wallets(&self) -> Result<(), CloudBackupError> {
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
            .upload_all_wallets(&cloud, &namespace, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;

        self.finalize_uploaded_wallets(
            &cloud,
            &namespace,
            uploaded_wallets,
            FinalizeUploadStateMode::PreserveVerification,
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn finalize_uploaded_wallets(
        &self,
        cloud: &CloudStorageClient,
        namespace_id: &str,
        uploaded_wallets: Vec<PreparedWalletBackup>,
        state_mode: FinalizeUploadStateMode,
    ) -> Result<(), CloudBackupError> {
        let db = Database::global();
        let wallet_count = cloud
            .list_wallet_backups(namespace_id.to_owned())
            .await
            .map(|ids| ids.len() as u32)
            .unwrap_or(uploaded_wallets.len() as u32);
        match state_mode {
            FinalizeUploadStateMode::PreserveVerification => {
                CloudBackupStore::new(&db).persist_enabled(wallet_count)?;
            }
            FinalizeUploadStateMode::ResetVerification => {
                CloudBackupStore::new(&db).persist_enabled_reset_verification(wallet_count)?;
            }
        }

        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        for wallet in uploaded_wallets {
            self.mark_blob_uploaded_pending_confirmation(
                namespace_id,
                CloudBackupRecordKey::Wallet(wallet.metadata.id, wallet.record_id),
                wallet.revision_hash,
                uploaded_at,
            )?;
        }

        Ok(())
    }
}
