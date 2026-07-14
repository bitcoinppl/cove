use cove_cspp::backup_data::wallet_record_id;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::warn;
use zeroize::Zeroizing;

use super::cloud_inventory::RemoteWalletTruth;
use super::wallets::{WalletBackupLookup, WalletBackupReader};
use super::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupDetail, CloudBackupError,
    CloudBackupStore, RustCloudBackupManager, blocking_cloud_error,
};
use crate::database::Database;

impl RustCloudBackupManager {
    pub(crate) async fn build_cloud_backup_detail_with_remote_truth(
        &self,
        wallet_record_ids: &[String],
        remote_wallet_truth: RemoteWalletTruth,
    ) -> Result<CloudBackupDetail, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let other_backups = self.other_backup_state(&cloud).await;

        Ok(super::cloud_inventory::CloudWalletInventory::load_with_remote_truth(
            wallet_record_ids,
            remote_wallet_truth,
        )
        .await?
        .build_detail(other_backups))
    }

    pub(crate) async fn load_remote_wallet_truth(
        &self,
        wallet_record_ids: &[String],
        cloud: CloudStorageClient,
    ) -> Result<RemoteWalletTruth, CloudBackupError> {
        let namespace = self.current_namespace_id()?;
        let db = Database::global();
        let local_wallets = CloudBackupStore::new(&db).all_wallets()?;
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let Some(master_key) = cspp.load_master_key_from_store().map_err(|source| {
            CloudBackupError::internal_context("load local master key", source)
        })?
        else {
            return Ok(RemoteWalletTruth {
                unknown_record_ids: wallet_record_ids.iter().cloned().collect(),
                ..RemoteWalletTruth::default()
            });
        };

        let critical_key = master_key.critical_data_key();
        let mut remote_wallet_truth = RemoteWalletTruth::default();

        let mut summaries = stream::iter(local_wallets)
            .map(|wallet| {
                let cloud = cloud.clone();
                let namespace = namespace.clone();

                async move {
                    let record_id = wallet_record_id(wallet.id.as_ref());
                    let reader =
                        WalletBackupReader::new(cloud, namespace, Zeroizing::new(critical_key));
                    let result = reader.summary(&record_id).await;
                    (record_id, result)
                }
            })
            .buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, result)) = summaries.next().await {
            match result {
                Ok(WalletBackupLookup::Found(summary)) => {
                    remote_wallet_truth.summaries_by_record_id.insert(record_id, summary);
                }
                Ok(WalletBackupLookup::NotFound) => {}
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!(
                        "Cloud backup remote truth found unsupported wallet backup version {version} for record_id={record_id}"
                    );
                    remote_wallet_truth.unsupported_record_ids.insert(record_id);
                }
                Err(error) => {
                    warn!("Cloud backup remote truth failed for record_id={record_id}: {error}");
                    remote_wallet_truth.unknown_record_ids.insert(record_id);
                }
            }
        }

        Ok(remote_wallet_truth)
    }
}

pub(crate) async fn current_namespace_wallet_record_ids(
    cloud: &CloudStorageClient,
    current_namespace: &str,
    step: BlockingCloudStep,
) -> Result<Vec<String>, CloudBackupError> {
    match cloud.list_wallet_backups(current_namespace.to_owned()).await {
        Ok(record_ids) => Ok(record_ids),
        Err(error) => Err(blocking_cloud_error(
            step,
            CloudBackupError::cloud_storage_context("list wallet backups", error),
        )),
    }
}
