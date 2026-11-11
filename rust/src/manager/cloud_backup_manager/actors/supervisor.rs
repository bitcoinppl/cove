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
use tracing::{debug, error, info, warn};

use super::CloudBackupSyncHealthWorker;
use super::cleanup::{CleanupSourceNamespace, CloudBackupCleanupJob, CloudBackupCleanupWorker};
use super::restore::{self, CloudBackupRestoreEvent, RestoreOperation, RestoredNamespaceCommit};
use super::uploads::CloudBackupUploadWorker;
use super::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteBlocker,
    CloudBackupWriteClient, CloudBackupWriteCompletion, CloudBackupWriteSupervisor,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudStorageIssue, DriveAccountSwitchId, PersistedCloudBackupState,
    PersistedCloudBlobSyncState, PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase,
};
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::admission::{Admission, UserIntent};
use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;
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
    CloudBackupVerificationSource, CloudBackupWalletItem, CloudOnlyOperation,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult,
    DriveAccountSwitchPlatformState, DriveAccountSwitchReconcileAction,
    EnablePasskeyRegistrationFlow, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, OtherBackupsOperation,
    PendingEnableJournal, PendingEnableJournalPhase, PendingEnableLocalMetadataSnapshot,
    PendingEnableNamespaceOwnership, PendingEnableSession, PendingUploadVerificationState,
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
mod restore_all;
mod runtime;

use runtime::{IdleFacts, RestoreRun, Runtime};
mod undecryptable_wallets;
mod verification;

pub(crate) use dispatch::DispatchOutcome;
pub(crate) use verification::DeepVerificationContinuation;

use detail_workflow::{DetailRefreshClaim, DetailRefreshPlan, DetailResultClaim, DetailWorkflow};
use restore_all::restore_all_marker_matches_active_namespace;

mod tests {
    #![cfg(test)]

    include!("supervisor/tests.rs");
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

/// Pending disable state held while the write lane drains before namespace delete
#[derive(Debug)]
struct PendingDisableWriteDrain {
    claim: CloudBackupExclusiveOperationClaim,
    blocker: CloudBackupWriteBlocker,
    attempt: crate::manager::cloud_backup_manager::ops::DisableAttempt,
}

/// Which local wallet set a verification checked
///
/// Process-local, like `OperationId`: verification evidence is only valid for the wallet set it
/// read, and a wallet change that lands while a verification runs makes that evidence stale
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WalletSetGeneration(u64);

impl WalletSetGeneration {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
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
    lifecycle: LifecycleWriter,
    cleanup: Addr<CloudBackupCleanupWorker>,
    sync_health: Addr<CloudBackupSyncHealthWorker>,
    uploads: Addr<CloudBackupUploadWorker>,
    write: Addr<CloudBackupWriteSupervisor>,
    /// The lifecycle body with the running operation nested in it
    runtime: Runtime,
    pending_enable_session: Option<PendingEnableSession>,
    next_request_id: u64,
    active_sync_request: Option<u64>,
    active_cloud_only_fetch_request: Option<u64>,
    active_other_backups_scan: Option<OtherBackupsScanClaim>,
    detail_workflow: DetailWorkflow,
    pending_disable_write_drain: Option<PendingDisableWriteDrain>,
    /// Counts local wallet-set changes this process has seen
    wallet_set: WalletSetGeneration,
    /// The wallet set the running verification read when it started
    verification_basis: WalletSetGeneration,
    last_outcome: Option<DispatchOutcome>,
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
                pending_enable_recovery: false,
            },
        );

        Self {
            addr: WeakAddr::default(),
            lifecycle,
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
            pending_disable_write_drain: None,
            wallet_set: WalletSetGeneration::default(),
            verification_basis: WalletSetGeneration::default(),
            last_outcome: None,
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

    async fn delete_cloud_backup_namespace_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        namespace: String,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::for_operation(write, origin)
            .delete_namespace(CloudStorage::global_explicit_client(), namespace)
            .await
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

        self.rebase_idle_runtime(manager);
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
        let record = RustCloudBackupManager::load_persisted_state();
        let facts = IdleFacts {
            record: &record,
            pending_enable_recovery: manager.state.read().shows_pending_enable_recovery(),
        };
        if let Err(unstartable) = self.runtime.hand_off(from, to, facts) {
            warn!("Cloud Backup could not hand off to {unstartable:?}");
            return None;
        }

        manager.project_exclusive_operation_handed_off(from, to);
        Some(to)
    }

    fn begin_restore_all_exclusive_operation(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if self.runtime.is_some() {
            return None;
        }

        self.rebase_idle_runtime(manager);
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

        self.rebase_idle_runtime(&manager);
        let exported = self.runtime.exported();
        if let Some(status) = exported.status {
            manager.reconcile_runtime_status(status);
        }
        if let Some(disable) = exported.disable {
            manager.apply_disable_outcome(disable);
        }

        Produces::ok(())
    }

    /// Moves an idle runtime body onto the supervisor's own committed record
    ///
    /// The supervisor holds the only writer, so the record is the owner's state, not a mirror;
    /// a running operation holds its body and is not moved
    fn rebase_idle_runtime(&mut self, manager: &RustCloudBackupManager) {
        let record = RustCloudBackupManager::load_persisted_state();
        self.runtime.rebase_idle(IdleFacts {
            record: &record,
            pending_enable_recovery: manager.state.read().shows_pending_enable_recovery(),
        });
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

        self.runtime.clear();
        if let Some(manager) = self.manager() {
            manager.project_exclusive_operation_finished(claim);
            if claim.operation() == CloudBackupExclusiveOperation::Restore
                && let Err(error) =
                    call!(self.uploads.resume_wallet_uploads_from_persisted_state()).await
            {
                warn!("Failed to resume wallet uploads after restore: {error}");
            }
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

        let result = manager.reconcile_committed(
            "persist empty restored cloud backup state",
            self.lifecycle.record_restore_without_backup(),
        );

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

        Produces::ok(Self::commit_restored_namespace(&manager, &self.lifecycle, commit))
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

    pub async fn start_recover_other_backups_operation(&mut self) -> ActorResult<()> {
        self.begin_recover_other_backups_operation();
        Produces::ok(())
    }

    pub async fn start_delete_other_backups_operation(&mut self) -> ActorResult<()> {
        self.begin_delete_other_backups_operation();
        Produces::ok(())
    }

    fn start_sync_request(&mut self, manager: Arc<RustCloudBackupManager>) {
        let request_id = self.next_request_id();
        self.active_sync_request = Some(request_id);
        manager.apply_sync_state(SyncState::Syncing);

        self.addr.send_fut_with(move |addr| async move {
            let result = manager.do_sync_unsynced_wallets().await;
            send!(addr.complete_sync_request(request_id, result));
        });
    }

    pub async fn complete_sync_request(
        &mut self,
        request_id: u64,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_sync_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_sync_request = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
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
            Err(error) => {
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
                let addr = self.addr();
                send!(addr.start_verification(force_discoverable));
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

    pub async fn prepare_local_reset_runtime(&mut self) -> ActorResult<()> {
        if let Some(released_claim) = self.runtime.prepare_local_reset()
            && let Some(manager) = self.manager()
        {
            manager.project_exclusive_operation_finished(released_claim);
        }
        self.active_sync_request = None;
        self.active_cloud_only_fetch_request = None;
        self.pending_disable_write_drain = None;
        self.clear_upload_runtime_state().await
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::*;

    impl CloudBackupSupervisor {
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

        pub(crate) async fn harness_last_outcome(
            &mut self,
        ) -> ActorResult<Option<DispatchOutcome>> {
            Produces::ok(self.last_outcome.clone())
        }

        /// The exclusive slot, for harness failure messages only
        pub(crate) async fn harness_active_operation(&mut self) -> ActorResult<String> {
            Produces::ok(format!("{:?}", self.runtime))
        }

        pub async fn run_wallet_upload_inline_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<()> {
            call!(self.uploads.run_wallet_upload_inline_for_test(wallet_id)).await?;
            Produces::ok(())
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
