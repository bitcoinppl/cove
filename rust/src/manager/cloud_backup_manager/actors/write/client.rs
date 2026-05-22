use act_zero::{Addr, call};
use cove_device::cloud_storage::CloudStorageClient;
use cove_util::ResultExt as _;

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

use super::supervisor::{CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor};
use super::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteCompletion,
};

#[derive(Clone)]
pub(crate) struct CloudBackupWriteClient {
    supervisor: Addr<CloudBackupWriteSupervisor>,
    origin: Option<CloudBackupExclusiveOperationClaim>,
}

impl CloudBackupWriteClient {
    pub(crate) fn new(supervisor: Addr<CloudBackupWriteSupervisor>) -> Self {
        Self { supervisor, origin: None }
    }

    pub(crate) fn for_operation(
        supervisor: Addr<CloudBackupWriteSupervisor>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Self {
        Self { supervisor, origin: Some(origin) }
    }

    async fn await_result<T>(
        &self,
        receiver: CloudBackupWriteResultReceiver<T>,
    ) -> Result<T, CloudBackupError> {
        let result = receiver
            .await
            .map_err_prefix("wait for cloud backup write supervisor", CloudBackupError::Internal)?;

        let context = result.context();
        let context_id = context.id();
        if context.origin() != self.origin {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor returned mismatched operation origin for command {context_id:?}",
            )));
        }

        result.into_result()
    }

    pub(crate) async fn upload_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudBackupError> {
        let receiver = match self.origin {
            // operation-owned writes carry the active claim so stale operation results are rejected
            Some(origin) => {
                call!(
                    self.supervisor.upload_wallet_backup_for_operation(
                        cloud, namespace, record_id, data, origin
                    )
                )
                .await
            }

            // background writes are not tied to an exclusive operation claim
            None => {
                call!(self.supervisor.upload_wallet_backup(cloud, namespace, record_id, data)).await
            }
        }
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        self.await_result(receiver).await
    }

    pub(crate) async fn upload_master_key_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup master-key upload requires an operation origin".into(),
            ));
        };

        let receiver = call!(
            self.supervisor.upload_master_key_backup_for_operation(cloud, namespace, data, origin)
        )
        .await
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        self.await_result(receiver).await
    }

    pub(crate) async fn upload_master_key_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup master-key upload completion requires an operation origin".into(),
            ));
        };

        let receiver =
            call!(self.supervisor.upload_master_key_backup_with_completion_for_operation(
                cloud, namespace, data, completion, origin
            ))
            .await
            .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        self.await_result(receiver).await
    }

    pub(crate) async fn finalize_uploaded_wallets(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup wallet finalization requires an operation origin".into(),
            ));
        };

        let receiver = call!(self.supervisor.finalize_uploaded_wallets_for_operation(
            cloud,
            namespace_id,
            uploaded_wallets,
            state_mode,
            origin
        ))
        .await
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        self.await_result(receiver).await
    }
}
