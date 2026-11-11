//! Top-level Cloud Backup operation supervisor
//!
//! This actor owns exclusive operation lifecycles and delegates slow work to
//! child actors or spawned tasks. Each exclusive operation receives a claim that
//! every async completion must present before it can update manager state

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
#[cfg(test)]
use std::time::Duration;

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, call, send};
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_tokio::task::spawn_actor;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

use super::CloudBackupSyncHealthWorker;
use super::cleanup::{CleanupSourceNamespace, CloudBackupCleanupJob, CloudBackupCleanupWorker};
use super::restore::{self, CloudBackupRestoreEvent, RestoreOperation, RestoredNamespaceCommit};
use super::uploads::CloudBackupUploadWorker;
use super::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteClient,
    CloudBackupWriteCompletion, CloudBackupWriteSupervisor, WriteOwner,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudStorageIssue, DriveAccountSwitchId, PersistedCloudBackupState,
    PersistedCloudBlobSyncState, PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase,
};
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::admission::{
    Admission, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::keychain::{CloudBackupKeychain, LocalStateOwner};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::LifecycleWriter;
use crate::manager::cloud_backup_manager::owner::operation::OperationId;
use crate::manager::cloud_backup_manager::verify::coordinator::CloudBackupVerificationCoordinator;
use crate::manager::cloud_backup_manager::verify::{
    CloudBackupDeepVerificationAutoSyncCompletion, CloudBackupDeepVerificationStep,
    CloudBackupPasskeyRepairFinalization, CloudBackupPendingDeepVerificationAutoSyncResume,
    CloudBackupPendingDeepVerificationResume, CloudBackupPreparedDeepVerificationAutoSync,
    CloudBackupPreparedDeepVerificationWrapperRepair, CloudBackupPreparedPasskeyWrapperRepair,
    CloudBackupUploadedDeepVerificationAutoSync, CloudBackupUploadedPasskeyWrapperRepair,
};
use crate::manager::cloud_backup_manager::wallets::{
    CloudOnlyRestoreOutcome, UnpersistedPrfKey, delay_before_new_passkey_auth,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_LABELS_WARNING_MESSAGE, CloudBackupCloudOnlyFetchOutcome,
    CloudBackupCloudOnlyOperationWarning, CloudBackupCloudOnlyWalletOutcome,
    CloudBackupDetailInventorySnapshot, CloudBackupDetailInventorySnapshotResult,
    CloudBackupDetailOutcome, CloudBackupDetailProviderConfirmation, CloudBackupDetailResult,
    CloudBackupDetailSnapshotCompletion, CloudBackupDisableOutcome, CloudBackupDisablePreparation,
    CloudBackupDriveAccountSwitchError, CloudBackupEnableContext,
    CloudBackupEnablePasskeyPreparation, CloudBackupEnablePasskeyRegistration,
    CloudBackupEnablePreparation, CloudBackupEnableRecoveryCompletion,
    CloudBackupEnableRecoveryPreparation, CloudBackupEnableState, CloudBackupError,
    CloudBackupInventoryAuthority, CloudBackupInventoryIncompleteReason,
    CloudBackupKeepEnabledPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupOtherBackupsOutcome, CloudBackupOtherBackupsState, CloudBackupPasskeyChoiceIntent,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
    CloudBackupPreparedCloudWalletDelete, CloudBackupPreparedRestoreAll,
    CloudBackupPreparedUndecryptableWalletDeletion, CloudBackupReadyEnableUpload,
    CloudBackupRegisteredEnablePasskey, CloudBackupRestoreAllState, CloudBackupRestoreOutcome,
    CloudBackupRestoreReport, CloudBackupReuploadedWallets, CloudBackupSavedPasskeyConfirmation,
    CloudBackupStatus, CloudBackupUndecryptableWalletDeletionState,
    CloudBackupUploadedEnableBackup, CloudBackupVerificationPresentation,
    CloudBackupVerificationSource, CloudBackupWalletItem, CloudOnlyOperation, CloudReachable,
    DeepVerificationReport, DeepVerificationResult, DriveAccountSwitchPlatformState,
    DriveAccountSwitchReconcileAction, EnablePasskeyRegistrationFlow,
    GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, OtherBackupsOperation, PendingEnableJournal,
    PendingEnableJournalPhase, PendingEnableLocalMetadataSnapshot, PendingEnableNamespaceOwnership,
    PendingEnableRun, PendingEnableSession, PendingUploadVerificationState,
    PendingVerificationCompletion, PendingVerificationUpload, RecoveryAction, RecoveryState,
    RustCloudBackupManager, SavedPasskeyConfirmationMode, SyncState, VerificationState, WalletId,
    blocking_cloud_error, is_provider_wide_interruption,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

mod account_switch;
mod cloud_only;
mod detail_workflow;
mod disable;
mod dispatch;
mod enable;
mod lifecycle_results;
mod other_backups;
mod owned_runtime;
mod restore_all;
mod runtime;

use owned_runtime::OwnedRuntime;
#[cfg(test)]
pub(crate) use owned_runtime::{OpTransition, OwnerView};
use runtime::{Effect, IdleFacts, ResetStart, RestoreRun, Runtime};
mod undecryptable_wallets;
mod verification;

pub(crate) use dispatch::DispatchOutcome;
pub(crate) use lifecycle_results::OwnedKeychainWrite;
pub(crate) use verification::DeepVerificationContinuation;

use detail_workflow::{DetailRefreshClaim, DetailRefreshPlan, DetailResultClaim, DetailWorkflow};
use restore_all::restore_all_marker_matches_active_namespace;

mod tests {
    #![cfg(test)]

    include!("supervisor/tests.rs");
}

/// Proof that the lifecycle owner is changing Cloud Backup's local keychain state
///
/// Only the supervisor and its submodules can mint one: in a handler, or for an `Effecting`
/// stage it started. Together with the recovery-cleanup permit, which can only exist while no
/// supervisor does, it gives the keychain namespace one owner at any time
#[derive(Debug)]
pub(crate) struct OwnerToken(());

impl OwnerToken {
    fn mint() -> Self {
        Self(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveAccountSwitchReinitializationCompletion {
    NotDriveAccountSwitch,
    Stale,
    Handled,
}

/// Passkey proof cached only for the current supervisor session
///
/// The cache lets detail entry reuse fresh authorization after enable or repair,
/// but it is intentionally lost on restart so passkey availability is checked
/// again through the platform
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RuntimePasskeyAuthorization {
    namespace_id: String,
    credential_id: Vec<u8>,
    prf_salt: [u8; 32],
}

impl std::fmt::Debug for RuntimePasskeyAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimePasskeyAuthorization")
            .field("namespace_id", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .finish()
    }
}

#[derive(Debug)]
enum DetailEntryPlan {
    RefreshOnly,
    ResumePendingUploadConfirmation(PendingVerificationCompletion),
    UseFreshEnableProof(RuntimePasskeyAuthorization),
    ContinueRustOwnedVerification,
    StartPasskeyVerification { force_discoverable: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OtherBackupsScanClaim(u64);

/// Refresh attempt kind used to avoid retry loops on connectivity failures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailRefreshAttempt {
    Initial,
    AutomaticConnectivityRetry,
}

/// Verification attempt kind used to avoid retry loops on connectivity failures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationAttempt {
    Initial,
    AutomaticConnectivityRetry,
}

fn should_retry_connectivity_failure(status: ConnectivityStatus) -> bool {
    matches!(status, ConnectivityStatus::Unknown | ConnectivityStatus::Connected)
}

fn apply_refresh_detail_result(manager: &RustCloudBackupManager, result: &CloudBackupDetailResult) {
    match result {
        CloudBackupDetailResult::SuccessWithAuthority { listed, authority } => {
            manager.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithAuthority {
                listed: listed.clone(),
                authority: *authority,
            });
        }
        CloudBackupDetailResult::AccessError(error) => {
            error!("Failed to refresh detail: {error}");
            manager.apply_detail_outcome(CloudBackupDetailOutcome::Failed {
                reason: CloudBackupInventoryIncompleteReason::from(CloudStorageIssue::from(error)),
                error: error.reader_message(),
            });
        }
    }
}

fn refresh_detail_needs_connectivity_retry(
    manager: &RustCloudBackupManager,
    attempt: DetailRefreshAttempt,
    result: &Option<CloudBackupDetailResult>,
) -> bool {
    if attempt != DetailRefreshAttempt::Initial {
        return false;
    }

    let Some(result) = result else { return false };
    result.is_connectivity_access_error()
        && should_retry_connectivity_failure(manager.connection_status())
}

fn verification_needs_connectivity_retry(
    manager: &RustCloudBackupManager,
    attempt: VerificationAttempt,
    result: &DeepVerificationResult,
) -> bool {
    if attempt != VerificationAttempt::Initial {
        return false;
    }

    matches!(result, DeepVerificationResult::Failed(failure) if failure.is_connectivity_retry())
        && should_retry_connectivity_failure(manager.connection_status())
}

/// Which local wallet set a verification checked
///
/// Process-local, like `OperationId`: verification evidence is only valid for the wallet set it
/// read, and a wallet change that lands while a verification runs makes that evidence stale
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WalletSetGeneration(u64);

impl WalletSetGeneration {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// How a user's sync request ended
///
/// A pause is not a failure. The write lane stops the sync's writes while Turn Off, a
/// reinitialize, an account switch or a reset holds the backup, and the wallets it didn't reach
/// stay unsynced for the next sync, so the sync shows idle instead of an error
#[derive(Debug)]
pub(crate) enum SyncRequestOutcome {
    Synced,
    Paused(PauseReason),
    Failed(CloudBackupError),
}

impl From<Result<(), CloudBackupError>> for SyncRequestOutcome {
    fn from(result: Result<(), CloudBackupError>) -> Self {
        match result {
            Ok(()) => Self::Synced,
            Err(CloudBackupError::WritesPaused(reason)) => Self::Paused(reason),
            Err(error) => Self::Failed(error),
        }
    }
}

/// Actor that owns Cloud Backup operation exclusivity, async completions and the durable
/// lifecycle record
///
/// The supervisor is the lifecycle owner: it holds the only [`LifecycleWriter`], so every
/// durable lifecycle write runs in one of its handlers. Producers off the actor (verification,
/// the write lane, pending upload confirmation, platform calls) send it a typed message with
/// their result and claim, and the supervisor applies the write and reconciles
///
/// Waiting calls go one way only. The write supervisor and other workers may `call!` the
/// supervisor and wait for the write's outcome, but the supervisor never waits on the write
/// supervisor: it `send!`s and handles the reply as a later completion message, matched with
/// `Runtime::accepts_completion`. A wait in both directions would deadlock, because
/// each actor runs one handler at a time
#[derive(Debug)]
pub(crate) struct CloudBackupSupervisor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    cleanup: Addr<CloudBackupCleanupWorker>,
    sync_health: Addr<CloudBackupSyncHealthWorker>,
    uploads: Addr<CloudBackupUploadWorker>,
    write: Addr<CloudBackupWriteSupervisor>,
    /// The lifecycle body with the running operation nested in it, and the only lifecycle
    /// writer, both published to the write lane
    runtime: OwnedRuntime,
    pending_enable_session: Option<PendingEnableSession>,
    next_request_id: u64,
    active_sync_request: Option<u64>,
    active_cloud_only_fetch_request: Option<u64>,
    active_other_backups_scan: Option<OtherBackupsScanClaim>,
    detail_workflow: DetailWorkflow,
    /// Counts local wallet-set changes this process has seen
    wallet_set: WalletSetGeneration,
    last_outcome: Option<DispatchOutcome>,
    /// What the harness invariants read about dispatch since their last check
    #[cfg(test)]
    invariant_log: test_support::DispatchLog,
}

#[async_trait::async_trait]
impl Actor for CloudBackupSupervisor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupSupervisor {
    fn commit_restored_namespace(
        manager: &RustCloudBackupManager,
        lifecycle: &LifecycleWriter,
        commit: RestoredNamespaceCommit,
    ) -> Result<(), CloudBackupError> {
        let RestoredNamespaceCommit { master_key, passkey, namespace_id, state, wallet_ids } =
            commit;
        let snapshot = CloudBackupKeychain::global()
            .capture_restore_activation_snapshot()
            .map_err(|error| {
                CloudBackupError::internal_context("capture restore activation keychain", error)
            })?;

        let changed_at = cove_util::time::unix_timestamp_secs_or_zero();
        let dirty_states = wallet_ids
            .into_iter()
            .map(|wallet_id| {
                PersistedCloudBlobSyncState::dirty_wallet(
                    namespace_id.clone(),
                    wallet_id,
                    changed_at,
                )
            })
            .collect::<Vec<_>>();

        restore::save_restore_keychain_entries(&snapshot, master_key, passkey, namespace_id)?;

        // keychain, configured state, and dirty-wallet rows are one restore commit
        if let Err(source) = lifecycle.activate_restored_namespace(&state, &dirty_states) {
            let original =
                CloudBackupError::internal_context("persist restored namespace activation", source);

            return Err(restore::rollback_restore_activation_keychain(&snapshot, original));
        }

        manager.reconcile_persisted_state(&state);
        manager.refresh_sync_health();

        Ok(())
    }

    pub(crate) fn new(
        manager: Weak<RustCloudBackupManager>,
        cloud_writes: Addr<CloudBackupWriteSupervisor>,
        lifecycle: LifecycleWriter,
        load_error: Option<DatabaseError>,
    ) -> Self {
        let runtime = Runtime::at_load(
            load_error,
            IdleFacts {
                record: &RustCloudBackupManager::load_persisted_state(),
                pending_enable_recovery: None,
            },
        );

        let runtime = OwnedRuntime::new(runtime, lifecycle, cloud_writes.clone(), manager.clone());

        Self {
            addr: WeakAddr::default(),
            cleanup: spawn_actor(CloudBackupCleanupWorker::new(manager.clone())),
            sync_health: spawn_actor(CloudBackupSyncHealthWorker::new(manager.clone())),
            uploads: spawn_actor(CloudBackupUploadWorker::new(manager.clone())),
            write: cloud_writes,
            manager,
            runtime,
            pending_enable_session: None,
            next_request_id: 0,
            active_sync_request: None,
            active_cloud_only_fetch_request: None,
            active_other_backups_scan: None,
            detail_workflow: DetailWorkflow::default(),
            wallet_set: WalletSetGeneration::default(),
            last_outcome: None,
            #[cfg(test)]
            invariant_log: test_support::DispatchLog::default(),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    /// The manager, only while `claim` still owns the active operation
    ///
    /// A dropped manager also ends the operation, so its claim is cleared here instead of at
    /// every completion handler
    fn current(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Option<Arc<RustCloudBackupManager>> {
        if !self.runtime.accepts_completion(claim, "completion") {
            return None;
        }

        let manager = self.manager();
        if manager.is_none() {
            self.runtime.clear();
        }

        manager
    }

    fn addr(&self) -> Addr<Self> {
        self.addr.upgrade()
    }

    async fn delete_prepared_cloud_wallet_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        prepared: CloudBackupPreparedCloudWalletDelete,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        let writes = CloudBackupWriteClient::for_operation(write, origin);
        writes
            .delete_active_wallet_backup(
                prepared.cloud,
                prepared.namespace,
                prepared.record_id.clone(),
            )
            .await
            .map_err(|error| {
                let error = match error {
                    CloudBackupError::CloudStorage(source) => {
                        CloudBackupError::cloud_storage_context("delete wallet backup", source)
                    }
                    error => error,
                };

                blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error)
            })?;

        info!("Deleted cloud wallet");
        Ok(())
    }

    async fn apply_cloud_backup_write_completion_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::for_operation(write, origin).apply_completion(completion).await
    }

    fn begin_exclusive_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        operation: CloudBackupExclusiveOperation,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if self.runtime.is_some() {
            return None;
        }

        let claim = CloudBackupExclusiveOperationClaim::new(operation, OperationId::next());
        let started = if operation == CloudBackupExclusiveOperation::Restore {
            self.runtime.start_restore(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))))
        } else {
            self.runtime.start_standard(claim)
        };
        if let Err(unstartable) = started {
            warn!("Cloud Backup could not start {unstartable:?}");
            return None;
        }

        manager.project_exclusive_operation_started(claim);
        Some(claim)
    }

    /// Moves the active exclusive slot from `from` to a new claim for `operation`
    ///
    /// The slot is never free in between, so no other operation can start, and completions that
    /// still carry `from` are stale from then on
    fn hand_off_exclusive_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        from: CloudBackupExclusiveOperationClaim,
        operation: CloudBackupExclusiveOperation,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if !self.runtime.is_current(from) {
            return None;
        }

        let to = CloudBackupExclusiveOperationClaim::new(operation, OperationId::next());
        if let Err(unstartable) = self.runtime.hand_off(from, to) {
            warn!("Cloud Backup could not hand off to {unstartable:?}");
            return None;
        }

        manager.project_exclusive_operation_handed_off(from, to);
        Some(to)
    }

    fn begin_restore_all_exclusive_operation(
        &mut self,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if self.runtime.is_some() {
            return None;
        }

        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
            OperationId::next(),
        );
        if let Err(unstartable) =
            self.runtime.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))))
        {
            warn!("Cloud Backup could not start {unstartable:?}");
            return None;
        }

        Some(claim)
    }

    /// Shows the runtime through the read-only adapter onto today's exported types
    ///
    /// Bodies the reducer can't tell apart from the record alone, such as `LoadFailed` after a
    /// failed read, get their exported shape here, so a load failure never shows as Disabled
    pub async fn project_runtime(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let exported = self.runtime.exported();
        if let Some(status) = exported.status {
            manager.reconcile_runtime_status(status);
        }
        if let Some(disable) = exported.disable {
            manager.apply_disable_outcome(disable);
        }

        Produces::ok(())
    }

    fn next_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        request_id
    }

    pub async fn complete_exclusive_operation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<()> {
        if !self.runtime.accepts_completion(claim, "completion") {
            return Produces::ok(());
        }

        // a finished restore configures the backup, and the write lane wakes uploads when the
        // published policy lets them run
        self.runtime.clear();
        if let Some(manager) = self.manager() {
            manager.project_exclusive_operation_finished(claim);
        }

        Produces::ok(())
    }

    pub async fn fail_exclusive_operation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) -> ActorResult<()> {
        if !self.runtime.accepts_completion(claim, "failure") {
            return Produces::ok(());
        }

        self.runtime.clear();
        if let Some(manager) = self.manager() {
            manager.project_exclusive_operation_failed(claim, &error);
        }

        Produces::ok(())
    }

    fn restore_operation_is_current(&self, claim: CloudBackupExclusiveOperationClaim) -> bool {
        let Some(run) = self.runtime.restore(claim) else { return false };

        !run.cancellation.load(Ordering::Acquire)
    }

    /// The cancellation flag of a restore claim that was just started
    fn restore_cancellation(&self, claim: CloudBackupExclusiveOperationClaim) -> Arc<AtomicBool> {
        self.runtime
            .restore(claim)
            .map(|run| run.cancellation.clone())
            .expect("restore claim must carry a cancellation state")
    }

    pub async fn ensure_restore_current(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if self.restore_operation_is_current(claim) {
            Produces::ok(Ok(()))
        } else {
            Produces::ok(Err(CloudBackupError::Cancelled))
        }
    }

    pub async fn apply_restore_status(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        status: CloudBackupStatus,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        manager.reconcile_runtime_status(status);
        Produces::ok(Ok(()))
    }

    pub async fn apply_restore_outcome(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        outcome: CloudBackupRestoreOutcome,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        manager.apply_restore_outcome(outcome);
        Produces::ok(Ok(()))
    }

    pub async fn clear_restore_enable_progress(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        manager.clear_enable_progress_report();
        Produces::ok(Ok(()))
    }

    pub async fn record_restore_without_backup(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        let result = self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "persist empty restored cloud backup state",
                lifecycle.record_restore_without_backup(),
            )
        });

        Produces::ok(result)
    }

    pub async fn commit_restore_namespace_activation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        commit: RestoredNamespaceCommit,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        Produces::ok(
            self.runtime
                .commit(|lifecycle| Self::commit_restored_namespace(&manager, lifecycle, commit)),
        )
    }

    pub async fn start_enable_operation(
        &mut self,
        context: CloudBackupEnableContext,
    ) -> ActorResult<()> {
        self.begin_enable_operation(context);
        Produces::ok(())
    }

    pub async fn start_enable_force_new_operation(
        &mut self,
        context: CloudBackupEnableContext,
    ) -> ActorResult<()> {
        self.begin_enable_force_new_operation(context);
        Produces::ok(())
    }

    pub async fn start_enable_no_discovery_operation(
        &mut self,
        context: CloudBackupEnableContext,
    ) -> ActorResult<()> {
        self.begin_enable_no_discovery_operation(context);
        Produces::ok(())
    }

    pub async fn start_recovery_operation(&mut self, action: RecoveryAction) -> ActorResult<()> {
        self.begin_recovery_operation(action);
        Produces::ok(())
    }

    pub async fn start_repair_passkey_operation(&mut self, no_discovery: bool) -> ActorResult<()> {
        self.begin_repair_passkey_operation(no_discovery);
        Produces::ok(())
    }

    pub async fn start_sync_operation(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.start_sync_request(manager);
        Produces::ok(())
    }

    pub async fn start_disable_operation(&mut self) -> ActorResult<()> {
        self.begin_disable_operation();
        Produces::ok(())
    }

    pub async fn start_restore_cloud_wallet_operation(
        &mut self,
        record_id: String,
    ) -> ActorResult<()> {
        self.begin_restore_cloud_wallet_operation(record_id);
        Produces::ok(())
    }

    pub async fn start_restore_all_operation(&mut self, retry: bool) -> ActorResult<()> {
        self.begin_restore_all_operation(retry);
        Produces::ok(())
    }

    pub async fn cancel_restore_all_operation(&mut self) -> ActorResult<()> {
        self.request_restore_all_cancellation();
        Produces::ok(())
    }

    pub async fn start_delete_cloud_wallet_operation(
        &mut self,
        record_id: String,
    ) -> ActorResult<()> {
        self.begin_delete_cloud_wallet_operation(record_id);
        Produces::ok(())
    }

    pub async fn start_delete_undecryptable_wallet_backups_operation(&mut self) -> ActorResult<()> {
        self.begin_delete_undecryptable_wallet_backups_operation();
        Produces::ok(())
    }

    fn start_sync_request(&mut self, manager: Arc<RustCloudBackupManager>) {
        let request_id = self.next_request_id();
        self.active_sync_request = Some(request_id);
        manager.apply_sync_state(SyncState::Syncing);

        self.addr.send_fut_with(move |addr| async move {
            let outcome = manager.do_sync_unsynced_wallets().await.into();
            send!(addr.complete_sync_request(request_id, outcome));
        });
    }

    pub async fn complete_sync_request(
        &mut self,
        request_id: u64,
        outcome: SyncRequestOutcome,
    ) -> ActorResult<()> {
        if self.active_sync_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_sync_request = None;
            return Produces::ok(());
        };

        match outcome {
            SyncRequestOutcome::Synced => {
                manager.refresh_sync_health();
                let detail_claim = self.detail_workflow.start_operation_result();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_sync_request_refresh_detail(
                        request_id,
                        detail_claim,
                        result
                    ));
                });
            }

            SyncRequestOutcome::Paused(reason) => {
                info!("Cloud backup sync paused: {reason:?}");
                manager.apply_sync_state(SyncState::Idle);
                self.active_sync_request = None;
            }

            SyncRequestOutcome::Failed(error) => {
                manager.apply_sync_state(SyncState::Failed(
                    error.logged_reader_message("Cloud backup sync failed"),
                ));
                self.active_sync_request = None;
            }
        }

        Produces::ok(())
    }

    pub async fn complete_sync_request_refresh_detail(
        &mut self,
        request_id: u64,
        detail_claim: DetailResultClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        if self.active_sync_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_sync_request = None;
            return Produces::ok(());
        };

        if self.detail_workflow.is_latest_result(detail_claim)
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, &result);
        }

        manager.apply_sync_state(SyncState::Idle);
        self.active_sync_request = None;
        Produces::ok(())
    }

    pub async fn start_refresh_detail(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.detail_workflow.open();
        let plan = self.detail_workflow.request_refresh();
        self.handle_detail_refresh_plan(manager, plan);

        Produces::ok(())
    }

    pub async fn provider_inventory_did_change(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else {
            return Produces::ok(());
        };
        if !self.detail_workflow.is_open() {
            return Produces::ok(());
        }

        self.ensure_supplemental_inventory_discovery(manager.clone());
        if manager.detail_inventory_is_complete() {
            return Produces::ok(());
        }

        let plan = self.detail_workflow.request_refresh();
        self.handle_detail_refresh_plan(manager, plan);

        Produces::ok(())
    }

    pub async fn local_wallets_did_change(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else {
            return Produces::ok(());
        };
        manager.apply_local_wallets_changed();

        // the shown detail sorts wallets by which ones are local, so it is stale as well
        if self.detail_workflow.is_open() {
            let plan = self.detail_workflow.request_refresh();
            self.handle_detail_refresh_plan(manager, plan);
        }

        Produces::ok(())
    }

    pub async fn start_background_inventory_discovery(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        if !matches!(manager.state.read().status(), CloudBackupStatus::Enabled) {
            return Produces::ok(());
        }

        self.detail_workflow.open();
        self.ensure_supplemental_inventory_discovery(manager.clone());
        if !manager.detail_inventory_is_ready() {
            let plan = self.detail_workflow.request_refresh();
            self.handle_detail_refresh_plan(manager, plan);
        }

        Produces::ok(())
    }

    pub async fn start_detail_result_claim(&mut self) -> ActorResult<DetailResultClaim> {
        Produces::ok(self.detail_workflow.start_operation_result())
    }

    fn schedule_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) {
        manager.refresh_sync_health();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.load_cloud_backup_detail_inventory_snapshot().await;
            send!(addr.complete_refresh_detail_snapshot(result, attempt, claim));
        });
    }

    fn schedule_complete_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        snapshot: CloudBackupDetailInventorySnapshot,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let completion =
                manager.complete_cloud_backup_detail_inventory_snapshot(snapshot).await;
            send!(addr.complete_refresh_detail_from_snapshot(completion, attempt, claim));
        });
    }

    fn schedule_confirm_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        confirmation: CloudBackupDetailProviderConfirmation,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.confirm_cloud_backup_detail_inventory(confirmation.clone()).await;
            send!(addr.complete_confirm_refresh_detail(confirmation, result, attempt, claim));
        });
    }

    fn handle_detail_refresh_plan(
        &self,
        manager: Arc<RustCloudBackupManager>,
        plan: DetailRefreshPlan,
    ) {
        match plan {
            DetailRefreshPlan::Start(claim) => {
                manager.apply_detail_outcome(CloudBackupDetailOutcome::Checking);
                self.schedule_refresh_detail(manager, DetailRefreshAttempt::Initial, claim);
            }
            DetailRefreshPlan::Wait { owner, delay } => {
                self.addr.send_fut_with(move |addr| async move {
                    tokio::time::sleep(delay).await;
                    send!(addr.resume_rate_limited_detail_refresh(owner));
                });
            }
            DetailRefreshPlan::Queued | DetailRefreshPlan::Ignored => {}
        }
    }

    pub async fn resume_rate_limited_detail_refresh(&mut self, owner: u64) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let plan = self.detail_workflow.timer_elapsed(owner);
        self.handle_detail_refresh_plan(manager, plan);

        Produces::ok(())
    }

    pub async fn complete_refresh_detail(
        &mut self,
        result: Option<CloudBackupDetailResult>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if !self.detail_workflow.is_refresh_active(claim) {
            return Produces::ok(());
        }

        if refresh_detail_needs_connectivity_retry(&manager, attempt, &result) {
            self.schedule_refresh_detail(
                manager,
                DetailRefreshAttempt::AutomaticConnectivityRetry,
                claim,
            );
            return Produces::ok(());
        }

        let completion = self.detail_workflow.complete_refresh(claim);
        if completion.apply
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, &result);
        }

        self.ensure_supplemental_inventory_discovery(manager.clone());

        self.handle_detail_refresh_plan(manager, completion.next);

        Produces::ok(())
    }

    pub async fn complete_refresh_detail_from_snapshot(
        &mut self,
        completion: Option<CloudBackupDetailSnapshotCompletion>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if !self.detail_workflow.is_refresh_active(claim) {
            return Produces::ok(());
        }

        match completion {
            None => self.complete_refresh_detail(None, attempt, claim).await,
            Some(CloudBackupDetailSnapshotCompletion::Final(result)) => {
                self.complete_refresh_detail(Some(result), attempt, claim).await
            }
            Some(CloudBackupDetailSnapshotCompletion::TrustedLocal { listed, confirmation }) => {
                if self.detail_workflow.is_latest_refresh(claim) {
                    manager.apply_detail_outcome(
                        CloudBackupDetailOutcome::RefreshedWithAuthority {
                            listed,
                            authority:
                                CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
                        },
                    );
                }

                self.ensure_supplemental_inventory_discovery(manager.clone());
                self.schedule_confirm_refresh_detail(manager, confirmation, attempt, claim);

                Produces::ok(())
            }
        }
    }

    pub async fn complete_confirm_refresh_detail(
        &mut self,
        confirmation: CloudBackupDetailProviderConfirmation,
        result: Option<CloudBackupDetailResult>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if !self.detail_workflow.is_refresh_active(claim) {
            return Produces::ok(());
        }

        if refresh_detail_needs_connectivity_retry(&manager, attempt, &result) {
            self.schedule_confirm_refresh_detail(
                manager,
                confirmation,
                DetailRefreshAttempt::AutomaticConnectivityRetry,
                claim,
            );
            return Produces::ok(());
        }

        let completion = self.detail_workflow.complete_refresh(claim);
        if completion.apply
            && let Some(result) = result
        {
            // the local rows stay retained, but an unconfirmed inventory is incomplete so the user
            // sees why restore and destructive actions are unavailable and can check again
            apply_refresh_detail_result(&manager, &result);
        }

        self.ensure_supplemental_inventory_discovery(manager.clone());
        self.handle_detail_refresh_plan(manager, completion.next);

        Produces::ok(())
    }

    pub async fn complete_refresh_detail_snapshot(
        &mut self,
        result: Option<CloudBackupDetailInventorySnapshotResult>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if !self.detail_workflow.is_refresh_active(claim) {
            return Produces::ok(());
        }

        let Some(result) = result else {
            let completion = self.detail_workflow.complete_refresh(claim);
            self.handle_detail_refresh_plan(manager, completion.next);
            return Produces::ok(());
        };

        match result {
            CloudBackupDetailInventorySnapshotResult::Success(snapshot) => {
                // retain locally known rows while the authoritative inventory check completes
                if self.detail_workflow.is_latest_refresh(claim)
                    && let Some(provisional_detail) = snapshot.provisional_detail.clone()
                {
                    manager.apply_detail_outcome(CloudBackupDetailOutcome::Provisional(
                        provisional_detail,
                    ));
                }

                self.schedule_complete_refresh_detail(manager, snapshot, attempt, claim);
            }
            CloudBackupDetailInventorySnapshotResult::AccessError(error) => {
                let result = Some(CloudBackupDetailResult::AccessError(error));
                if refresh_detail_needs_connectivity_retry(&manager, attempt, &result) {
                    self.schedule_refresh_detail(
                        manager,
                        DetailRefreshAttempt::AutomaticConnectivityRetry,
                        claim,
                    );
                    return Produces::ok(());
                }

                let completion = self.detail_workflow.complete_refresh(claim);
                if completion.apply
                    && let Some(result) = result
                {
                    apply_refresh_detail_result(&manager, &result);
                }
                self.ensure_supplemental_inventory_discovery(manager.clone());
                self.handle_detail_refresh_plan(manager, completion.next);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        detail_claim: DetailResultClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        if self.detail_workflow.is_latest_result(detail_claim)
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, &result);
        }

        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn start_enter_detail(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.detail_workflow.open();
        self.ensure_supplemental_inventory_discovery(manager.clone());
        let plan = self.detail_workflow.entry_plan(&manager);
        match plan {
            DetailEntryPlan::StartPasskeyVerification { force_discoverable } => {
                self.begin_manual_verification(manager, force_discoverable);
                return Produces::ok(());
            }
            DetailEntryPlan::UseFreshEnableProof(authorization) => {
                debug_assert_eq!(
                    manager.current_namespace_id().ok().as_deref(),
                    Some(authorization.namespace_id.as_str())
                );
            }
            DetailEntryPlan::ResumePendingUploadConfirmation(completion) => {
                debug_assert!(!completion.uploads().is_empty());
            }
            DetailEntryPlan::ContinueRustOwnedVerification => {}
            DetailEntryPlan::RefreshOnly => {}
        }

        let plan = self.detail_workflow.request_refresh();
        self.handle_detail_refresh_plan(manager, plan);

        Produces::ok(())
    }

    fn ensure_supplemental_inventory_discovery(&mut self, manager: Arc<RustCloudBackupManager>) {
        if !matches!(manager.state.read().status(), CloudBackupStatus::Enabled) {
            return;
        }

        if self.active_other_backups_scan.is_none()
            && matches!(
                manager.state.read().other_backups_state(),
                CloudBackupOtherBackupsState::NotChecked
                    | CloudBackupOtherBackupsState::LoadFailed { .. }
            )
        {
            self.schedule_other_backups_scan(manager.clone());
        }

        self.begin_cloud_only_fetch_if_needed(manager);
    }

    fn schedule_other_backups_scan(&mut self, manager: Arc<RustCloudBackupManager>) {
        let addr = self.addr();
        let claim = OtherBackupsScanClaim(self.next_request_id());
        self.active_other_backups_scan = Some(claim);

        manager.apply_other_backups_state(CloudBackupOtherBackupsState::Checking);
        let scan_manager = manager;
        addr.send_fut_with(move |addr| async move {
            let cloud = CloudStorage::global_silent_client();
            let state = scan_manager.other_backup_state(&cloud).await;
            send!(addr.complete_other_backups_scan(claim, state));
        });
    }

    async fn complete_other_backups_scan(
        &mut self,
        claim: OtherBackupsScanClaim,
        state: CloudBackupOtherBackupsState,
    ) -> ActorResult<()> {
        if self.active_other_backups_scan == Some(claim)
            && let Some(manager) = self.manager()
        {
            self.active_other_backups_scan = None;
            manager.apply_other_backups_state(state);
        }

        Produces::ok(())
    }

    pub async fn start_master_key_upload_confirmation_grace(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        call!(self.sync_health.start_master_key_upload_confirmation_grace(namespace_id)).await?;
        Produces::ok(())
    }

    pub async fn request_sync_health_refresh(&mut self) -> ActorResult<()> {
        call!(self.sync_health.request_sync_health_refresh()).await?;
        Produces::ok(())
    }

    pub async fn cache_pending_verification_completion(
        &mut self,
        completion: PendingVerificationCompletion,
    ) -> ActorResult<()> {
        self.detail_workflow.cache_pending_completion(completion);
        Produces::ok(())
    }

    pub async fn schedule_wallet_upload(
        &mut self,
        wallet_id: WalletId,
        immediate: bool,
    ) -> ActorResult<()> {
        call!(self.uploads.schedule_wallet_upload(wallet_id, immediate)).await?;
        Produces::ok(())
    }

    /// Restarts background work after the cloud may be reachable again, as the runtime allows
    ///
    /// A committed Turn Off that nothing is finishing resumes; any other background work
    /// restarts only while the published policy runs ordinary writes
    pub async fn cloud_reachable(&mut self, cause: CloudReachable) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if matches!(*self.runtime, Runtime::FinishingDisable { op: None, .. }) {
            if cause != CloudReachable::PendingVerification {
                manager.resume_persisted_disable_if_needed();
            }
            return Produces::ok(());
        }

        if !self.runtime.published_policy().policy.runs_ordinary() {
            debug!("Cloud Backup left background work paused after {cause:?}");
            return Produces::ok(());
        }

        manager.restart_background_work(cause);
        Produces::ok(())
    }

    /// The write lane runs ordinary writes again, under this or a new configuration
    ///
    /// Paused uploads kept their rows Dirty with no retry timer, so one scheduler pass picks
    /// every one of them up, and the pending upload verifier restarts with them. Both are sent,
    /// not awaited, so this handler never waits on a worker
    pub async fn ordinary_writes_resumed(&mut self) -> ActorResult<()> {
        send!(self.uploads.resume_wallet_uploads_from_persisted_state());
        send!(self.uploads.ensure_pending_upload_verification_loop());
        Produces::ok(())
    }

    pub async fn resume_wallet_uploads_from_persisted_state(&mut self) -> ActorResult<()> {
        call!(self.uploads.resume_wallet_uploads_from_persisted_state()).await?;
        Produces::ok(())
    }

    pub async fn ensure_pending_upload_verification_loop(&mut self) -> ActorResult<()> {
        call!(self.uploads.ensure_pending_upload_verification_loop()).await?;
        Produces::ok(())
    }

    pub async fn wake_pending_upload_verifier(&mut self) -> ActorResult<()> {
        call!(self.uploads.wake_pending_upload_verifier()).await?;
        Produces::ok(())
    }

    pub async fn start_restore_from_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let addr = self.addr();
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
        else {
            warn!("restore_from_cloud_backup called while another operation is active, ignoring");
            return Produces::ok(());
        };

        let cancellation = self.restore_cancellation(claim);
        let operation = RestoreOperation::new(claim, addr.clone(), cancellation);
        addr.send_fut_with(move |addr| async move {
            tracing::info!("restore_from_cloud_backup: task started");
            match operation.restore_from_cloud_backup(&manager).await {
                Ok(_) => {
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(CloudBackupError::Cancelled) => {
                    tracing::info!("restore_from_cloud_backup: task cancelled");
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(CloudBackupError::NoBackupFound) => {
                    tracing::info!("restore_from_cloud_backup: no cloud backups found");
                    let _ =
                        operation.apply_outcome(CloudBackupRestoreOutcome::ProgressCleared).await;
                    let status = RustCloudBackupManager::runtime_status_for(
                        &RustCloudBackupManager::load_persisted_state(),
                    );
                    let _ = operation.apply_status(status).await;
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(error) => {
                    error!("restore_from_cloud_backup failed: {error}");
                    send!(addr.fail_exclusive_operation(claim, error));
                }
            }
        });

        Produces::ok(())
    }

    pub async fn start_restore_from_cloud_backup_with_events(
        &mut self,
        sender: flume::Sender<CloudBackupRestoreEvent>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let addr = self.addr();

        // onboarding starts this outside dispatch, so it is admitted the same way: a configured
        // backup or a running operation refuses it
        let claim = match self.admit(&manager, &UserIntent::RestoreFromCloudBackup) {
            Admission::Accepted => {
                self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
            }
            Admission::Coalesced(_) | Admission::Rejected(_) => None,
        };
        let Some(claim) = claim else {
            warn!("restore_from_cloud_backup was not admitted, ignoring");
            if let Err(error) = sender
                .send_async(CloudBackupRestoreEvent::Failed("restore already in progress".into()))
                .await
            {
                warn!(
                    "restore_from_cloud_backup: failed to send already in progress event: {error}"
                );
            }
            return Produces::ok(());
        };

        let cancellation = self.restore_cancellation(claim);
        let operation =
            RestoreOperation::new_with_events(claim, addr.clone(), sender, cancellation);
        addr.send_fut_with(move |addr| async move {
            tracing::info!("restore_from_cloud_backup: task started for onboarding");
            match operation.restore_from_cloud_backup(&manager).await {
                Ok(report) => {
                    operation
                        .send_event_if_current(CloudBackupRestoreEvent::Complete(report))
                        .await;
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(CloudBackupError::Cancelled) => {
                    tracing::info!("restore_from_cloud_backup: task cancelled");
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(CloudBackupError::NoBackupFound) => {
                    tracing::info!("restore_from_cloud_backup: no cloud backups found");
                    let _ =
                        operation.apply_outcome(CloudBackupRestoreOutcome::ProgressCleared).await;
                    let status = RustCloudBackupManager::runtime_status_for(
                        &RustCloudBackupManager::load_persisted_state(),
                    );
                    let _ = operation.apply_status(status).await;
                    operation.send_event_if_current(CloudBackupRestoreEvent::NoBackupFound).await;
                    send!(addr.complete_exclusive_operation(claim));
                }
                Err(error) => {
                    error!("restore_from_cloud_backup failed: {error}");
                    operation
                        .send_event_if_current(CloudBackupRestoreEvent::Failed(
                            error.reader_message(),
                        ))
                        .await;
                    send!(addr.fail_exclusive_operation(claim, error));
                }
            }
        });

        Produces::ok(())
    }

    pub async fn cancel_restore(&mut self) -> ActorResult<()> {
        let Some(run) = self.runtime.claim().and_then(|claim| self.runtime.restore(claim)) else {
            return Produces::ok(());
        };

        if run.cancellation.swap(true, Ordering::AcqRel) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else { return Produces::ok(()) };

        manager.clear_enable_progress_report();
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));
        tracing::info!("restore_from_cloud_backup: cancellation requested for active restore");
        Produces::ok(())
    }

    pub async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        self.detail_workflow.clear_pending_completion();
        call!(self.sync_health.clear_upload_runtime_state()).await?;
        call!(self.uploads.clear_upload_runtime_state()).await?;
        Produces::ok(())
    }

    /// Closes admission for a local reset; the receiver answers once no effect is in flight
    ///
    /// A cancellable operation ends now. One in an `Effecting` stage keeps authority until its
    /// completion message arrives, and the reset continues then. The caller awaits the
    /// receiver, so this handler never waits and the actor still receives that completion
    pub async fn begin_local_reset(
        &mut self,
    ) -> ActorResult<Result<oneshot::Receiver<()>, Rejection>> {
        if let Some(manager) = self.manager()
            && let Admission::Rejected(rejection) = self.admit(&manager, &UserIntent::LocalReset)
        {
            return Produces::ok(Err(rejection));
        }

        let (ready, receiver) = oneshot::channel();
        let start = self.runtime.begin_reset(ready);
        debug!("Cloud Backup local reset began: {start:?}");
        if let Some(manager) = self.manager() {
            match start {
                ResetStart::Cancelled(claim) | ResetStart::AwaitingEffect(claim) => {
                    manager.project_exclusive_operation_finished(claim);
                }
                ResetStart::Idle => {}
            }
        }

        self.active_sync_request = None;
        self.active_cloud_only_fetch_request = None;
        self.clear_upload_runtime_state().await?;
        Produces::ok(Ok(receiver))
    }

    /// Clears Cloud Backup's local keychain state for a full wipe, as the namespace's owner
    pub async fn clear_local_state_for_reset(
        &mut self,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !matches!(*self.runtime, Runtime::Resetting { held: None, .. }) {
            return Produces::ok(Err(CloudBackupError::Internal(
                "cloud backup local state can only be cleared by a ready reset".into(),
            )));
        }

        let owner = OwnerToken::mint();
        let result = CloudBackupKeychain::global()
            .clear_local_state(&LocalStateOwner::Lifecycle(&owner))
            .map_err(|source| {
                CloudBackupError::internal_context(
                    "clear cloud backup local keychain state",
                    source,
                )
            });

        Produces::ok(result)
    }

    /// Ends a local reset with the body it left, and reopens admission
    ///
    /// After a wipe Cloud Backup is off, whatever the record said before; the owner sets that
    /// here so an idle body never waits for the next admission to catch up with the wipe
    pub async fn finish_local_reset(&mut self, outcome: LocalResetOutcome) -> ActorResult<()> {
        if !matches!(*self.runtime, Runtime::Resetting { .. }) {
            return Produces::ok(());
        }

        let after = match outcome {
            LocalResetOutcome::Wiped => Runtime::off(),
            LocalResetOutcome::Abandoned => Runtime::at_load(
                None,
                IdleFacts {
                    record: &RustCloudBackupManager::load_persisted_state(),
                    pending_enable_recovery: None,
                },
            ),
        };
        self.runtime.finish_reset(after);
        debug!("Cloud Backup local reset finished: {outcome:?}");

        Produces::ok(())
    }
}

/// How a local reset ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalResetOutcome {
    /// The database reset committed, so Cloud Backup is off
    Wiped,
    /// The wipe failed before its commit, so the stored record still holds
    Abandoned,
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::*;

    impl CloudBackupKeychain {
        /// Clears local state the way the lifecycle owner does, for test setup
        pub(crate) fn clear_local_state_for_test(
            &self,
        ) -> Result<(), crate::manager::cloud_backup_manager::keychain::CloudBackupKeychainError>
        {
            self.clear_local_state(&LocalStateOwner::Lifecycle(&OwnerToken::mint()))
        }
    }

    /// What a test reset finds on one test's supervisor
    pub(crate) enum LeftoverOperationForTest {
        /// An off-actor effect is in flight; the receiver answers once it settles
        Settling(oneshot::Receiver<()>),
        /// No operation runs any more; `orphaned` when its test already dropped the manager,
        /// so the supervisor can begin no operation again
        Ended { orphaned: bool },
    }

    /// One dispatched intent and how many admissions its handler made
    #[derive(Debug, Clone)]
    pub(crate) struct DispatchRecord {
        pub(crate) intent: UserIntent,
        pub(crate) admissions: u32,
    }

    /// Admissions counted across every caller of `admit`, and the dispatches since the last read
    #[derive(Debug, Default)]
    pub(crate) struct DispatchLog {
        pub(crate) admissions: u32,
        dispatches: Vec<DispatchRecord>,
    }

    /// Dispatch in progress: the intent and the admission count when its handler began
    pub(crate) struct OpenDispatch(DispatchRecord);

    /// What the harness invariants read from the supervisor at a settled boundary
    #[derive(Debug)]
    pub(crate) struct SupervisorView {
        pub(crate) owner: OwnerView,
        pub(crate) dispatches: Vec<DispatchRecord>,
        /// The operation the reducer shows as running
        pub(crate) shown_operation: Option<CloudBackupExclusiveOperationClaim>,
        pub(crate) uploads: Addr<CloudBackupUploadWorker>,
    }

    impl CloudBackupSupervisor {
        pub(crate) fn begin_dispatch_record(&self, intent: &UserIntent) -> OpenDispatch {
            OpenDispatch(DispatchRecord {
                intent: intent.clone(),
                admissions: self.invariant_log.admissions,
            })
        }

        pub(crate) fn finish_dispatch_record(&mut self, OpenDispatch(mut record): OpenDispatch) {
            record.admissions = self.invariant_log.admissions - record.admissions;
            self.invariant_log.dispatches.push(record);
        }

        /// Reads the supervisor for the harness invariants, taking what it logged since the
        /// last read
        pub(crate) async fn harness_invariant_view(&mut self) -> ActorResult<SupervisorView> {
            let shown_operation =
                self.manager().and_then(|manager| manager.state.read().active_operation());

            Produces::ok(SupervisorView {
                owner: self.runtime.take_view(),
                dispatches: std::mem::take(&mut self.invariant_log.dispatches),
                shown_operation,
                uploads: self.uploads.clone(),
            })
        }

        /// What dispatch would do with `intent` now, without doing it
        pub(crate) async fn harness_admission(
            &mut self,
            intent: crate::manager::cloud_backup_manager::admission::UserIntent,
        ) -> ActorResult<crate::manager::cloud_backup_manager::admission::Admission> {
            use crate::manager::cloud_backup_manager::admission::Rejection;

            let admission = match self.manager() {
                Some(manager) => self.decide(&manager, &intent),
                None => Admission::Rejected(Rejection::NotConfigured),
            };
            Produces::ok(admission)
        }

        /// The stage checkpoint: answers once no operation is waiting on an off-actor effect
        pub(crate) async fn harness_effects_settled(
            &mut self,
        ) -> ActorResult<oneshot::Receiver<()>> {
            let (settled, receiver) = oneshot::channel();
            self.runtime.when_effects_settle(settled);
            Produces::ok(receiver)
        }

        pub(crate) async fn harness_last_outcome(
            &mut self,
        ) -> ActorResult<Option<DispatchOutcome>> {
            Produces::ok(self.last_outcome.clone())
        }

        /// The exclusive slot, for harness failure messages only
        pub(crate) async fn harness_active_operation(&mut self) -> ActorResult<String> {
            Produces::ok(format!("{:?}", *self.runtime))
        }

        /// Whether an operation holds the runtime now
        pub(crate) async fn harness_runs_operation(&mut self) -> ActorResult<bool> {
            Produces::ok(self.runtime.claim().is_some())
        }

        pub async fn run_wallet_upload_inline_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<()> {
            call!(self.uploads.run_wallet_upload_inline_for_test(wallet_id)).await?;
            Produces::ok(())
        }

        pub async fn wallet_upload_retry_count_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<u32> {
            let count = call!(self.uploads.wallet_upload_retry_count_for_test(wallet_id)).await?;
            Produces::ok(count)
        }

        pub async fn new_restore_operation(&mut self) -> ActorResult<RestoreOperation> {
            let manager = self.manager().expect("cloud backup manager exists");
            let addr = self.addr();
            if let Some(claim) = self.runtime.take_claim() {
                manager.project_exclusive_operation_finished(claim);
            }
            let claim = self
                .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
                .expect("begin restore operation");
            let cancellation = self.restore_cancellation(claim);
            let operation = RestoreOperation::new(claim, addr, cancellation);
            Produces::ok(operation)
        }

        pub async fn invalidate_restore_operation(&mut self) -> ActorResult<()> {
            self.cancel_restore().await?;
            Produces::ok(())
        }

        pub async fn take_pending_enable_session_for_test(
            &mut self,
        ) -> ActorResult<Option<PendingEnableSession>> {
            Produces::ok(self.pending_enable_session.take())
        }

        pub async fn replace_pending_enable_session_for_test(
            &mut self,
            session: PendingEnableSession,
        ) -> ActorResult<()> {
            self.pending_enable_session = Some(session);
            Produces::ok(())
        }

        pub async fn has_awaiting_saved_passkey_confirmation_for_test(&self) -> ActorResult<bool> {
            Produces::ok(
                self.pending_enable_session
                    .as_ref()
                    .is_some_and(PendingEnableSession::is_awaiting_saved_passkey_confirmation),
            )
        }

        /// Starts an exclusive operation and hands back its write client, for tests that run the
        /// operation's body directly
        pub async fn begin_operation_writes_for_test(
            &mut self,
            operation: CloudBackupExclusiveOperation,
        ) -> ActorResult<(CloudBackupExclusiveOperationClaim, CloudBackupWriteClient)> {
            let manager = self.manager().expect("cloud backup manager exists");
            let claim = self
                .begin_exclusive_operation(&manager, operation)
                .expect("begin exclusive operation");

            Produces::ok((claim, CloudBackupWriteClient::for_operation(self.write.clone(), claim)))
        }

        /// Ends the operation a previous test left running, once no off-actor effect is in flight
        ///
        /// While an effect runs, answers a receiver for its settling; the caller asks again
        /// after it, so an effect a late completion began is waited out too. Once the claim is
        /// gone, later completions are stale and start nothing
        pub async fn end_leftover_operation_for_test(
            &mut self,
        ) -> ActorResult<LeftoverOperationForTest> {
            let (settled, mut receiver) = oneshot::channel();
            self.runtime.when_effects_settle(settled);
            if receiver.try_recv().is_err() {
                return Produces::ok(LeftoverOperationForTest::Settling(receiver));
            }

            let manager = self.manager();
            if let Some(claim) = self.runtime.take_claim()
                && let Some(manager) = &manager
            {
                manager.project_exclusive_operation_finished(claim);
            }

            self.runtime.rebase_after_test_write();
            Produces::ok(LeftoverOperationForTest::Ended { orphaned: manager.is_none() })
        }

        /// Rebases on a record a test seeded without the owner
        pub async fn rebase_after_test_write(&mut self) -> ActorResult<()> {
            self.rebase_on_seeded_record();
            Produces::ok(())
        }

        /// The same, for a supervisor a test drives directly instead of as an actor
        pub(crate) fn rebase_on_seeded_record(&mut self) {
            self.runtime.rebase_after_test_write();
        }

        /// Ends any operation a previous test left running and rebases on the reset record
        pub async fn reset_runtime_for_test(&mut self) -> ActorResult<()> {
            let manager = self.manager().expect("cloud backup manager exists");
            if let Some(claim) = self.runtime.take_claim() {
                manager.project_exclusive_operation_finished(claim);
            }

            self.runtime.rebase_after_test_write();
            Produces::ok(())
        }

        pub async fn cleanup_idle_for_test(&mut self) -> ActorResult<bool> {
            let idle = call!(self.cleanup.is_idle_for_test()).await?;
            Produces::ok(idle)
        }

        pub async fn enqueue_cleanup_for_test(
            &mut self,
            job: CloudBackupCleanupJob,
        ) -> ActorResult<()> {
            call!(self.cleanup.enqueue_cleanup(job)).await?;
            Produces::ok(())
        }
    }
}
