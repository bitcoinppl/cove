//! Leaf actor for remote Cloud Backup write commands
//!
//! This actor performs cloud-storage calls and returns raw remote results to
//! the write supervisor. It does not mutate manager state directly

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, send};
use cove_device::cloud_storage::CloudStorageClient;
use tracing::warn;

use super::CloudBackupWriteError;
use super::supervisor::{CloudBackupWriteCommandContext, CloudBackupWriteSupervisor};

/// Remote storage operation submitted through the serialized write lane
#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteCommand {
    UploadWallet { cloud: CloudStorageClient, namespace: String, record_id: String, data: Vec<u8> },
    UploadMasterKey { cloud: CloudStorageClient, namespace: String, data: Vec<u8> },
    DeleteActiveWallet { cloud: CloudStorageClient, namespace: String, record_id: String },
    ListWalletCount { cloud: CloudStorageClient, namespace_id: String, fallback_count: u32 },
    ListWalletCountOptional { cloud: CloudStorageClient, namespace_id: String },
    DeleteNamespace { cloud: CloudStorageClient, namespace: String },
    None,
}

/// Remote result shape expected by the paired local completion
#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteResult {
    None,
    WalletRecordIds(Vec<String>),
    WalletCount(u32),
    ListedWalletCount(Option<u32>),
}

impl CloudBackupRemoteWriteCommand {
    async fn execute(self) -> Result<CloudBackupRemoteWriteResult, CloudBackupWriteError> {
        match self {
            Self::UploadWallet { cloud, namespace, record_id, data } => {
                cloud.upload_wallet_backup(namespace, record_id, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::UploadMasterKey { cloud, namespace, data } => {
                cloud.upload_master_key_backup(namespace, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::DeleteActiveWallet { cloud, namespace, record_id } => {
                cloud.delete_wallet_backup(namespace.clone(), record_id).await?;
                let wallet_record_ids = match cloud.list_wallet_backups(namespace).await {
                    Ok(wallet_record_ids) => wallet_record_ids,
                    Err(error) => {
                        return Err(CloudBackupWriteError::cloud_storage_context(
                            "list wallet backups",
                            error,
                        ));
                    }
                };
                Ok(CloudBackupRemoteWriteResult::WalletRecordIds(wallet_record_ids))
            }
            Self::ListWalletCount { cloud, namespace_id, fallback_count } => {
                let wallet_count = match cloud.list_wallet_backups(namespace_id.clone()).await {
                    Ok(ids) => ids.len() as u32,
                    Err(error) => {
                        warn!(
                            "Finalize wallet uploads: failed to list wallet backups, falling back to uploaded wallet count: {error}"
                        );
                        fallback_count
                    }
                };
                Ok(CloudBackupRemoteWriteResult::WalletCount(wallet_count))
            }
            Self::ListWalletCountOptional { cloud, namespace_id } => {
                let listed_wallet_count = cloud
                    .list_wallet_backups(namespace_id)
                    .await
                    .ok()
                    .map(|record_ids| record_ids.len() as u32);
                Ok(CloudBackupRemoteWriteResult::ListedWalletCount(listed_wallet_count))
            }
            Self::DeleteNamespace { cloud, namespace } => {
                cloud.delete_namespace(namespace).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::None => Ok(CloudBackupRemoteWriteResult::None),
        }
    }
}

/// Actor that executes one remote write command and reports back to its parent
#[derive(Debug, Default)]
pub(crate) struct CloudBackupWriteWorker {
    parent: WeakAddr<CloudBackupWriteSupervisor>,
}

#[async_trait::async_trait]
impl Actor for CloudBackupWriteWorker {
    async fn started(&mut self, _addr: Addr<Self>) -> ActorResult<()> {
        Produces::ok(())
    }
}

impl CloudBackupWriteWorker {
    pub(crate) async fn execute(
        &mut self,
        parent: WeakAddr<CloudBackupWriteSupervisor>,
        context: CloudBackupWriteCommandContext,
        remote: CloudBackupRemoteWriteCommand,
    ) -> ActorResult<()> {
        self.parent = parent;
        let result = remote.execute().await;
        send!(self.parent.complete_remote_write(context, result));
        Produces::ok(())
    }
}

#[cfg(test)]
mod tests {
    use cove_device::cloud_storage::{CloudStorage, CloudStorageError};

    use super::*;
    use crate::manager::cloud_backup_manager::ops::test_support::{async_test_lock, test_globals};

    #[tokio::test(flavor = "current_thread")]
    async fn delete_active_wallet_fails_closed_when_listing_is_missing() {
        let _guard = async_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        let globals = test_globals();
        globals.cloud.reset();

        let namespace = "0123456789abcdef0123456789abcdef".to_owned();
        let record_id = "wallet".to_owned();
        globals.cloud.set_wallet_backup(namespace.clone(), record_id.clone(), vec![1, 2, 3]);
        globals.cloud.fail_list_wallet_files_for_namespace(
            namespace.clone(),
            CloudStorageError::NotFound("wallet files missing".into()),
        );

        let result = CloudBackupRemoteWriteCommand::DeleteActiveWallet {
            cloud: CloudStorage::global_explicit_client(),
            namespace,
            record_id,
        }
        .execute()
        .await
        .unwrap_err();

        assert!(result.to_string().contains("wallet files missing"), "{result}");
    }
}
