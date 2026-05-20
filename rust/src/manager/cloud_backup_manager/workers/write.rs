use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, Produces, call};
use cove_device::cloud_storage::CloudStorageClient;
use cove_util::ResultExt as _;
use tokio::sync::{RwLock, RwLockReadGuard, oneshot};
use tokio::task;
use tracing::{error, warn};

use super::super::CloudBackupError;
use super::super::model::CloudBackupExclusiveOperationClaim;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, PersistedCloudBackupState, PersistedCloudBlobSyncState,
};
use crate::manager::cloud_backup_manager::{CloudBackupStore, RustCloudBackupManager};
use crate::wallet::metadata::WalletId;

pub(crate) type CloudBackupWriteResultReceiver<T> =
    oneshot::Receiver<CloudBackupWriteCommandResult<T>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupWriteCommandId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupWriteCommandContext {
    id: CloudBackupWriteCommandId,
    origin: Option<CloudBackupExclusiveOperationClaim>,
}

impl CloudBackupWriteCommandContext {
    fn new(id: u64, origin: Option<CloudBackupExclusiveOperationClaim>) -> Self {
        Self { id: CloudBackupWriteCommandId(id), origin }
    }

    pub(crate) fn id(self) -> CloudBackupWriteCommandId {
        self.id
    }

    pub(crate) fn origin(self) -> Option<CloudBackupExclusiveOperationClaim> {
        self.origin
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupWriteCommandResult<T> {
    context: CloudBackupWriteCommandContext,
    result: Result<T, CloudBackupError>,
}

impl<T> CloudBackupWriteCommandResult<T> {
    fn new(context: CloudBackupWriteCommandContext, result: Result<T, CloudBackupError>) -> Self {
        Self { context, result }
    }

    pub(crate) fn context(&self) -> CloudBackupWriteCommandContext {
        self.context
    }

    pub(crate) fn into_result(self) -> Result<T, CloudBackupError> {
        self.result
    }
}

#[derive(Clone)]
pub(crate) struct CloudBackupWriteClient {
    write: Addr<CloudBackupWriteWorker>,
    origin: Option<CloudBackupExclusiveOperationClaim>,
}

impl CloudBackupWriteClient {
    pub(crate) fn new(write: Addr<CloudBackupWriteWorker>) -> Self {
        Self { write, origin: None }
    }

    pub(crate) fn for_operation(
        write: Addr<CloudBackupWriteWorker>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Self {
        Self { write, origin: Some(origin) }
    }

    async fn await_result<T>(
        &self,
        receiver: CloudBackupWriteResultReceiver<T>,
    ) -> Result<T, CloudBackupError> {
        let result = receiver
            .await
            .map_err_prefix("wait for cloud backup write worker", CloudBackupError::Internal)?;
        let context = result.context();
        if context.origin() != self.origin {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write worker returned mismatched operation origin for command {:?}",
                context.id()
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
        let receiver = if let Some(origin) = self.origin {
            call!(
                self.write
                    .upload_wallet_backup_for_operation(cloud, namespace, record_id, data, origin)
            )
            .await
        } else {
            call!(self.write.upload_wallet_backup(cloud, namespace, record_id, data)).await
        }
        .map_err_prefix("start cloud backup write worker", CloudBackupError::Internal)?;

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
            self.write.upload_master_key_backup_for_operation(cloud, namespace, data, origin)
        )
        .await
        .map_err_prefix("start cloud backup write worker", CloudBackupError::Internal)?;

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

        let receiver = call!(self.write.upload_master_key_backup_with_completion_for_operation(
            cloud, namespace, data, completion, origin
        ))
        .await
        .map_err_prefix("start cloud backup write worker", CloudBackupError::Internal)?;

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

        let receiver = call!(self.write.finalize_uploaded_wallets_for_operation(
            cloud,
            namespace_id,
            uploaded_wallets,
            state_mode,
            origin
        ))
        .await
        .map_err_prefix("start cloud backup write worker", CloudBackupError::Internal)?;

        self.await_result(receiver).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupWriteBlocker {
    Disabling { operation_id: u64 },
}

#[derive(Debug, Default)]
struct CloudBackupWriteWorkerState {
    active_blocker: RwLock<Option<CloudBackupWriteBlocker>>,
    write_lock: tokio::sync::Mutex<()>,
    next_command_id: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct CloudBackupWriteWorker {
    manager: Weak<RustCloudBackupManager>,
    state: Arc<CloudBackupWriteWorkerState>,
}

#[async_trait::async_trait]
impl Actor for CloudBackupWriteWorker {
    async fn started(&mut self, _addr: act_zero::Addr<Self>) -> ActorResult<()> {
        Produces::ok(())
    }
}

impl CloudBackupWriteWorker {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self { manager, state: Arc::default() }
    }

    pub(crate) async fn block(&mut self, blocker: CloudBackupWriteBlocker) -> ActorResult<()> {
        *self.state.active_blocker.write().await = Some(blocker);
        Produces::ok(())
    }

    pub(crate) async fn unblock(&mut self, blocker: CloudBackupWriteBlocker) -> ActorResult<()> {
        let mut active_blocker = self.state.active_blocker.write().await;
        if *active_blocker == Some(blocker) {
            *active_blocker = None;
        }
        Produces::ok(())
    }

    async fn run_allowed_write<T>(
        state: Arc<CloudBackupWriteWorkerState>,
        operation: impl Future<Output = Result<T, CloudBackupError>>,
        writes_blocked_by_persisted_state: impl Fn() -> bool,
    ) -> Result<T, CloudBackupError> {
        let _guard = state.write_lock.lock().await;
        ensure_writes_allowed(&state, writes_blocked_by_persisted_state()).await?;
        operation.await
    }

    async fn run_exclusive_write<T>(
        state: Arc<CloudBackupWriteWorkerState>,
        operation: impl Future<Output = T>,
    ) -> T {
        let _guard = state.write_lock.lock().await;
        operation.await
    }

    fn persisted_state_blocks_writes() -> bool {
        Database::global()
            .cloud_backup_state
            .get()
            .unwrap_or_else(|error| {
                error!("Failed to load cloud backup state for write worker: {error}");
                PersistedCloudBackupState::default()
            })
            .is_disabling()
    }

    fn next_command(
        &self,
        origin: Option<CloudBackupExclusiveOperationClaim>,
    ) -> CloudBackupWriteCommandContext {
        CloudBackupWriteCommandContext::new(
            self.state.next_command_id.fetch_add(1, Ordering::Relaxed),
            origin,
        )
    }

    async fn ensure_operation_origin_current(
        manager: &RustCloudBackupManager,
        command: CloudBackupWriteCommandContext,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = command.origin() else { return Ok(()) };

        call!(manager.supervisor.ensure_exclusive_operation_current(origin))
            .await
            .map_err_prefix("check cloud backup operation freshness", CloudBackupError::Internal)?
    }

    fn spawn_allowed_write<T>(
        &self,
        command: CloudBackupWriteCommandContext,
        operation: impl Future<Output = Result<T, CloudBackupError>> + Send + 'static,
    ) -> CloudBackupWriteResultReceiver<T>
    where
        T: Send + 'static,
    {
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result =
                Self::run_allowed_write(state, operation, Self::persisted_state_blocks_writes)
                    .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_wallet_upload_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;
                cloud.upload_wallet_backup(namespace, record_id, data).await?;
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                Self::ensure_operation_origin_current(&manager, command).await?;
                completion.apply(&manager).await
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_master_key_upload_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;
                cloud.upload_master_key_backup(namespace, data).await?;
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                Self::ensure_operation_origin_current(&manager, command).await?;
                completion.apply(&manager).await
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_active_wallet_delete(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;
                cloud.delete_wallet_backup(namespace.clone(), record_id.clone()).await?;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let wallet_record_ids = cloud.list_wallet_backups(namespace).await.map_err(|error| {
                    CloudBackupError::cloud_storage_context("list wallet backups", error)
                })?;
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                Self::ensure_operation_origin_current(&manager, command).await?;
                manager.remove_blob_sync_states(std::iter::once(record_id.clone()))?;

                let wallet_count = wallet_record_ids.len() as u32;
                match Database::global().cloud_backup_state.get() {
                    Ok(mut current) => {
                        current.set_wallet_count(Some(wallet_count));
                        if let Err(error) = manager.persist_cloud_backup_state(
                            &current,
                            "persist cloud backup state after deleting cloud wallet",
                        ) {
                            warn!(
                                "Failed to persist cloud backup state after deleting cloud wallet: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        warn!(
                            "Failed to load cloud backup state after deleting cloud wallet, skipping wallet count update: {error}"
                        );
                    }
                }

                Ok(())
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_finalize_uploaded_wallets(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let wallet_count = match cloud.list_wallet_backups(namespace_id.clone()).await {
                    Ok(ids) => ids.len() as u32,
                    Err(error) => {
                        warn!(
                            "Finalize wallet uploads: failed to list wallet backups for namespace_id={namespace_id}, falling back to uploaded wallet count: {error}"
                        );
                        uploaded_wallets.len() as u32
                    }
                };
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                Self::ensure_operation_origin_current(&manager, command).await?;

                match state_mode {
                    CloudBackupUploadedWalletsStateMode::PreserveVerification => {
                        CloudBackupStore::global().persist_enabled(wallet_count)?;
                    }
                    CloudBackupUploadedWalletsStateMode::ResetVerification => {
                        CloudBackupStore::global().persist_enabled_reset_verification(wallet_count)?;
                    }
                }

                let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
                for wallet in uploaded_wallets {
                    manager
                        .mark_wallet_uploaded_pending_confirmation_if_revision_current(
                            &namespace_id,
                            wallet.wallet_id,
                            wallet.record_id,
                            wallet.revision_hash,
                            uploaded_at,
                        )
                        .await?;
                }

                Ok(())
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_complete_uploaded_wallet_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                ensure_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let listed_wallet_count = cloud
                    .list_wallet_backups(namespace_id.clone())
                    .await
                    .ok()
                    .map(|record_ids| record_ids.len() as u32);
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let wallet_count = count_refresh.wallet_count(listed_wallet_count);
                CloudBackupStore::global().persist_enabled(wallet_count)?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
                for wallet in uploaded_wallets {
                    manager
                        .mark_wallet_uploaded_pending_confirmation_if_revision_current(
                            &namespace_id,
                            wallet.wallet_id,
                            wallet.record_id,
                            wallet.revision_hash,
                            uploaded_at,
                        )
                        .await?;
                }

                Ok(())
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_apply_completion(
        &self,
        completion: CloudBackupWriteCompletion,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let manager = self.manager.clone();
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = async {
                let _guard = state.write_lock.lock().await;
                let _completion_guard =
                    hold_writes_allowed(&state, Self::persisted_state_blocks_writes()).await?;

                let manager = manager.upgrade().ok_or_else(|| {
                    CloudBackupError::Internal("cloud backup manager stopped".into())
                })?;
                Self::ensure_operation_origin_current(&manager, command).await?;
                completion.apply(&manager).await
            }
            .await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    fn spawn_exclusive_write<T>(
        &self,
        command: CloudBackupWriteCommandContext,
        operation: impl Future<Output = Result<T, CloudBackupError>> + Send + 'static,
    ) -> CloudBackupWriteResultReceiver<T>
    where
        T: Send + 'static,
    {
        let state = Arc::clone(&self.state);
        let (sender, receiver) = oneshot::channel();
        task::spawn(async move {
            let result = Self::run_exclusive_write(state, operation).await;
            let _ = sender.send(CloudBackupWriteCommandResult::new(command, result));
        });

        receiver
    }

    pub(crate) async fn upload_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_allowed_write(self.next_command(None), async move {
            cloud.upload_wallet_backup(namespace, record_id, data).await.map_err(Into::into)
        });
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_wallet_backup_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_allowed_write(self.next_command(Some(origin)), async move {
            cloud.upload_wallet_backup(namespace, record_id, data).await.map_err(Into::into)
        });
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_wallet_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_wallet_upload_with_completion(
            cloud,
            namespace,
            record_id,
            data,
            completion,
            self.next_command(None),
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_master_key_backup_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_allowed_write(self.next_command(Some(origin)), async move {
            cloud.upload_master_key_backup(namespace, data).await.map_err(Into::into)
        });
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_master_key_backup_with_completion_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_master_key_upload_with_completion(
            cloud,
            namespace,
            data,
            completion,
            self.next_command(Some(origin)),
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_allowed_write(self.next_command(None), async move {
            cloud.delete_wallet_backup(namespace, record_id).await.map_err(Into::into)
        });
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_active_wallet_backup_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_active_wallet_delete(
            cloud,
            namespace,
            record_id,
            self.next_command(Some(origin)),
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn finalize_uploaded_wallets_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_finalize_uploaded_wallets(
            cloud,
            namespace_id,
            uploaded_wallets,
            state_mode,
            self.next_command(Some(origin)),
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn complete_uploaded_wallet_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_complete_uploaded_wallet_batch(
            cloud,
            namespace_id,
            uploaded_wallets,
            count_refresh,
            self.next_command(None),
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn apply_completion_for_operation(
        &self,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_apply_completion(completion, self.next_command(Some(origin)));
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_namespace_for_operation(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.spawn_exclusive_write(self.next_command(Some(origin)), async move {
            cloud.delete_namespace(namespace).await.map_err(Into::into)
        });
        Produces::ok(receiver)
    }
}

impl Default for CloudBackupWriteWorker {
    fn default() -> Self {
        Self::new(Weak::new())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupUploadedWallet {
    wallet_id: WalletId,
    record_id: String,
    revision_hash: String,
}

impl CloudBackupUploadedWallet {
    pub(crate) fn new(wallet_id: WalletId, record_id: String, revision_hash: String) -> Self {
        Self { wallet_id, record_id, revision_hash }
    }

    pub(crate) fn record_id(&self) -> &str {
        &self.record_id
    }

    pub(crate) fn revision_hash(&self) -> &str {
        &self.revision_hash
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CloudBackupUploadedWalletsStateMode {
    PreserveVerification,
    ResetVerification,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CloudBackupWalletCountRefresh {
    previous_count: u32,
    estimated_wallet_count: Option<u32>,
    sync_state_estimated_wallet_count: Option<u32>,
}

impl CloudBackupWalletCountRefresh {
    pub(crate) fn new(
        previous_count: u32,
        estimated_wallet_count: Option<u32>,
        sync_state_estimated_wallet_count: Option<u32>,
    ) -> Self {
        Self { previous_count, estimated_wallet_count, sync_state_estimated_wallet_count }
    }

    fn wallet_count(self, listed_wallet_count: Option<u32>) -> u32 {
        [
            Some(self.previous_count),
            self.estimated_wallet_count,
            self.sync_state_estimated_wallet_count,
            listed_wallet_count,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(self.previous_count)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CloudBackupWriteCompletion {
    MarkUploadedPendingConfirmation {
        namespace_id: String,
        record_key: CloudBackupRecordKey,
        revision_hash: String,
        uploaded_at: u64,
    },
    MarkUploadedPendingConfirmationIfCurrent {
        current_state: PersistedCloudBlobSyncState,
        revision_hash: String,
        uploaded_at: u64,
    },
}

impl CloudBackupWriteCompletion {
    pub(crate) fn mark_uploaded_pending_confirmation(
        namespace_id: String,
        record_key: CloudBackupRecordKey,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Self {
        Self::MarkUploadedPendingConfirmation {
            namespace_id,
            record_key,
            revision_hash,
            uploaded_at,
        }
    }

    pub(crate) fn mark_uploaded_pending_confirmation_if_current(
        current_state: PersistedCloudBlobSyncState,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Self {
        Self::MarkUploadedPendingConfirmationIfCurrent { current_state, revision_hash, uploaded_at }
    }

    async fn apply(self, manager: &RustCloudBackupManager) -> Result<(), CloudBackupError> {
        match self {
            Self::MarkUploadedPendingConfirmation {
                namespace_id,
                record_key,
                revision_hash,
                uploaded_at,
            } => {
                manager.mark_blob_uploaded_pending_confirmation(
                    &namespace_id,
                    record_key,
                    revision_hash,
                    uploaded_at,
                )?;
            }
            Self::MarkUploadedPendingConfirmationIfCurrent {
                current_state,
                revision_hash,
                uploaded_at,
            } => {
                let _ = manager.mark_blob_uploaded_pending_confirmation_if_current(
                    &current_state,
                    revision_hash,
                    uploaded_at,
                )?;
            }
        }
        Ok(())
    }
}

async fn ensure_writes_allowed(
    state: &CloudBackupWriteWorkerState,
    writes_blocked_by_persisted_state: bool,
) -> Result<(), CloudBackupError> {
    let active_blocker = state.active_blocker.read().await;
    ensure_writes_allowed_with_blocker(&active_blocker, writes_blocked_by_persisted_state)
}

async fn hold_writes_allowed(
    state: &CloudBackupWriteWorkerState,
    writes_blocked_by_persisted_state: bool,
) -> Result<RwLockReadGuard<'_, Option<CloudBackupWriteBlocker>>, CloudBackupError> {
    let active_blocker = state.active_blocker.read().await;
    ensure_writes_allowed_with_blocker(&active_blocker, writes_blocked_by_persisted_state)?;
    Ok(active_blocker)
}

fn ensure_writes_allowed_with_blocker(
    active_blocker: &Option<CloudBackupWriteBlocker>,
    writes_blocked_by_persisted_state: bool,
) -> Result<(), CloudBackupError> {
    if active_blocker.is_some() || writes_blocked_by_persisted_state {
        return Err(CloudBackupError::Deferred(
            "cloud backup writes are paused while disabling cloud backup".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::super::super::model::CloudBackupExclusiveOperation;
    use super::*;
    use crate::manager::cloud_backup_manager::ops::test_support::{
        reset_cloud_backup_test_state, test_globals, test_lock,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn run_allowed_write_preserves_success_when_writes_block_after_operation() {
        let writes = CloudBackupWriteWorker::new(Weak::new());
        let blocked = AtomicBool::new(false);

        let result = CloudBackupWriteWorker::run_allowed_write(
            Arc::clone(&writes.state),
            async {
                blocked.store(true, Ordering::Relaxed);
                Ok::<_, CloudBackupError>(42)
            },
            || blocked.load(Ordering::Relaxed),
        )
        .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_allowed_write_preserves_operation_error_when_writes_block_after_operation() {
        let writes = CloudBackupWriteWorker::new(Weak::new());
        let blocked = AtomicBool::new(false);

        let result = CloudBackupWriteWorker::run_allowed_write(
            Arc::clone(&writes.state),
            async {
                blocked.store(true, Ordering::Relaxed);
                Err::<(), _>(CloudBackupError::Internal("operation failed".into()))
            },
            || blocked.load(Ordering::Relaxed),
        )
        .await;

        assert!(
            matches!(result, Err(CloudBackupError::Internal(message)) if message == "operation failed")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn worker_blocker_methods_update_actor_state() {
        let mut worker = CloudBackupWriteWorker::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };

        worker.block(blocker).await.unwrap();

        assert_eq!(*worker.state.active_blocker.read().await, Some(blocker));

        worker.unblock(blocker).await.unwrap();

        assert_eq!(*worker.state.active_blocker.read().await, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocker_waits_for_in_flight_completion_guard() {
        let mut worker = CloudBackupWriteWorker::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };
        let state = Arc::clone(&worker.state);
        let completion_guard = hold_writes_allowed(&state, false).await.unwrap();
        let block = worker.block(blocker);

        tokio::pin!(block);
        tokio::select! {
            _ = &mut block => panic!("blocker installed before completion guard released"),
            _ = tokio::task::yield_now() => {}
        }

        drop(completion_guard);
        block.await.unwrap();

        assert_eq!(*state.active_blocker.read().await, Some(blocker));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_results_carry_command_identity_and_origin() {
        let writes = CloudBackupWriteWorker::new(Weak::new());
        let origin =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, 42);

        let first = writes.spawn_exclusive_write(writes.next_command(Some(origin)), async {
            Ok::<_, CloudBackupError>(())
        });
        let first = first.await.unwrap();
        let first_context = first.context();

        assert_eq!(first_context.origin(), Some(origin));
        assert!(first.into_result().is_ok());

        let second = writes.spawn_exclusive_write(writes.next_command(None), async {
            Ok::<_, CloudBackupError>(())
        });
        let second = second.await.unwrap();
        let second_context = second.context();

        assert_ne!(first_context.id(), second_context.id());
        assert_eq!(second_context.origin(), None);
        assert!(second.into_result().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn operation_origin_currentness_rejects_stale_supervisor_operation() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);
        let writes = CloudBackupWriteWorker::new(Arc::downgrade(&manager));
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        let result = CloudBackupWriteWorker::ensure_operation_origin_current(
            &manager,
            writes.next_command(Some(stale)),
        )
        .await;

        assert!(matches!(result, Err(CloudBackupError::Cancelled)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_commands_serialize_accepted_operations() {
        let writes = Arc::new(CloudBackupWriteWorker::new(Weak::new()));
        let first_started = Arc::new(AtomicBool::new(false));
        let first_continue = Arc::new(tokio::sync::Notify::new());
        let second_started = Arc::new(AtomicBool::new(false));

        let first = tokio::spawn({
            let writes = Arc::clone(&writes);
            let first_started = Arc::clone(&first_started);
            let first_continue = Arc::clone(&first_continue);
            async move {
                CloudBackupWriteWorker::run_allowed_write(
                    Arc::clone(&writes.state),
                    async move {
                        first_started.store(true, Ordering::Relaxed);
                        first_continue.notified().await;
                        Ok::<_, CloudBackupError>(())
                    },
                    || false,
                )
                .await
            }
        });
        while !first_started.load(Ordering::Relaxed) {
            tokio::task::yield_now().await;
        }

        let second = tokio::spawn({
            let writes = Arc::clone(&writes);
            let second_started = Arc::clone(&second_started);
            async move {
                CloudBackupWriteWorker::run_allowed_write(
                    Arc::clone(&writes.state),
                    async move {
                        second_started.store(true, Ordering::Relaxed);
                        Ok::<_, CloudBackupError>(())
                    },
                    || false,
                )
                .await
            }
        });

        tokio::task::yield_now().await;
        assert!(!second_started.load(Ordering::Relaxed));

        first_continue.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert!(second_started.load(Ordering::Relaxed));
    }
}
