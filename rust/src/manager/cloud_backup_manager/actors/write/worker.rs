use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, send};
use cove_device::cloud_storage::CloudStorageClient;
use tracing::warn;

use super::CloudBackupWriteError;
use super::supervisor::{CloudBackupWriteCommandContext, CloudBackupWriteSupervisor};

#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteCommand {
    UploadWallet { cloud: CloudStorageClient, namespace: String, record_id: String, data: Vec<u8> },
    UploadMasterKey { cloud: CloudStorageClient, namespace: String, data: Vec<u8> },
    DeleteWallet { cloud: CloudStorageClient, namespace: String, record_id: String },
    DeleteActiveWallet { cloud: CloudStorageClient, namespace: String, record_id: String },
    ListWalletCount { cloud: CloudStorageClient, namespace_id: String, fallback_count: u32 },
    ListWalletCountOptional { cloud: CloudStorageClient, namespace_id: String },
    DeleteNamespace { cloud: CloudStorageClient, namespace: String },
    None,
}

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
            Self::DeleteWallet { cloud, namespace, record_id } => {
                cloud.delete_wallet_backup(namespace, record_id).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::DeleteActiveWallet { cloud, namespace, record_id } => {
                cloud.delete_wallet_backup(namespace.clone(), record_id).await?;
                let wallet_record_ids =
                    cloud.list_wallet_backups(namespace).await.map_err(|error| {
                        CloudBackupWriteError::cloud_storage_context("list wallet backups", error)
                    })?;
                Ok(CloudBackupRemoteWriteResult::WalletRecordIds(wallet_record_ids))
            }
            Self::ListWalletCount { cloud, namespace_id, fallback_count } => {
                let wallet_count = match cloud.list_wallet_backups(namespace_id.clone()).await {
                    Ok(ids) => ids.len() as u32,
                    Err(error) => {
                        warn!(
                            "Finalize wallet uploads: failed to list wallet backups for namespace_id={namespace_id}, falling back to uploaded wallet count: {error}"
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
