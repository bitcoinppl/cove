use std::sync::Arc;

use act_zero::{Addr, call};
use cove_device::cloud_storage::CloudStorageClient;

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::owner::delete_authorization::DeleteAuthorization;

use super::keychain::OwnedKeychainWrite;
use super::policy::{CommandClass, ConfigGeneration};
use super::supervisor::{CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor, LaneOutput};
use super::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};
use super::worker::{RowDeleteReport, WalletUploadOutcome};
use crate::manager::cloud_backup_manager::wallets::{WalletBackupReader, WalletUpload};

/// Who a producer writes for, which gives every command or local effect it sends its class
///
/// Producers read it before anything they write depends on, so a configuration or operation that
/// changes after that leaves their writes with an authority the owner no longer accepts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteOwner {
    /// Background work, read under this configuration generation
    Background(ConfigGeneration),
    /// The running exclusive operation with this claim
    Operation(CloudBackupExclusiveOperationClaim),
}

impl WriteOwner {
    /// The class the write policy decides this owner's writes under
    fn class(self) -> CommandClass {
        match self {
            Self::Background(generation) => CommandClass::Ordinary { generation },
            Self::Operation(claim) => CommandClass::Operation(claim),
        }
    }
}

/// Handle for submitting writes to the write supervisor
///
/// Background producers use [`CloudBackupWriteClient::background`] with the generation they read
/// before their configuration. Exclusive operations use [`CloudBackupWriteClient::for_operation`],
/// so the lane runs their commands only while the operation runs
#[derive(Clone)]
pub(crate) struct CloudBackupWriteClient {
    supervisor: Addr<CloudBackupWriteSupervisor>,
    owner: WriteOwner,
}

impl CloudBackupWriteClient {
    pub(crate) fn background(
        supervisor: Addr<CloudBackupWriteSupervisor>,
        generation: ConfigGeneration,
    ) -> Self {
        Self { supervisor, owner: WriteOwner::Background(generation) }
    }

    pub(crate) fn for_operation(
        supervisor: Addr<CloudBackupWriteSupervisor>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Self {
        Self { supervisor, owner: WriteOwner::Operation(origin) }
    }

    fn origin(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        match self.owner {
            WriteOwner::Background(_) => None,
            WriteOwner::Operation(claim) => Some(claim),
        }
    }

    fn operation(
        &self,
        what: &str,
    ) -> Result<CloudBackupExclusiveOperationClaim, CloudBackupError> {
        self.origin().ok_or_else(|| {
            CloudBackupError::Internal(format!("cloud backup {what} requires an operation").into())
        })
    }

    /// Waits for a command's result; `receiver` is what `call!` gave when submitting it
    async fn await_result<T, E>(
        &self,
        receiver: Result<CloudBackupWriteResultReceiver<T>, E>,
    ) -> Result<T, CloudBackupError>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let receiver = receiver.map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;
        let result = receiver.await.map_err(|source| {
            CloudBackupError::internal_context("wait for cloud backup write supervisor", source)
        })?;

        let context = result.context();
        let context_id = context.id();
        if context.origin() != self.origin() {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor returned mismatched operation origin for command {context_id:?}",
            )
            .into()));
        }

        result.into_result()
    }

    /// Waits for a command whose only answer is success or failure
    async fn await_done<E>(
        &self,
        receiver: Result<CloudBackupWriteResultReceiver<LaneOutput>, E>,
    ) -> Result<(), CloudBackupError>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.await_result(receiver).await.map(drop)
    }

    pub(crate) async fn upload_wallet_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        upload: WalletUpload,
    ) -> Result<WalletUploadOutcome, CloudBackupError> {
        let class = self.owner.class();
        let receiver =
            call!(self.supervisor.upload_wallet_backup(class, cloud, namespace, upload, None))
                .await;
        self.await_wallet_upload(receiver).await
    }

    /// Uploads a wallet record and applies `completion` only once it was written
    pub(crate) async fn upload_wallet_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        upload: WalletUpload,
        completion: CloudBackupWriteCompletion,
    ) -> Result<WalletUploadOutcome, CloudBackupError> {
        let class = self.owner.class();
        let receiver = call!(self.supervisor.upload_wallet_backup(
            class,
            cloud,
            namespace,
            upload,
            Some(completion)
        ))
        .await;
        self.await_wallet_upload(receiver).await
    }

    async fn await_wallet_upload<E>(
        &self,
        receiver: Result<CloudBackupWriteResultReceiver<LaneOutput>, E>,
    ) -> Result<WalletUploadOutcome, CloudBackupError>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match self.await_result(receiver).await? {
            LaneOutput::Done => Ok(WalletUploadOutcome::Uploaded),
            LaneOutput::KeptKeyMaterial => Ok(WalletUploadOutcome::KeptKeyMaterial),
            LaneOutput::RowDelete(_) | LaneOutput::RecordKept => Err(CloudBackupError::Internal(
                "cloud backup write lane answered a wallet upload with a row delete report".into(),
            )),
        }
    }

    pub(crate) async fn upload_master_key_backup(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::Operation(self.operation("master-key upload")?);
        let receiver =
            call!(self.supervisor.upload_master_key_backup(class, cloud, namespace, data, None))
                .await;
        self.await_done(receiver).await
    }

    pub(crate) async fn upload_master_key_backup_with_completion(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::Operation(self.operation("master-key upload completion")?);
        let receiver = call!(self.supervisor.upload_master_key_backup(
            class,
            cloud,
            namespace,
            data,
            Some(completion)
        ))
        .await;
        self.await_done(receiver).await
    }

    pub(crate) async fn finalize_uploaded_wallets(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::Operation(self.operation("wallet finalization")?);
        let receiver = call!(self.supervisor.finalize_uploaded_wallets(
            class,
            cloud,
            namespace_id,
            uploaded_wallets,
            state_mode
        ))
        .await;
        self.await_done(receiver).await
    }

    pub(crate) async fn complete_uploaded_wallet_batch(
        &self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> Result<(), CloudBackupError> {
        let class = self.owner.class();
        let receiver = call!(self.supervisor.complete_uploaded_wallet_batch(
            class,
            cloud,
            namespace_id,
            uploaded_wallets,
            count_refresh
        ))
        .await;
        self.await_done(receiver).await
    }

    pub(crate) async fn apply_completion(
        &self,
        completion: CloudBackupWriteCompletion,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::Operation(self.operation("write completion")?);
        let receiver = call!(self.supervisor.apply_completion(class, completion)).await;
        self.await_done(receiver).await
    }

    /// Deletes one wallet record a row delete's authorization names, if it still holds when the
    /// lane reaches it
    pub(crate) async fn delete_authorized_record(
        &self,
        cloud: CloudStorageClient,
        authorization: Arc<DeleteAuthorization>,
        record_id: String,
    ) -> Result<RowDeleteReport, CloudBackupError> {
        let claim = self.operation("row delete")?;
        let receiver =
            call!(self.supervisor.delete_wallet_record(claim, authorization, record_id, cloud))
                .await;

        match self.await_result(receiver).await? {
            LaneOutput::RowDelete(report) => Ok(report),
            LaneOutput::Done | LaneOutput::KeptKeyMaterial | LaneOutput::RecordKept => {
                Err(CloudBackupError::Internal(
                    "cloud backup write lane answered a row delete without its report".into(),
                ))
            }
        }
    }

    /// Deletes one record of an unreadable batch, if its authorization still holds when the lane
    /// reaches it and `reader`, the configuration's verified key, still fails to decrypt it then
    ///
    /// `None` when the lane's read kept the record; the inventory owner has what it read
    pub(crate) async fn delete_unreadable_record(
        &self,
        cloud: CloudStorageClient,
        reader: WalletBackupReader,
        authorization: Arc<DeleteAuthorization>,
        record_id: String,
    ) -> Result<Option<RowDeleteReport>, CloudBackupError> {
        let claim = self.operation("unreadable delete")?;
        let receiver = call!(self.supervisor.delete_unreadable_record(
            claim,
            authorization,
            record_id,
            cloud,
            reader
        ))
        .await;

        match self.await_result(receiver).await? {
            LaneOutput::RowDelete(report) => Ok(Some(report)),
            LaneOutput::RecordKept => Ok(None),
            LaneOutput::Done | LaneOutput::KeptKeyMaterial => Err(CloudBackupError::Internal(
                "cloud backup write lane answered an unreadable delete without its report".into(),
            )),
        }
    }

    /// Turn Off's delete of the namespace it committed to delete
    pub(crate) async fn delete_committed_namespace(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let claim = self.operation("Turn Off namespace delete")?;
        let class = CommandClass::TurnOffDelete { claim, namespace_id: namespace.clone() };
        self.delete_namespace(class, cloud, namespace).await
    }

    /// Recover Other Backups or Delete Other Backups deleting another namespace
    pub(crate) async fn delete_other_namespace(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::OtherBackupDelete(self.operation("other backup delete")?);
        self.delete_namespace(class, cloud, namespace).await
    }

    /// Discarding the namespace an interrupted Enable wrote
    pub(crate) async fn delete_discarded_namespace(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let class = CommandClass::EnableDiscard(self.operation("pending Enable discard")?);
        self.delete_namespace(class, cloud, namespace).await
    }

    /// Deletes a namespace a merge made redundant, proven under this client's generation
    pub(crate) async fn delete_source_namespace(
        &self,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let WriteOwner::Background(generation) = self.owner else {
            return Err(CloudBackupError::Internal(
                "merged namespace cleanup runs in the background, not in an operation".into(),
            ));
        };

        self.delete_namespace(CommandClass::SourceCleanup { generation }, cloud, namespace).await
    }

    async fn delete_namespace(
        &self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> Result<(), CloudBackupError> {
        let receiver = call!(self.supervisor.delete_namespace(class, cloud, namespace)).await;
        self.await_done(receiver).await
    }

    /// Saves keys this client's owner proved for `namespace_id`
    ///
    /// The lane decides the save under the owner's authority and runs it off the actor, so it is
    /// refused once the owner's operation ended or its configuration was replaced, and Turn Off and
    /// a reset wait for a save that already started
    pub(crate) async fn save_keychain(
        &self,
        namespace_id: String,
        writes: Vec<OwnedKeychainWrite>,
    ) -> Result<(), CloudBackupError> {
        let class = self.owner.class();
        let receiver = call!(self.supervisor.save_keychain(class, namespace_id, writes)).await;
        self.await_done(receiver).await
    }

    /// Waits until every write queued before this one finished, while the operation runs
    pub(crate) async fn drain(&self) -> Result<(), CloudBackupError> {
        let class = CommandClass::Operation(self.operation("write drain")?);
        let receiver = call!(self.supervisor.barrier(class)).await;
        self.await_done(receiver).await
    }
}
