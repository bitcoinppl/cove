use std::collections::VecDeque;
use std::sync::Weak;

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, send};
use cove_device::cloud_storage::CloudStorageClient;
use cove_tokio::task::spawn_actor;
use tokio::sync::oneshot;
use tracing::{error, warn};

use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::{CloudBackupStore, RustCloudBackupManager};

use super::super::supervisor::CloudBackupSupervisor;
use super::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};
use super::worker::{
    CloudBackupRemoteWriteCommand, CloudBackupRemoteWriteResult, CloudBackupWriteWorker,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupWriteBlocker {
    Disabling { operation_id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudBackupWriteAdmission {
    RequiresWritesAllowed,
    BypassBlocker,
}

impl CloudBackupWriteAdmission {
    fn requires_writes_allowed(self) -> bool {
        matches!(self, Self::RequiresWritesAllowed)
    }
}

#[derive(Debug)]
enum CloudBackupWriteLocalCompletion {
    None,
    Apply(CloudBackupWriteCompletion),
    DeleteActiveWallet {
        record_id: String,
    },
    FinalizeUploadedWallets {
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
    },
    CompleteUploadedWalletBatch {
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    },
}

impl CloudBackupWriteLocalCompletion {
    fn requires_writes_allowed(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug)]
struct CloudBackupPendingWrite {
    context: CloudBackupWriteCommandContext,
    admission: CloudBackupWriteAdmission,
    remote: CloudBackupRemoteWriteCommand,
    completion: CloudBackupWriteLocalCompletion,
    sender: oneshot::Sender<CloudBackupWriteCommandResult<()>>,
}

impl CloudBackupPendingWrite {
    fn in_flight(self) -> (CloudBackupInFlightWrite, CloudBackupRemoteWriteCommand) {
        (
            CloudBackupInFlightWrite {
                context: self.context,
                completion: self.completion,
                sender: self.sender,
            },
            self.remote,
        )
    }

    fn complete(self, result: Result<(), CloudBackupError>) {
        let _ = self.sender.send(CloudBackupWriteCommandResult::new(self.context, result));
    }
}

#[derive(Debug)]
struct CloudBackupInFlightWrite {
    context: CloudBackupWriteCommandContext,
    completion: CloudBackupWriteLocalCompletion,
    sender: oneshot::Sender<CloudBackupWriteCommandResult<()>>,
}

impl CloudBackupInFlightWrite {
    fn complete(self, result: Result<(), CloudBackupError>) {
        let _ = self.sender.send(CloudBackupWriteCommandResult::new(self.context, result));
    }
}

#[derive(Debug)]
struct CloudBackupWriteDrainWaiter {
    supervisor: WeakAddr<CloudBackupSupervisor>,
    claim: CloudBackupExclusiveOperationClaim,
    blocker: CloudBackupWriteBlocker,
}

#[derive(Debug)]
pub(crate) struct CloudBackupWriteSupervisor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    worker: Option<Addr<CloudBackupWriteWorker>>,
    active_blocker: Option<CloudBackupWriteBlocker>,
    in_flight_write: Option<CloudBackupInFlightWrite>,
    pending_writes: VecDeque<CloudBackupPendingWrite>,
    drain_waiters: Vec<CloudBackupWriteDrainWaiter>,
    next_command_id: u64,
}

#[async_trait::async_trait]
impl Actor for CloudBackupWriteSupervisor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.worker = Some(spawn_actor(CloudBackupWriteWorker::default()));
        Produces::ok(())
    }
}

impl CloudBackupWriteSupervisor {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self {
            addr: WeakAddr::default(),
            manager,
            worker: None,
            active_blocker: None,
            in_flight_write: None,
            pending_writes: VecDeque::new(),
            drain_waiters: Vec::new(),
            next_command_id: 0,
        }
    }

    pub(crate) async fn block(&mut self, blocker: CloudBackupWriteBlocker) -> ActorResult<()> {
        self.active_blocker = Some(blocker);
        self.reject_blocked_pending_writes();
        self.start_next_pending_write();
        Produces::ok(())
    }

    pub(crate) async fn block_until_drained(
        &mut self,
        blocker: CloudBackupWriteBlocker,
        supervisor: WeakAddr<CloudBackupSupervisor>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        self.active_blocker = Some(blocker);
        self.reject_blocked_pending_writes();
        self.start_next_pending_write();

        if self.in_flight_write.is_some() {
            self.drain_waiters.push(CloudBackupWriteDrainWaiter { supervisor, claim, blocker });
        } else {
            send!(supervisor.complete_disable_write_drain(claim, blocker));
        }

        Produces::ok(())
    }

    pub(crate) async fn unblock(&mut self, blocker: CloudBackupWriteBlocker) -> ActorResult<()> {
        if self.active_blocker == Some(blocker) {
            self.active_blocker = None;
        }
        self.drain_waiters.retain(|waiter| waiter.blocker != blocker);
        self.start_next_pending_write();
        Produces::ok(())
    }

    fn persisted_state_blocks_writes() -> bool {
        Database::global()
            .cloud_backup_state
            .get()
            .unwrap_or_else(|error| {
                error!("Failed to load cloud backup state for write supervisor: {error}");
                PersistedCloudBackupState::default()
            })
            .is_disabling()
    }

    fn next_command(
        &self,
        origin: Option<CloudBackupExclusiveOperationClaim>,
    ) -> CloudBackupWriteCommandContext {
        CloudBackupWriteCommandContext::new(self.next_command_id, origin)
    }

    fn ensure_operation_origin_current(
        manager: &RustCloudBackupManager,
        command: CloudBackupWriteCommandContext,
    ) -> Result<(), CloudBackupError> {
        let Some(origin) = command.origin() else { return Ok(()) };

        if manager.projected_exclusive_operation() == Some(origin) {
            Ok(())
        } else {
            Err(CloudBackupError::Cancelled)
        }
    }

    fn ensure_pending_write_origin_current(
        &self,
        context: CloudBackupWriteCommandContext,
    ) -> Result<(), CloudBackupError> {
        if context.origin().is_none() {
            return Ok(());
        }

        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| CloudBackupError::Internal("cloud backup manager stopped".into()))?;

        Self::ensure_operation_origin_current(&manager, context)
    }

    fn advance_command_id(&mut self) -> CloudBackupWriteCommandContext {
        let command = self.next_command(None);
        self.next_command_id += 1;
        command
    }

    fn advance_operation_command_id(
        &mut self,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> CloudBackupWriteCommandContext {
        let command = self.next_command(Some(origin));
        self.next_command_id += 1;
        command
    }

    fn writes_allowed(&self) -> Result<(), CloudBackupError> {
        if self.active_blocker.is_some() {
            return Err(blocked_writes_error());
        }

        ensure_writes_allowed_with_blocker(
            &self.active_blocker,
            Self::persisted_state_blocks_writes(),
        )
    }

    fn submit_write(
        &mut self,
        admission: CloudBackupWriteAdmission,
        remote: CloudBackupRemoteWriteCommand,
        completion: CloudBackupWriteLocalCompletion,
        command: CloudBackupWriteCommandContext,
    ) -> CloudBackupWriteResultReceiver<()> {
        let (sender, receiver) = oneshot::channel();
        let write =
            CloudBackupPendingWrite { context: command, admission, remote, completion, sender };

        if write.admission.requires_writes_allowed()
            && let Err(error) = self.writes_allowed()
        {
            write.complete(Err(error));
            return receiver;
        }

        self.pending_writes.push_back(write);
        self.start_next_pending_write();

        receiver
    }

    fn start_next_pending_write(&mut self) {
        if self.in_flight_write.is_some() {
            return;
        }

        while let Some(write) = self.pending_writes.pop_front() {
            if write.admission.requires_writes_allowed()
                && let Err(error) = self.writes_allowed()
            {
                write.complete(Err(error));
                continue;
            }

            if let Err(error) = self.ensure_pending_write_origin_current(write.context) {
                write.complete(Err(error));
                continue;
            }

            self.start_pending_write(write);
            break;
        }
    }

    fn complete_drain_waiters_if_idle(&mut self) {
        if self.in_flight_write.is_some() {
            return;
        }

        for waiter in self.drain_waiters.drain(..) {
            send!(waiter.supervisor.complete_disable_write_drain(waiter.claim, waiter.blocker));
        }
    }

    fn start_pending_write(&mut self, write: CloudBackupPendingWrite) {
        let context = write.context;
        let (in_flight, remote) = write.in_flight();
        self.in_flight_write = Some(in_flight);
        if let Some(worker) = &self.worker {
            send!(worker.execute(self.addr.clone(), context, remote));
        }
    }

    fn reject_blocked_pending_writes(&mut self) {
        let mut retained = VecDeque::new();
        while let Some(write) = self.pending_writes.pop_front() {
            if write.admission.requires_writes_allowed() {
                write.complete(Err(blocked_writes_error()));
            } else {
                retained.push_back(write);
            }
        }
        self.pending_writes = retained;
    }

    pub(crate) async fn complete_remote_write(
        &mut self,
        context: CloudBackupWriteCommandContext,
        result: Result<CloudBackupRemoteWriteResult, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(active) = self.in_flight_write.take() else {
            warn!("Cloud backup write supervisor received completion for inactive command");
            return Produces::ok(());
        };

        if active.context != context {
            active.complete(Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor received mismatched completion for command {:?}",
                context.id()
            ))));
            self.start_next_pending_write();
            self.complete_drain_waiters_if_idle();
            return Produces::ok(());
        }

        let result = match result {
            Ok(output) => self.apply_local_completion(&active, output).await,
            Err(error) => Err(error),
        };
        active.complete(result);
        self.start_next_pending_write();
        self.complete_drain_waiters_if_idle();

        Produces::ok(())
    }

    async fn apply_local_completion(
        &self,
        active: &CloudBackupInFlightWrite,
        output: CloudBackupRemoteWriteResult,
    ) -> Result<(), CloudBackupError> {
        if active.completion.requires_writes_allowed() {
            self.writes_allowed()?;
        }

        self.ensure_pending_write_origin_current(active.context)?;

        let manager = match &active.completion {
            CloudBackupWriteLocalCompletion::None => return Ok(()),
            _ => self
                .manager
                .upgrade()
                .ok_or_else(|| CloudBackupError::Internal("cloud backup manager stopped".into()))?,
        };

        match (&active.completion, output) {
            // apply caller-provided local state only after the remote write succeeds
            (
                CloudBackupWriteLocalCompletion::Apply(completion),
                CloudBackupRemoteWriteResult::None,
            ) => completion.clone().apply(&manager).await,

            // remove the local active-wallet sync state and refresh the count from cloud
            (
                CloudBackupWriteLocalCompletion::DeleteActiveWallet { record_id },
                CloudBackupRemoteWriteResult::WalletRecordIds(wallet_record_ids),
            ) => {
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

            // persist the enabled state before marking the uploaded wallets pending confirmation
            (
                CloudBackupWriteLocalCompletion::FinalizeUploadedWallets {
                    namespace_id,
                    uploaded_wallets,
                    state_mode,
                },
                CloudBackupRemoteWriteResult::WalletCount(wallet_count),
            ) => {
                match state_mode {
                    CloudBackupUploadedWalletsStateMode::PreserveVerification => {
                        CloudBackupStore::global().persist_enabled(wallet_count)?;
                    }
                    CloudBackupUploadedWalletsStateMode::ResetVerification => {
                        CloudBackupStore::global()
                            .persist_enabled_reset_verification(wallet_count)?;
                    }
                }

                self.mark_uploaded_wallets(&manager, namespace_id, uploaded_wallets).await
            }

            // complete a background upload batch using the best wallet count available
            (
                CloudBackupWriteLocalCompletion::CompleteUploadedWalletBatch {
                    namespace_id,
                    uploaded_wallets,
                    count_refresh,
                },
                CloudBackupRemoteWriteResult::ListedWalletCount(listed_wallet_count),
            ) => {
                let wallet_count = count_refresh.wallet_count(listed_wallet_count);
                CloudBackupStore::global().persist_enabled(wallet_count)?;
                self.mark_uploaded_wallets(&manager, namespace_id, uploaded_wallets).await
            }

            _ => Err(CloudBackupError::Internal(
                "cloud backup write supervisor received mismatched write output".into(),
            )),
        }
    }

    async fn mark_uploaded_wallets(
        &self,
        manager: &RustCloudBackupManager,
        namespace_id: &str,
        uploaded_wallets: &[CloudBackupUploadedWallet],
    ) -> Result<(), CloudBackupError> {
        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        for wallet in uploaded_wallets {
            manager
                .mark_wallet_uploaded_pending_confirmation_if_revision_current(
                    namespace_id,
                    wallet.wallet_id().clone(),
                    wallet.record_id().to_owned(),
                    wallet.revision_hash().to_owned(),
                    uploaded_at,
                )
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn upload_wallet_backup(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_command_id();
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::UploadWallet { cloud, namespace, record_id, data },
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_wallet_backup_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::UploadWallet { cloud, namespace, record_id, data },
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_wallet_backup_with_completion(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_command_id();
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::UploadWallet { cloud, namespace, record_id, data },
            CloudBackupWriteLocalCompletion::Apply(completion),
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_master_key_backup_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::UploadMasterKey { cloud, namespace, data },
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_master_key_backup_with_completion_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::UploadMasterKey { cloud, namespace, data },
            CloudBackupWriteLocalCompletion::Apply(completion),
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_wallet_backup(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_command_id();
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::DeleteWallet { cloud, namespace, record_id },
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_active_wallet_backup_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::DeleteActiveWallet {
                cloud,
                namespace,
                record_id: record_id.clone(),
            },
            CloudBackupWriteLocalCompletion::DeleteActiveWallet { record_id },
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn finalize_uploaded_wallets_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let fallback_count = uploaded_wallets.len() as u32;
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::ListWalletCount {
                cloud,
                namespace_id: namespace_id.clone(),
                fallback_count,
            },
            CloudBackupWriteLocalCompletion::FinalizeUploadedWallets {
                namespace_id,
                uploaded_wallets,
                state_mode,
            },
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn complete_uploaded_wallet_batch(
        &mut self,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_command_id();
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::ListWalletCountOptional {
                cloud,
                namespace_id: namespace_id.clone(),
            },
            CloudBackupWriteLocalCompletion::CompleteUploadedWalletBatch {
                namespace_id,
                uploaded_wallets,
                count_refresh,
            },
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn apply_completion_for_operation(
        &mut self,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::Apply(completion),
            command,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_namespace_for_operation(
        &mut self,
        cloud: CloudStorageClient,
        namespace: String,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let command = self.advance_operation_command_id(origin);
        let receiver = self.submit_write(
            CloudBackupWriteAdmission::BypassBlocker,
            CloudBackupRemoteWriteCommand::DeleteNamespace { cloud, namespace },
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        Produces::ok(receiver)
    }
}

impl Default for CloudBackupWriteSupervisor {
    fn default() -> Self {
        Self::new(Weak::new())
    }
}

fn ensure_writes_allowed_with_blocker(
    active_blocker: &Option<CloudBackupWriteBlocker>,
    writes_blocked_by_persisted_state: bool,
) -> Result<(), CloudBackupError> {
    if active_blocker.is_some() || writes_blocked_by_persisted_state {
        return Err(blocked_writes_error());
    }

    Ok(())
}

fn blocked_writes_error() -> CloudBackupError {
    CloudBackupError::Deferred("cloud backup writes are paused while disabling cloud backup".into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperation;
    use crate::manager::cloud_backup_manager::ops::test_support::{
        async_test_lock, reset_cloud_backup_test_state, test_globals,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_blocker_methods_update_actor_state() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };

        supervisor.block(blocker).await.unwrap();

        assert_eq!(supervisor.active_blocker, Some(blocker));

        supervisor.unblock(blocker).await.unwrap();

        assert_eq!(supervisor.active_blocker, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_rejects_pending_writes_that_require_admission() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };
        let (pending, receiver) =
            pending_write(CloudBackupWriteAdmission::RequiresWritesAllowed, 1);
        supervisor.in_flight_write =
            Some(in_flight_write(0, CloudBackupWriteAdmission::RequiresWritesAllowed));
        supervisor.pending_writes.push_back(pending);

        supervisor.block(blocker).await.unwrap();

        let result = receiver.await.unwrap().into_result();
        assert!(matches!(result, Err(CloudBackupError::Deferred(_))));
        assert!(supervisor.pending_writes.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_keeps_bypass_writes_pending() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };
        let (blocked, blocked_receiver) =
            pending_write(CloudBackupWriteAdmission::RequiresWritesAllowed, 1);
        let (bypass, _bypass_receiver) = pending_write(CloudBackupWriteAdmission::BypassBlocker, 2);
        supervisor.in_flight_write =
            Some(in_flight_write(0, CloudBackupWriteAdmission::RequiresWritesAllowed));
        supervisor.pending_writes.push_back(blocked);
        supervisor.pending_writes.push_back(bypass);

        supervisor.block(blocker).await.unwrap();

        assert!(matches!(
            blocked_receiver.await.unwrap().into_result(),
            Err(CloudBackupError::Deferred(_))
        ));
        assert_eq!(supervisor.pending_writes.len(), 1);

        supervisor.in_flight_write = None;
        supervisor.start_next_pending_write();

        assert_eq!(
            supervisor.in_flight_write.as_ref().map(|write| write.context.id()),
            Some(CloudBackupWriteCommandId(2))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_until_drained_waits_for_in_flight_write() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 7 };
        supervisor.in_flight_write =
            Some(in_flight_write(0, CloudBackupWriteAdmission::RequiresWritesAllowed));

        supervisor
            .block_until_drained(
                blocker,
                WeakAddr::<CloudBackupSupervisor>::default(),
                CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, 42),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_blocker, Some(blocker));
        assert_eq!(supervisor.drain_waiters.len(), 1);

        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(0, None),
                Ok(CloudBackupRemoteWriteResult::None),
            )
            .await
            .unwrap();

        assert!(supervisor.in_flight_write.is_none());
        assert!(supervisor.drain_waiters.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_command_contexts_carry_identity_and_origin() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        let origin =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, 42);

        let first_context = supervisor.advance_operation_command_id(origin);
        assert_eq!(first_context.origin(), Some(origin));

        let second_context = supervisor.advance_command_id();

        assert_ne!(first_context.id(), second_context.id());
        assert_eq!(second_context.origin(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn operation_origin_currentness_rejects_stale_supervisor_operation() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);
        let supervisor = CloudBackupWriteSupervisor::new(Arc::downgrade(&manager));
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        let result = CloudBackupWriteSupervisor::ensure_operation_origin_current(
            &manager,
            supervisor.next_command(Some(stale)),
        );

        assert!(matches!(result, Err(CloudBackupError::Cancelled)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_operation_write_is_rejected_before_starting_remote_execution() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);
        let mut supervisor = CloudBackupWriteSupervisor::new(Arc::downgrade(&manager));
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );
        let command = supervisor.advance_operation_command_id(stale);

        let receiver = supervisor.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::None,
            command,
        );

        let result = receiver.await.unwrap().into_result();
        assert!(matches!(result, Err(CloudBackupError::Cancelled)));
        assert!(supervisor.in_flight_write.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_queued_operation_write_is_rejected_before_starting_remote_execution() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);
        let mut supervisor = CloudBackupWriteSupervisor::new(Arc::downgrade(&manager));
        supervisor.in_flight_write =
            Some(in_flight_write(0, CloudBackupWriteAdmission::RequiresWritesAllowed));
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );
        let command = supervisor.advance_operation_command_id(stale);

        let receiver = supervisor.submit_write(
            CloudBackupWriteAdmission::RequiresWritesAllowed,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::None,
            command,
        );
        assert_eq!(supervisor.pending_writes.len(), 1);

        supervisor.in_flight_write = None;
        supervisor.start_next_pending_write();

        let result = receiver.await.unwrap().into_result();
        assert!(matches!(result, Err(CloudBackupError::Cancelled)));
        assert!(supervisor.in_flight_write.is_none());
        assert!(supervisor.pending_writes.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_completion_rechecks_write_blocker() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        supervisor.active_blocker = Some(CloudBackupWriteBlocker::Disabling { operation_id: 7 });
        let in_flight = in_flight_completion_write(0);

        let result =
            supervisor.apply_local_completion(&in_flight, CloudBackupRemoteWriteResult::None).await;

        assert!(matches!(result, Err(CloudBackupError::Deferred(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn remote_only_completion_ignores_write_blocker() {
        let mut supervisor = CloudBackupWriteSupervisor::new(Weak::new());
        supervisor.active_blocker = Some(CloudBackupWriteBlocker::Disabling { operation_id: 7 });
        let in_flight = in_flight_write(0, CloudBackupWriteAdmission::RequiresWritesAllowed);

        let result =
            supervisor.apply_local_completion(&in_flight, CloudBackupRemoteWriteResult::None).await;

        assert!(result.is_ok());
    }

    fn in_flight_write(id: u64, _admission: CloudBackupWriteAdmission) -> CloudBackupInFlightWrite {
        let (sender, _receiver) = oneshot::channel();
        CloudBackupInFlightWrite {
            context: CloudBackupWriteCommandContext::new(id, None),
            completion: CloudBackupWriteLocalCompletion::None,
            sender,
        }
    }

    fn in_flight_completion_write(id: u64) -> CloudBackupInFlightWrite {
        let (sender, _receiver) = oneshot::channel();
        CloudBackupInFlightWrite {
            context: CloudBackupWriteCommandContext::new(id, None),
            completion: CloudBackupWriteLocalCompletion::Apply(
                CloudBackupWriteCompletion::mark_uploaded_pending_confirmation(
                    "namespace".into(),
                    CloudBackupRecordKey::MasterKeyWrapper,
                    "revision".into(),
                    0,
                ),
            ),
            sender,
        }
    }

    fn pending_write(
        admission: CloudBackupWriteAdmission,
        id: u64,
    ) -> (CloudBackupPendingWrite, CloudBackupWriteResultReceiver<()>) {
        let (sender, receiver) = oneshot::channel();
        (
            CloudBackupPendingWrite {
                context: CloudBackupWriteCommandContext::new(id, None),
                admission,
                remote: CloudBackupRemoteWriteCommand::None,
                completion: CloudBackupWriteLocalCompletion::None,
                sender,
            },
            receiver,
        )
    }
}
