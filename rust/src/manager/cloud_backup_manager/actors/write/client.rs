use act_zero::{Addr, call};
use cove_device::cloud_storage::CloudStorageClient;

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

use super::supervisor::{CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor};
use super::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};

/// Operation-aware handle for submitting writes to the write supervisor
///
/// Background callers use [`CloudBackupWriteClient::new`]. Exclusive operations
/// use [`CloudBackupWriteClient::for_operation`] so stale operation completions
/// are rejected before local state is mutated
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
        let result = receiver.await.map_err(|source| {
            CloudBackupError::internal_context("wait for cloud backup write supervisor", source)
        })?;

        let context = result.context();
        let context_id = context.id();
        if context.origin() != self.origin {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor returned mismatched operation origin for command {context_id:?}",
            )
            .into()));
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
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        self.await_result(receiver).await
    }

    pub(crate) async fn upload_wallet_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        let receiver = match self.origin {
            Some(origin) => {
                call!(self.supervisor.upload_wallet_backup_with_completion_for_operation(
                    cloud, namespace, record_id, data, completion, origin
                ))
                .await
            }
            None => {
                call!(self.supervisor.upload_wallet_backup_with_completion(
                    cloud, namespace, record_id, data, completion
                ))
                .await
            }
        }
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

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
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

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
            .map_err(|source| {
                CloudBackupError::internal_context("start cloud backup write supervisor", source)
            })?;

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
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        self.await_result(receiver).await
    }

    pub(crate) async fn complete_uploaded_wallet_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> Result<(), CloudBackupError> {
        let receiver = match self.origin {
            Some(origin) => {
                call!(self.supervisor.complete_uploaded_wallet_batch_for_operation(
                    cloud,
                    namespace_id,
                    uploaded_wallets,
                    count_refresh,
                    origin
                ))
                .await
            }
            None => {
                call!(self.supervisor.complete_uploaded_wallet_batch(
                    cloud,
                    namespace_id,
                    uploaded_wallets,
                    count_refresh
                ))
                .await
            }
        }
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        self.await_result(receiver).await
    }

    pub(crate) async fn delete_active_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup active-wallet delete requires an operation origin".into(),
            ));
        };

        let receiver = call!(
            self.supervisor
                .delete_active_wallet_backup_for_operation(cloud, namespace, record_id, origin)
        )
        .await
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        self.await_result(receiver).await
    }

    pub(crate) async fn delete_namespace(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup namespace delete requires an operation origin".into(),
            ));
        };

        let receiver =
            call!(self.supervisor.delete_namespace_for_operation(cloud, namespace, origin))
                .await
                .map_err(|source| {
                CloudBackupError::internal_context("start cloud backup write supervisor", source)
            })?;

        self.await_result(receiver).await
    }

    pub(crate) async fn apply_completion(
        &self,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = self.origin else {
            return Err(CloudBackupError::Internal(
                "cloud backup write completion requires an operation origin".into(),
            ));
        };

        let receiver = call!(self.supervisor.apply_completion_for_operation(completion, origin))
            .await
            .map_err(|source| {
                CloudBackupError::internal_context("start cloud backup write supervisor", source)
            })?;

        self.await_result(receiver).await
    }
}
