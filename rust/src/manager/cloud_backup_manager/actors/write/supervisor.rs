//! Ordered Cloud Backup write supervisor
//!
//! This actor serializes cloud writes and applies local completion after remote success. Every
//! command carries its [`CommandClass`]; the supervisor decides it at the head of the queue, when
//! it is about to run, against the latest [`WritePolicy`] the lifecycle supervisor pushed. A
//! paused command answers `WritesPaused`, a stale one is dropped, and nothing here holds a pause
//! that someone has to lift

use std::collections::VecDeque;
use std::sync::Weak;

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, call, send};
use cove_device::cloud_storage::CloudStorageClient;
use cove_tokio::task::spawn_actor;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::manager::cloud_backup_manager::admission::PauseReason;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

use super::keychain::{KeychainSave, OwnedKeychainWrite};
use super::policy::{CommandClass, ConfigGeneration, Decision, VersionedPolicy, WritePolicy};
use super::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};
use super::worker::{
    CloudBackupRemoteWriteCommand, CloudBackupRemoteWriteResult, CloudBackupWriteWorker,
};

pub(crate) type CloudBackupWriteResultReceiver<T> =
    oneshot::Receiver<CloudBackupWriteCommandResult<T>>;

/// Monotonic identifier for commands accepted by the write supervisor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupWriteCommandId(u64);

/// Write command identity plus the exclusive operation that owns it, if any
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

/// Result returned to the submitter after remote and local write work finishes
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

/// What a producer with no operation needs before it reads its configuration
///
/// The generation is read first, so a configuration that changes after the producer read it
/// leaves the command with the older generation, which the lane drops
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriteTicket {
    pub(crate) generation: ConfigGeneration,
    /// What the lane would decide for an ordinary write right now; the pending upload verifier
    /// reads it because it isn't a lane command, so the lane can't decide it at a queue head
    pub(crate) ordinary: Decision,
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
    /// A keychain save, which answers stale when the keychain moved to another namespace
    KeychainSave,
}

#[derive(Debug)]
struct CloudBackupPendingWrite {
    context: CloudBackupWriteCommandContext,
    class: CommandClass,
    remote: CloudBackupRemoteWriteCommand,
    completion: CloudBackupWriteLocalCompletion,
    sender: oneshot::Sender<CloudBackupWriteCommandResult<()>>,
}

impl CloudBackupPendingWrite {
    fn in_flight(self) -> (CloudBackupInFlightWrite, CloudBackupRemoteWriteCommand) {
        (
            CloudBackupInFlightWrite {
                context: self.context,
                class: self.class,
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
    class: CommandClass,
    completion: CloudBackupWriteLocalCompletion,
    sender: oneshot::Sender<CloudBackupWriteCommandResult<()>>,
}

impl CloudBackupInFlightWrite {
    fn complete(self, result: Result<(), CloudBackupError>) {
        let _ = self.sender.send(CloudBackupWriteCommandResult::new(self.context, result));
    }
}

/// The answer a stale command gets: operation work is cancelled, and background work keeps its
/// local state so the new configuration's scheduler redoes it
fn stale_error(class: &CommandClass) -> CloudBackupError {
    match class.claim() {
        Some(_) => CloudBackupError::Cancelled,
        None => CloudBackupError::WritesPaused(PauseReason::ConfigurationChanged),
    }
}

/// Coordinates one-at-a-time remote writes and their local completion
#[derive(Debug)]
pub(crate) struct CloudBackupWriteSupervisor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    worker: Option<Addr<CloudBackupWriteWorker>>,
    /// The latest policy the lifecycle supervisor published
    policy: VersionedPolicy,
    in_flight_write: Option<CloudBackupInFlightWrite>,
    pending_writes: VecDeque<CloudBackupPendingWrite>,
    /// Told once no write is in flight
    idle_waiters: Vec<oneshot::Sender<()>>,
    next_command_id: u64,
    /// Every queue-head decision since the harness last read them
    #[cfg(test)]
    decisions: Vec<LaneDecision>,
}

/// One decision the lane made at the queue head, with the policy it decided under
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct LaneDecision {
    pub(crate) class: CommandClass,
    pub(crate) policy: VersionedPolicy,
    pub(crate) decision: Decision,
}

/// What the harness invariants read from the lane at a settled boundary
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct LaneView {
    pub(crate) applied: VersionedPolicy,
    pub(crate) decisions: Vec<LaneDecision>,
}

/// The lane's commands at one moment, oldest first
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct LaneQueue {
    pub(crate) in_flight: Option<CommandClass>,
    pub(crate) pending: Vec<CommandClass>,
}

impl Default for CloudBackupWriteSupervisor {
    fn default() -> Self {
        Self::new(Weak::new())
    }
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
            policy: VersionedPolicy { version: 0, policy: WritePolicy::starting() },
            in_flight_write: None,
            pending_writes: VecDeque::new(),
            idle_waiters: Vec::new(),
            next_command_id: 0,
            #[cfg(test)]
            decisions: Vec::new(),
        }
    }

    /// Takes the policy the lifecycle supervisor derived from its runtime
    ///
    /// When ordinary writes may run again, under this or a new configuration, the upload
    /// scheduler is woken once: paused uploads kept their rows Dirty with no timer
    pub(crate) async fn apply_policy(&mut self, published: VersionedPolicy) -> ActorResult<()> {
        if published.version <= self.policy.version {
            debug!("Cloud Backup write lane ignored policy {} after {}", published.version, {
                self.policy.version
            });
            return Produces::ok(());
        }

        let previous = std::mem::replace(&mut self.policy, published);
        let ordinary_resumed = self.policy.policy.runs_ordinary()
            && (!previous.policy.runs_ordinary()
                || previous.policy.generation != self.policy.policy.generation);

        debug!("Cloud Backup write lane applied {:?}", self.policy);
        self.start_next_pending_write();

        if ordinary_resumed && let Some(manager) = self.manager.upgrade() {
            send!(manager.supervisor.ordinary_writes_resumed());
        }

        Produces::ok(())
    }

    /// Reads the lane for the harness invariants, taking the decisions made since the last read
    #[cfg(test)]
    pub(crate) async fn harness_lane_view(&mut self) -> ActorResult<LaneView> {
        Produces::ok(LaneView {
            applied: self.policy.clone(),
            decisions: std::mem::take(&mut self.decisions),
        })
    }

    /// The command running and those queued behind it, for harness tests
    #[cfg(test)]
    pub(crate) async fn harness_queue(&mut self) -> ActorResult<LaneQueue> {
        Produces::ok(LaneQueue {
            in_flight: self.in_flight_write.as_ref().map(|write| write.class.clone()),
            pending: self.pending_writes.iter().map(|write| write.class.clone()).collect(),
        })
    }

    /// The generation and ordinary decision a producer with no operation reads first
    pub(crate) async fn ticket(&mut self) -> ActorResult<WriteTicket> {
        let policy = &self.policy.policy;
        let ordinary = policy.decide(&CommandClass::Ordinary { generation: policy.generation });
        Produces::ok(WriteTicket { generation: policy.generation, ordinary })
    }

    /// Answers once no write is in flight
    ///
    /// A reset uses this after its policy closed the lane: queued commands are decided paused or
    /// stale at the head, so only the write already running can still touch remote data
    pub(crate) async fn when_idle(&mut self) -> ActorResult<oneshot::Receiver<()>> {
        let (sender, receiver) = oneshot::channel();
        self.idle_waiters.push(sender);
        self.answer_idle_waiters();
        Produces::ok(receiver)
    }

    fn answer_idle_waiters(&mut self) {
        if self.in_flight_write.is_some() {
            return;
        }

        for waiter in self.idle_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    fn submit(
        &mut self,
        class: CommandClass,
        remote: CloudBackupRemoteWriteCommand,
        completion: CloudBackupWriteLocalCompletion,
    ) -> CloudBackupWriteResultReceiver<()> {
        let (sender, receiver) = oneshot::channel();
        let context = CloudBackupWriteCommandContext::new(self.next_command_id, class.claim());
        self.next_command_id += 1;

        self.pending_writes.push_back(CloudBackupPendingWrite {
            context,
            class,
            remote,
            completion,
            sender,
        });
        self.start_next_pending_write();

        receiver
    }

    /// Starts the command at the head of the queue if the latest policy runs it
    fn start_next_pending_write(&mut self) {
        if self.in_flight_write.is_some() {
            return;
        }

        while let Some(write) = self.pending_writes.pop_front() {
            let decision = self.policy.policy.decide(&write.class);

            #[cfg(test)]
            self.decisions.push(LaneDecision {
                class: write.class.clone(),
                policy: self.policy.clone(),
                decision,
            });

            match decision {
                Decision::Run => {
                    self.start_pending_write(write);
                    return;
                }
                Decision::Paused(reason) => {
                    debug!("Cloud Backup write lane paused {:?}: {reason:?}", write.class);
                    write.complete(Err(CloudBackupError::WritesPaused(reason)));
                }
                Decision::Stale => {
                    debug!(
                        "Cloud Backup write lane dropped stale {:?} under {:?}",
                        write.class, self.policy
                    );
                    let error = stale_error(&write.class);
                    write.complete(Err(error));
                }
            }
        }

        self.answer_idle_waiters();
    }

    fn start_pending_write(&mut self, write: CloudBackupPendingWrite) {
        let context = write.context;
        let (in_flight, remote) = write.in_flight();
        self.in_flight_write = Some(in_flight);
        if let Some(worker) = &self.worker {
            send!(worker.execute(self.addr.clone(), context, remote));
        }
    }

    /// Whether `class` is still current when its remote work finished
    ///
    /// Remote work that finished for an operation that ended, or under a configuration that was
    /// replaced, reports the stale answer instead of applying its local completion. A pause that
    /// began meanwhile doesn't stop it: the remote write happened, and pausing operations drain
    /// the lane before they act
    fn ensure_current(&self, class: &CommandClass) -> Result<(), CloudBackupError> {
        if self.policy.policy.decide(class) != Decision::Stale {
            return Ok(());
        }

        debug!("Cloud Backup write lane dropped the completion of stale {class:?}");
        Err(stale_error(class))
    }

    pub(crate) async fn complete_remote_write(
        &mut self,
        context: CloudBackupWriteCommandContext,
        result: Result<CloudBackupRemoteWriteResult, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(active) = self.in_flight_write.take() else {
            warn!("Cloud Backup write supervisor received completion for inactive command");
            return Produces::ok(());
        };

        let result = if active.context != context {
            Err(CloudBackupError::Internal(
                format!(
                    "cloud backup write supervisor received mismatched completion for command {:?}",
                    context.id()
                )
                .into(),
            ))
        } else {
            match (self.ensure_current(&active.class), result) {
                (Err(error), _) => Err(error),
                (Ok(()), Ok(output)) => self.apply_local_completion(&active, output).await,
                (Ok(()), Err(error)) => Err(error),
            }
        };

        active.complete(result);
        self.start_next_pending_write();

        Produces::ok(())
    }

    async fn apply_local_completion(
        &self,
        active: &CloudBackupInFlightWrite,
        output: CloudBackupRemoteWriteResult,
    ) -> Result<(), CloudBackupError> {
        let manager = match (&active.completion, &output) {
            (CloudBackupWriteLocalCompletion::None, _) => return Ok(()),
            (
                CloudBackupWriteLocalCompletion::KeychainSave,
                CloudBackupRemoteWriteResult::Keychain(KeychainSave::Saved),
            ) => return Ok(()),
            (
                CloudBackupWriteLocalCompletion::KeychainSave,
                CloudBackupRemoteWriteResult::Keychain(KeychainSave::NamespaceChanged),
            ) => {
                debug!(
                    "Cloud Backup refused keychain writes from {:?} for another namespace",
                    active.class
                );
                return Err(stale_error(&active.class));
            }
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
                send!(manager.supervisor.record_cloud_wallet_count(wallet_count));

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
                // waits on the lifecycle owner; the supervisor never waits on this actor
                call!(manager.supervisor.record_uploaded_wallets(
                    active.context.origin(),
                    wallet_count,
                    state_mode.clone()
                ))
                .await
                .map_err(CloudBackupError::internal)??;

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
                call!(manager.supervisor.record_uploaded_wallets(
                    active.context.origin(),
                    wallet_count,
                    CloudBackupUploadedWalletsStateMode::PreserveVerification
                ))
                .await
                .map_err(CloudBackupError::internal)??;
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
        let uploaded_at = cove_util::time::unix_timestamp_secs_or_zero();
        for wallet in uploaded_wallets {
            manager
                .mark_wallet_uploaded_pending_confirmation_if_revision_current(
                    namespace_id,
                    wallet.wallet_id.clone(),
                    wallet.record_id.clone(),
                    wallet.revision_hash.clone(),
                    uploaded_at,
                )
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn upload_wallet_backup(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        completion: Option<CloudBackupWriteCompletion>,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let completion = completion
            .map_or(CloudBackupWriteLocalCompletion::None, CloudBackupWriteLocalCompletion::Apply);
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::UploadWallet { cloud, namespace, record_id, data },
            completion,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn upload_master_key_backup(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
        completion: Option<CloudBackupWriteCompletion>,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let completion = completion
            .map_or(CloudBackupWriteLocalCompletion::None, CloudBackupWriteLocalCompletion::Apply);
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::UploadMasterKey { cloud, namespace, data },
            completion,
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn delete_active_wallet_backup(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::DeleteActiveWallet {
                cloud,
                namespace,
                record_id: record_id.clone(),
            },
            CloudBackupWriteLocalCompletion::DeleteActiveWallet { record_id },
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn finalize_uploaded_wallets(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        state_mode: CloudBackupUploadedWalletsStateMode,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let fallback_count = uploaded_wallets.len() as u32;
        let receiver = self.submit(
            class,
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
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn complete_uploaded_wallet_batch(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        count_refresh: CloudBackupWalletCountRefresh,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::ListWalletCountOptional {
                cloud,
                namespace_id: namespace_id.clone(),
            },
            CloudBackupWriteLocalCompletion::CompleteUploadedWalletBatch {
                namespace_id,
                uploaded_wallets,
                count_refresh,
            },
        );
        Produces::ok(receiver)
    }

    pub(crate) async fn apply_completion(
        &mut self,
        class: CommandClass,
        completion: CloudBackupWriteCompletion,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::Apply(completion),
        );
        Produces::ok(receiver)
    }

    /// Saves keys a producer proved for `namespace_id`, as the producer's class allows
    pub(crate) async fn save_keychain(
        &mut self,
        class: CommandClass,
        namespace_id: String,
        writes: Vec<OwnedKeychainWrite>,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::SaveKeychain { namespace_id, writes },
            CloudBackupWriteLocalCompletion::KeychainSave,
        );
        Produces::ok(receiver)
    }

    /// Deletes a whole namespace; the class names whose delete it is and decides when it runs
    pub(crate) async fn delete_namespace(
        &mut self,
        class: CommandClass,
        cloud: CloudStorageClient,
        namespace: String,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::DeleteNamespace { cloud, namespace },
            CloudBackupWriteLocalCompletion::None,
        );
        Produces::ok(receiver)
    }

    /// Answers when every command queued before it finished, if its operation still runs
    ///
    /// Turn Off and an account switch drain the lane this way before they read or delete remote
    /// data, so a write that already started can't land after them
    pub(crate) async fn barrier(
        &mut self,
        class: CommandClass,
    ) -> ActorResult<CloudBackupWriteResultReceiver<()>> {
        let receiver = self.submit(
            class,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::None,
        );
        Produces::ok(receiver)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::database::Database;
    use crate::database::cloud_backup::PersistedCloudBackupStatus;
    use crate::manager::cloud_backup_manager::actors::write::{
        ConfiguredWriter, ConfiguredWrites, OffWrites, PolicyBody,
    };
    use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperation;
    use crate::manager::cloud_backup_manager::ops::test_support::{
        async_test_lock, reset_cloud_backup_test_state, test_globals,
    };
    use crate::manager::cloud_backup_manager::owner::operation::test_support::operation_id;
    use crate::manager::cloud_backup_manager::{
        DeepVerificationReport, PendingVerificationCompletion, PendingVerificationUpload,
    };

    fn claim(
        operation: CloudBackupExclusiveOperation,
        id: u64,
    ) -> CloudBackupExclusiveOperationClaim {
        CloudBackupExclusiveOperationClaim::new(operation, operation_id(id))
    }

    fn configured(op: Option<ConfiguredWrites>) -> PolicyBody {
        PolicyBody::Configured { op }
    }

    /// Puts `body` in force under `generation`, as the lifecycle supervisor would publish it
    fn set_policy(
        supervisor: &mut CloudBackupWriteSupervisor,
        generation: ConfigGeneration,
        body: PolicyBody,
    ) {
        let version = supervisor.policy.version + 1;
        supervisor.policy = VersionedPolicy { version, policy: WritePolicy { generation, body } };
    }

    fn submit_none(
        supervisor: &mut CloudBackupWriteSupervisor,
        class: CommandClass,
    ) -> CloudBackupWriteResultReceiver<()> {
        supervisor.submit(
            class,
            CloudBackupRemoteWriteCommand::None,
            CloudBackupWriteLocalCompletion::None,
        )
    }

    fn in_flight_id(supervisor: &CloudBackupWriteSupervisor) -> Option<CloudBackupWriteCommandId> {
        supervisor.in_flight_write.as_ref().map(|write| write.context.id())
    }

    #[test]
    fn write_policy_decides_each_command_class() {
        use CloudBackupExclusiveOperation as Op;

        let generation = ConfigGeneration::next();
        let older = ConfigGeneration::next();
        let turn_off = claim(Op::Disable, 1);
        let delete_row = claim(Op::DeleteCloudWallet, 2);
        let other = claim(Op::DeleteOtherBackups, 3);
        let enable = claim(Op::Enable, 4);
        let discard = claim(Op::DiscardPendingEnable, 5);
        let switch =
            CloudBackupExclusiveOperationClaim::drive_account_switch(7.into(), operation_id(7));
        let other_switch =
            CloudBackupExclusiveOperationClaim::drive_account_switch(8.into(), operation_id(8));
        // the same switch resumed after a reset revoked its earlier run
        let earlier_run =
            CloudBackupExclusiveOperationClaim::drive_account_switch(7.into(), operation_id(9));
        let verify = claim(Op::Verify, 6);
        let reinitialize = claim(Op::ReinitializeBackup, 10);
        let ordinary = CommandClass::Ordinary { generation };
        let cleanup = CommandClass::SourceCleanup { generation };
        let policy = |body| WritePolicy { generation, body };
        let writes = |claim, kind| Some(ConfiguredWrites { claim, kind });

        let cases = [
            // idle and configured: background work runs, nothing op-owned does
            (policy(configured(None)), ordinary.clone(), Decision::Run),
            (policy(configured(None)), cleanup.clone(), Decision::Run),
            (policy(configured(None)), CommandClass::Operation(verify), Decision::Stale),
            (
                policy(configured(None)),
                CommandClass::Ordinary { generation: older },
                Decision::Stale,
            ),
            (
                policy(configured(None)),
                CommandClass::SourceCleanup { generation: older },
                Decision::Stale,
            ),
            // a running verification writes beside background work
            (
                policy(configured(writes(verify, ConfiguredWriter::Other))),
                ordinary.clone(),
                Decision::Run,
            ),
            (
                policy(configured(writes(verify, ConfiguredWriter::Other))),
                CommandClass::Operation(verify),
                Decision::Run,
            ),
            (
                policy(configured(writes(verify, ConfiguredWriter::Other))),
                CommandClass::RowDelete(verify),
                Decision::Stale,
            ),
            // a reinitialize replacing the configuration pauses background work and source
            // cleanup, and writes and drains through its claim
            (
                policy(configured(writes(reinitialize, ConfiguredWriter::Reinitialize))),
                ordinary.clone(),
                Decision::Paused(PauseReason::Reinitialize),
            ),
            (
                policy(configured(writes(reinitialize, ConfiguredWriter::Reinitialize))),
                cleanup.clone(),
                Decision::Paused(PauseReason::Reinitialize),
            ),
            (
                policy(configured(writes(reinitialize, ConfiguredWriter::Reinitialize))),
                CommandClass::Operation(reinitialize),
                Decision::Run,
            ),
            // Turn Off before its commit pauses background work and drains through its claim
            (
                policy(configured(writes(turn_off, ConfiguredWriter::TurnOff))),
                ordinary.clone(),
                Decision::Paused(PauseReason::TurnOff),
            ),
            (
                policy(configured(writes(turn_off, ConfiguredWriter::TurnOff))),
                CommandClass::Operation(turn_off),
                Decision::Run,
            ),
            (
                policy(configured(writes(turn_off, ConfiguredWriter::TurnOff))),
                CommandClass::TurnOffDelete { claim: turn_off, namespace_id: "ns".into() },
                Decision::Stale,
            ),
            // only the committed Turn Off deletes, and only the namespace it committed to
            (
                policy(PolicyBody::FinishingDisable {
                    namespace_id: "ns".into(),
                    op: Some(turn_off),
                }),
                CommandClass::TurnOffDelete { claim: turn_off, namespace_id: "ns".into() },
                Decision::Run,
            ),
            (
                policy(PolicyBody::FinishingDisable {
                    namespace_id: "ns".into(),
                    op: Some(turn_off),
                }),
                CommandClass::TurnOffDelete { claim: turn_off, namespace_id: "other".into() },
                Decision::Stale,
            ),
            (
                policy(PolicyBody::FinishingDisable { namespace_id: "ns".into(), op: None }),
                ordinary.clone(),
                Decision::Paused(PauseReason::TurnOff),
            ),
            // row deletes and other-backup deletes run only under their own operation
            (
                policy(configured(writes(delete_row, ConfiguredWriter::RowDelete))),
                CommandClass::RowDelete(delete_row),
                Decision::Run,
            ),
            (
                policy(configured(writes(delete_row, ConfiguredWriter::RowDelete))),
                CommandClass::OtherBackupDelete(delete_row),
                Decision::Stale,
            ),
            (
                policy(configured(writes(other, ConfiguredWriter::OtherBackups))),
                CommandClass::OtherBackupDelete(other),
                Decision::Run,
            ),
            // off: Enable writes through its claim, a discard deletes only its own namespace
            (
                policy(PolicyBody::Off { op: None }),
                ordinary.clone(),
                Decision::Paused(PauseReason::BackupOff),
            ),
            (
                policy(PolicyBody::Off { op: Some(OffWrites::Enable(enable)) }),
                CommandClass::Operation(enable),
                Decision::Run,
            ),
            (
                policy(PolicyBody::Off { op: Some(OffWrites::Enable(enable)) }),
                CommandClass::EnableDiscard(enable),
                Decision::Stale,
            ),
            (
                policy(PolicyBody::Off { op: Some(OffWrites::Discard(discard)) }),
                CommandClass::EnableDiscard(discard),
                Decision::Run,
            ),
            (
                policy(PolicyBody::Off { op: Some(OffWrites::Restore(enable)) }),
                CommandClass::Operation(enable),
                Decision::Stale,
            ),
            // an account switch pauses background work and runs its own reinitialize
            (
                policy(PolicyBody::AccountSwitch { op: Some(switch) }),
                ordinary.clone(),
                Decision::Paused(PauseReason::AccountSwitch),
            ),
            (
                policy(PolicyBody::AccountSwitch { op: Some(switch) }),
                CommandClass::Operation(switch),
                Decision::Run,
            ),
            (
                policy(PolicyBody::AccountSwitch { op: Some(switch) }),
                CommandClass::Operation(other_switch),
                Decision::Stale,
            ),
            (
                policy(PolicyBody::AccountSwitch { op: Some(switch) }),
                CommandClass::Operation(earlier_run),
                Decision::Stale,
            ),
            (
                policy(PolicyBody::AccountSwitch { op: None }),
                CommandClass::Operation(switch),
                Decision::Stale,
            ),
            // a reset or a failed load closes the lane
            (
                policy(PolicyBody::Closed(PauseReason::LocalReset)),
                ordinary,
                Decision::Paused(PauseReason::LocalReset),
            ),
            (
                policy(PolicyBody::Closed(PauseReason::LocalReset)),
                CommandClass::Operation(verify),
                Decision::Stale,
            ),
            (
                policy(PolicyBody::Closed(PauseReason::LoadFailed)),
                cleanup,
                Decision::Paused(PauseReason::LoadFailed),
            ),
        ];

        for (policy, class, expected) in cases {
            assert_eq!(policy.decide(&class), expected, "{class:?} under {:?}", policy.body);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn paused_background_write_answers_paused_at_the_queue_head_and_the_drain_runs() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        let generation = ConfigGeneration::next();
        let turn_off = claim(CloudBackupExclusiveOperation::Disable, 1);
        set_policy(&mut supervisor, generation, configured(None));

        let running = submit_none(&mut supervisor, CommandClass::Ordinary { generation });
        let queued = submit_none(&mut supervisor, CommandClass::Ordinary { generation });
        let barrier = submit_none(&mut supervisor, CommandClass::Operation(turn_off));
        assert_eq!(in_flight_id(&supervisor), Some(CloudBackupWriteCommandId(0)));

        // Turn Off starts while the first upload is in flight; its pause is decided at the head
        set_policy(
            &mut supervisor,
            generation,
            configured(Some(ConfiguredWrites { claim: turn_off, kind: ConfiguredWriter::TurnOff })),
        );
        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(0, None),
                Ok(CloudBackupRemoteWriteResult::None),
            )
            .await
            .unwrap();

        assert!(running.await.unwrap().into_result().is_ok());
        assert!(matches!(
            queued.await.unwrap().into_result(),
            Err(CloudBackupError::WritesPaused(PauseReason::TurnOff))
        ));
        assert_eq!(in_flight_id(&supervisor), Some(CloudBackupWriteCommandId(2)));

        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(2, Some(turn_off)),
                Ok(CloudBackupRemoteWriteResult::None),
            )
            .await
            .unwrap();
        assert!(barrier.await.unwrap().into_result().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_from_an_older_configuration_is_dropped_before_it_runs() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        let old = ConfigGeneration::next();
        let new = ConfigGeneration::next();
        let reinitialize = claim(CloudBackupExclusiveOperation::ReinitializeBackup, 1);
        set_policy(
            &mut supervisor,
            old,
            configured(Some(ConfiguredWrites {
                claim: reinitialize,
                kind: ConfiguredWriter::Other,
            })),
        );

        let blocking = submit_none(&mut supervisor, CommandClass::Operation(reinitialize));
        let upload = submit_none(&mut supervisor, CommandClass::Ordinary { generation: old });
        let cleanup = submit_none(&mut supervisor, CommandClass::SourceCleanup { generation: old });

        // the reinitialize ends and replaces the configuration before the queued writes run
        set_policy(&mut supervisor, new, configured(None));
        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(0, Some(reinitialize)),
                Ok(CloudBackupRemoteWriteResult::None),
            )
            .await
            .unwrap();

        // the operation's own write finished after it ended, so it reports cancelled
        assert!(matches!(blocking.await.unwrap().into_result(), Err(CloudBackupError::Cancelled)));
        for dropped in [upload, cleanup] {
            assert!(matches!(
                dropped.await.unwrap().into_result(),
                Err(CloudBackupError::WritesPaused(PauseReason::ConfigurationChanged))
            ));
        }
        assert!(supervisor.in_flight_write.is_none());
        assert!(supervisor.pending_writes.is_empty());
    }

    /// An operation that ended while its write was in flight gets Cancelled whatever the remote
    /// answered: the caller is gone, and a cloud error would read as this operation failing
    #[tokio::test(flavor = "current_thread")]
    async fn ended_operation_write_reports_cancelled_over_its_remote_error() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        let generation = ConfigGeneration::next();
        let verify = claim(CloudBackupExclusiveOperation::Verify, 1);
        let writes = ConfiguredWrites { claim: verify, kind: ConfiguredWriter::Other };
        set_policy(&mut supervisor, generation, configured(Some(writes)));

        let write = submit_none(&mut supervisor, CommandClass::Operation(verify));
        set_policy(&mut supervisor, generation, configured(None));
        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(0, Some(verify)),
                Err(CloudBackupError::Offline("the cloud is unreachable".into())),
            )
            .await
            .unwrap();

        assert!(matches!(write.await.unwrap().into_result(), Err(CloudBackupError::Cancelled)));
        assert!(supervisor.in_flight_write.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_operation_write_is_rejected_before_starting_remote_execution() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        set_policy(&mut supervisor, ConfigGeneration::next(), configured(None));
        let stale = claim(CloudBackupExclusiveOperation::Enable, u64::MAX);

        let receiver = submit_none(&mut supervisor, CommandClass::Operation(stale));

        assert!(matches!(receiver.await.unwrap().into_result(), Err(CloudBackupError::Cancelled)));
        assert!(supervisor.in_flight_write.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_completion_after_its_configuration_changed_skips_local_state() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = crate::manager::cloud_backup_manager::initialize_cloud_backup_manager();
        reset_cloud_backup_test_state(&manager, globals);
        let mut supervisor = CloudBackupWriteSupervisor::new(Arc::downgrade(&manager));
        let old = ConfigGeneration::next();
        set_policy(&mut supervisor, old, configured(None));
        let (sender, receiver) = oneshot::channel();
        let context = CloudBackupWriteCommandContext::new(0, None);
        supervisor.in_flight_write = Some(CloudBackupInFlightWrite {
            context,
            class: CommandClass::Ordinary { generation: old },
            completion: CloudBackupWriteLocalCompletion::FinalizeUploadedWallets {
                namespace_id: "namespace".into(),
                uploaded_wallets: Vec::new(),
                state_mode:
                    CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                        pending_completion(),
                    ),
            },
            sender,
        });
        let before = Database::global().cloud_backup_state.get().unwrap();

        set_policy(&mut supervisor, ConfigGeneration::next(), configured(None));
        supervisor
            .complete_remote_write(context, Ok(CloudBackupRemoteWriteResult::WalletCount(2)))
            .await
            .unwrap();

        assert!(matches!(
            receiver.await.unwrap().into_result(),
            Err(CloudBackupError::WritesPaused(PauseReason::ConfigurationChanged))
        ));
        assert_eq!(Database::global().cloud_backup_state.get().unwrap(), before);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn when_idle_answers_after_the_write_in_flight() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        let generation = ConfigGeneration::next();
        set_policy(&mut supervisor, generation, configured(None));
        let _running = submit_none(&mut supervisor, CommandClass::Ordinary { generation });
        let queued = submit_none(&mut supervisor, CommandClass::Ordinary { generation });

        set_policy(&mut supervisor, generation, PolicyBody::Closed(PauseReason::LocalReset));
        let mut idle = supervisor.when_idle().await.unwrap().await.unwrap();
        assert!(idle.try_recv().is_err());

        supervisor
            .complete_remote_write(
                CloudBackupWriteCommandContext::new(0, None),
                Ok(CloudBackupRemoteWriteResult::None),
            )
            .await
            .unwrap();

        assert!(idle.try_recv().is_ok());
        assert!(matches!(
            queued.await.unwrap().into_result(),
            Err(CloudBackupError::WritesPaused(PauseReason::LocalReset))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_older_policy_version_never_replaces_a_newer_one() {
        let mut supervisor = CloudBackupWriteSupervisor::default();
        let generation = ConfigGeneration::next();
        let newer = VersionedPolicy {
            version: 2,
            policy: WritePolicy { generation, body: PolicyBody::Closed(PauseReason::LocalReset) },
        };
        let older = VersionedPolicy {
            version: 1,
            policy: WritePolicy { generation, body: configured(None) },
        };

        supervisor.apply_policy(newer.clone()).await.unwrap();
        supervisor.apply_policy(older).await.unwrap();

        assert_eq!(supervisor.policy, newer);
    }

    fn pending_completion() -> PendingVerificationCompletion {
        PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallet_issues: Default::default(),
                detail: None,
            },
            "namespace".into(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wallet_finalization_persists_enabled_state_and_pending_completion_together() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = crate::manager::cloud_backup_manager::initialize_cloud_backup_manager();
        reset_cloud_backup_test_state(&manager, globals);
        let supervisor = CloudBackupWriteSupervisor::new(Arc::downgrade(&manager));
        let completion = pending_completion();
        let (sender, _receiver) = oneshot::channel();
        let in_flight = CloudBackupInFlightWrite {
            context: CloudBackupWriteCommandContext::new(0, None),
            class: CommandClass::Operation(claim(CloudBackupExclusiveOperation::Enable, 1)),
            completion: CloudBackupWriteLocalCompletion::FinalizeUploadedWallets {
                namespace_id: "namespace".into(),
                uploaded_wallets: Vec::new(),
                state_mode:
                    CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                        completion.clone(),
                    ),
            },
            sender,
        };

        supervisor
            .apply_local_completion(&in_flight, CloudBackupRemoteWriteResult::WalletCount(2))
            .await
            .unwrap();

        let state = Database::global().cloud_backup_state.get().unwrap();
        assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.wallet_count(), Some(2));
        assert_eq!(state.pending_verification_completion(), Some(&completion));
    }

    impl CloudBackupWriteSupervisor {
        /// Commands waiting behind the write in flight
        pub(crate) async fn queued_writes_for_test(&mut self) -> ActorResult<usize> {
            Produces::ok(self.pending_writes.len())
        }
    }
}
