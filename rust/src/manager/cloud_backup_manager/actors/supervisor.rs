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
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

use super::cleanup::{CleanupSourceNamespace, CloudBackupCleanupJob};
use super::restore::{self, CloudBackupRestoreEvent, RestoreOperation, RestoredNamespaceCommit};
use super::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteClient,
    CloudBackupWriteCompletion, CloudBackupWriteSupervisor, ConfigGeneration,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudStorageIssue, DriveAccountSwitchId, PersistedCloudBackupState,
    PersistedCloudBlobSyncState, PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase,
};
use crate::database::error::DatabaseError;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionBody, ContinuationOwner, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::keychain::{
    CloudBackupKeychain, CloudBackupKeychainError, LocalStateOwner,
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::LifecycleWriter;
use crate::manager::cloud_backup_manager::owner::operation::OperationId;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupScreenAction, LastOutcome, Notices, ScreenInputs,
};
use crate::manager::cloud_backup_manager::turn_off::AutomaticResume;
use crate::manager::cloud_backup_manager::verify::coordinator::CloudBackupVerificationCoordinator;
use crate::manager::cloud_backup_manager::verify::{
    CloudBackupDeepVerificationAutoSyncCompletion, CloudBackupDeepVerificationStep,
    CloudBackupPasskeyRepairFinalization, CloudBackupPendingDeepVerificationAutoSyncResume,
    CloudBackupPendingDeepVerificationResume, CloudBackupPreparedDeepVerificationAutoSync,
    CloudBackupPreparedDeepVerificationWrapperRepair, CloudBackupPreparedPasskeyWrapperRepair,
    CloudBackupUploadedDeepVerificationAutoSync, CloudBackupUploadedPasskeyWrapperRepair,
};
use crate::manager::cloud_backup_manager::wallets::{
    CloudOnlyRestoreOutcome, delay_before_new_passkey_auth,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_LABELS_WARNING_MESSAGE, CloudBackupCloudOnlyFetchOutcome,
    CloudBackupCloudOnlyOperationWarning, CloudBackupCloudOnlyWalletOutcome,
    CloudBackupDetailInventorySnapshot, CloudBackupDetailInventorySnapshotResult,
    CloudBackupDetailOutcome, CloudBackupDetailProviderConfirmation, CloudBackupDetailResult,
    CloudBackupDetailSnapshotCompletion, CloudBackupDriveAccountSwitchError,
    CloudBackupEnableContext, CloudBackupEnablePasskeyPreparation,
    CloudBackupEnablePasskeyRegistration, CloudBackupEnablePreparation,
    CloudBackupEnableRecoveryCompletion, CloudBackupEnableRecoveryPreparation,
    CloudBackupEnableState, CloudBackupError, CloudBackupInventoryAuthority,
    CloudBackupInventoryIncompleteReason, CloudBackupNoDiscoveryEnablePreparation,
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
    EnablePasskeyRegistrationFlow, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, InventoryReadAt,
    InventoryScope, OtherBackupsOperation, PendingEnableJournal, PendingEnableJournalPhase,
    PendingEnableLocalMetadataSnapshot, PendingEnableNamespaceOwnership, PendingEnableSession,
    PendingUploadVerificationState, PendingVerificationCompletion, PendingVerificationUpload,
    PendingWork, RecoveryAction, RecoveryState, RustCloudBackupManager,
    SavedPasskeyConfirmationMode, SyncState, VerificationState, WalletId, blocking_cloud_error,
    is_provider_wide_interruption,
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
#[cfg(test)]
pub(crate) use runtime::Finish;
pub(crate) use runtime::Runtime;
use runtime::{Effect, ResetStart, RestoreRun};
use workers::{CleanupWorker, SyncHealthWorker, UploadWorker};
mod undecryptable_wallets;
mod verification;
mod workers;

use enable::{DiscardTarget, EnableContinuation};

pub(crate) use account_switch::{
    DriveAccountSwitchReinitializationEnd, UnsavedDriveAccountSwitchCompletion,
};
pub(crate) use dispatch::DispatchOutcome;
pub(crate) use verification::DeepVerificationContinuation;

use detail_workflow::{
    DetailRefreshClaim, DetailRefreshOrigin, DetailRefreshPlan, DetailResultClaim, DetailWorkflow,
};

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

/// The passkey a wrapper repair uploaded, saved as this device's passkey once the upload lands
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

fn apply_refresh_detail_result(
    manager: &RustCloudBackupManager,
    scope: InventoryScope,
    result: &CloudBackupDetailResult,
) {
    let outcome = match result {
        CloudBackupDetailResult::SuccessWithAuthority { listed, authority } => {
            CloudBackupDetailOutcome::RefreshedWithAuthority {
                listed: listed.clone(),
                authority: *authority,
            }
        }
        CloudBackupDetailResult::AccessError(error) => {
            error!("Failed to refresh detail: {error}");
            CloudBackupDetailOutcome::Failed {
                reason: CloudBackupInventoryIncompleteReason::from(CloudStorageIssue::from(error)),
                error: error.reader_message(),
            }
        }
    };

    manager.apply_detail_outcome(scope, outcome);
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

/// What verification evidence was read under: the backup configuration and the local wallet set
///
/// A producer reads it from the supervisor before it reads any evidence, and the supervisor
/// records the evidence only while both are still current. A verification operation's claim
/// already pins its configuration; background upload confirmation has no claim, so the
/// configuration here is what keeps one backup's confirmation from settling another's
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EvidenceBasis {
    config: ConfigGeneration,
    wallet_set: WalletSetGeneration,
}

impl EvidenceBasis {
    /// The configuration generation this basis was read under
    pub(crate) fn config(self) -> ConfigGeneration {
        self.config
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
    cleanup: CleanupWorker,
    sync_health: SyncHealthWorker,
    uploads: UploadWorker,
    write: Addr<CloudBackupWriteSupervisor>,
    /// The lifecycle body with the running operation nested in it, and the only lifecycle
    /// writer, both published to the write lane
    runtime: OwnedRuntime,
    /// The Enable or reinitialize that stopped before it finished, with the lifecycle it
    /// belongs to
    enable_continuation: Option<EnableContinuation>,
    /// A switch's reinitialize that ended while saving the switch's next phase failed; its run
    /// and claim are over, and the switch saves this before it reads its phase again
    drive_account_switch_unsaved: Option<UnsavedDriveAccountSwitchCompletion>,
    next_request_id: u64,
    active_sync_request: Option<u64>,
    active_cloud_only_fetch_request: Option<u64>,
    active_other_backups_scan: Option<OtherBackupsScanClaim>,
    detail_workflow: DetailWorkflow,
    /// Counts local wallet-set changes this process has seen; each verification run carries the
    /// generation it read, so there is no supervisor-wide verification basis
    wallet_set: WalletSetGeneration,
    last_outcome: Option<DispatchOutcome>,
    /// The outcome the screen shows next to the control the user last tapped
    screen_outcome: LastOutcome,
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
        let runtime =
            OwnedRuntime::at_load(load_error, lifecycle, cloud_writes.clone(), manager.clone());

        Self {
            addr: WeakAddr::default(),
            cleanup: CleanupWorker::spawn(manager.clone()),
            sync_health: SyncHealthWorker::spawn(manager.clone()),
            uploads: UploadWorker::spawn(manager.clone()),
            write: cloud_writes,
            manager,
            runtime,
            enable_continuation: None,
            drive_account_switch_unsaved: None,
            next_request_id: 0,
            active_sync_request: None,
            active_cloud_only_fetch_request: None,
            active_other_backups_scan: None,
            detail_workflow: DetailWorkflow::default(),
            wallet_set: WalletSetGeneration::default(),
            last_outcome: None,
            screen_outcome: LastOutcome::default(),
            #[cfg(test)]
            invariant_log: test_support::DispatchLog::default(),
        }
    }

    /// The configuration and wallet set evidence read now would be judged against
    pub(crate) fn evidence_basis(&self) -> EvidenceBasis {
        EvidenceBasis { config: self.runtime.generation(), wallet_set: self.wallet_set }
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
            self.runtime.finish(claim);
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

    /// Publishes the screen for the runtime, its admission and the notices as they stand now
    pub async fn publish_screen(&mut self) -> ActorResult<()> {
        if let Some(manager) = self.manager() {
            manager.publish_screen(&self.runtime, || self.screen_inputs(&manager));
        }

        Produces::ok(())
    }

    /// What the projection reads besides the runtime, as it stands now
    pub(crate) fn screen_inputs(&self, manager: &RustCloudBackupManager) -> ScreenInputs {
        let inventory = manager.inventory_facts();
        let admission = self.admission_snapshot(manager, &inventory);
        let notices =
            Notices::new(&RustCloudBackupManager::load_persisted_state(), &self.screen_outcome);

        ScreenInputs { configuration: self.runtime.generation(), admission, inventory, notices }
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
        self.runtime.finish(claim);
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

        // clearing the runtime publishes, so the screen shows the failure with the body it left
        self.screen_outcome.operation_failed(claim, &error);
        self.runtime.finish(claim);
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

    /// Deletes those of `record_ids`, the batch the user confirmed, that still fail to decrypt
    pub async fn start_delete_undecryptable_wallet_backups_operation(
        &mut self,
        record_ids: Vec<String>,
    ) -> ActorResult<()> {
        self.begin_delete_undecryptable_wallet_backups_operation(record_ids);
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
                let detail_claim =
                    self.detail_workflow.start_operation_result(manager.inventory_scope());
                let config = self.runtime.generation();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail(config).await;
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
            apply_refresh_detail_result(&manager, detail_claim.scope(), &result);
        }

        manager.apply_sync_state(SyncState::Idle);
        self.active_sync_request = None;
        Produces::ok(())
    }

    pub async fn start_refresh_detail(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.detail_workflow.open();
        let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Tap);
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
        if manager.listing_is_provider_confirmed() {
            return Produces::ok(());
        }

        let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Automatic);
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
            let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Automatic);
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
            let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Automatic);
            self.handle_detail_refresh_plan(manager, plan);
        }

        Produces::ok(())
    }

    pub async fn start_detail_result_claim(
        &mut self,
        scope: InventoryScope,
    ) -> ActorResult<DetailResultClaim> {
        Produces::ok(self.detail_workflow.start_operation_result(scope))
    }

    fn schedule_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
        scope: InventoryScope,
    ) {
        manager.refresh_sync_health();
        self.addr.send_fut_with(move |addr| async move {
            let cloud = claim.origin().cloud_client();
            let result = manager.load_cloud_backup_detail_inventory_snapshot(cloud).await;
            send!(addr.complete_refresh_detail_snapshot(result, attempt, claim, scope));
        });
    }

    fn schedule_complete_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        snapshot: CloudBackupDetailInventorySnapshot,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
        scope: InventoryScope,
    ) {
        let config = self.runtime.generation();
        self.addr.send_fut_with(move |addr| async move {
            let cloud = claim.origin().cloud_client();
            let completion = manager
                .complete_cloud_backup_detail_inventory_snapshot(snapshot, config, cloud)
                .await;
            send!(addr.complete_refresh_detail_from_snapshot(completion, attempt, claim, scope));
        });
    }

    fn schedule_confirm_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        confirmation: CloudBackupDetailProviderConfirmation,
        attempt: DetailRefreshAttempt,
        claim: DetailRefreshClaim,
        scope: InventoryScope,
    ) {
        let config = self.runtime.generation();
        self.addr.send_fut_with(move |addr| async move {
            let cloud = claim.origin().cloud_client();
            let result = manager
                .confirm_cloud_backup_detail_inventory(confirmation.clone(), config, cloud)
                .await;
            send!(addr.complete_confirm_refresh_detail(
                confirmation,
                result,
                attempt,
                claim,
                scope
            ));
        });
    }

    fn handle_detail_refresh_plan(
        &self,
        manager: Arc<RustCloudBackupManager>,
        plan: DetailRefreshPlan,
    ) {
        match plan {
            DetailRefreshPlan::Start(claim) => {
                // the refresh and every result it produces belong to the configuration it
                // started under
                let scope = manager.inventory_scope();
                manager.apply_detail_outcome(scope, CloudBackupDetailOutcome::Checking);
                self.schedule_refresh_detail(manager, DetailRefreshAttempt::Initial, claim, scope);
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
        scope: InventoryScope,
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
                scope,
            );
            return Produces::ok(());
        }

        let completion = self.detail_workflow.complete_refresh(claim);
        if completion.apply
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, scope, &result);
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
        scope: InventoryScope,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if !self.detail_workflow.is_refresh_active(claim) {
            return Produces::ok(());
        }

        match completion {
            None => self.complete_refresh_detail(None, attempt, claim, scope).await,
            Some(CloudBackupDetailSnapshotCompletion::Final(result)) => {
                self.complete_refresh_detail(Some(result), attempt, claim, scope).await
            }
            Some(CloudBackupDetailSnapshotCompletion::TrustedLocal { listed, confirmation }) => {
                if self.detail_workflow.is_latest_refresh(claim) {
                    manager.apply_detail_outcome(
                        scope,
                        CloudBackupDetailOutcome::RefreshedWithAuthority {
                            listed,
                            authority:
                                CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
                        },
                    );
                }

                self.ensure_supplemental_inventory_discovery(manager.clone());
                self.schedule_confirm_refresh_detail(manager, confirmation, attempt, claim, scope);

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
        scope: InventoryScope,
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
                scope,
            );
            return Produces::ok(());
        }

        let completion = self.detail_workflow.complete_refresh(claim);
        if completion.apply
            && let Some(result) = result
        {
            // the local rows stay retained, but an unconfirmed inventory is incomplete so the user
            // sees why restore and destructive actions are unavailable and can check again
            apply_refresh_detail_result(&manager, scope, &result);
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
        scope: InventoryScope,
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
                    manager.apply_detail_outcome(
                        scope,
                        CloudBackupDetailOutcome::Provisional(provisional_detail),
                    );
                }

                self.schedule_complete_refresh_detail(manager, snapshot, attempt, claim, scope);
            }
            CloudBackupDetailInventorySnapshotResult::AccessError(error) => {
                let result = Some(CloudBackupDetailResult::AccessError(error));
                if refresh_detail_needs_connectivity_retry(&manager, attempt, &result) {
                    self.schedule_refresh_detail(
                        manager,
                        DetailRefreshAttempt::AutomaticConnectivityRetry,
                        claim,
                        scope,
                    );
                    return Produces::ok(());
                }

                let completion = self.detail_workflow.complete_refresh(claim);
                if completion.apply
                    && let Some(result) = result
                {
                    apply_refresh_detail_result(&manager, scope, &result);
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
            apply_refresh_detail_result(&manager, detail_claim.scope(), &result);
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn start_enter_detail(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.detail_workflow.open();
        self.ensure_supplemental_inventory_discovery(manager.clone());

        // entering detail only loads the inventory; it never asks for the passkey, so a missing
        // key shows as unchecked rows until the user starts a verification
        let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Tap);
        self.handle_detail_refresh_plan(manager, plan);

        Produces::ok(())
    }

    fn ensure_supplemental_inventory_discovery(&mut self, manager: Arc<RustCloudBackupManager>) {
        if !matches!(manager.state.read().status(), CloudBackupStatus::Enabled) {
            return;
        }

        if self.active_other_backups_scan.is_none()
            && matches!(
                manager.inventory.read().other_backups_state(),
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

        let scope = manager.inventory_scope();
        manager.apply_other_backups_state(scope, CloudBackupOtherBackupsState::Checking);
        let scan_manager = manager;
        addr.send_fut_with(move |addr| async move {
            let cloud = CloudStorage::global_silent_client();
            let state = scan_manager.other_backup_state(&cloud).await;
            send!(addr.complete_other_backups_scan(claim, scope, state));
        });
    }

    async fn complete_other_backups_scan(
        &mut self,
        claim: OtherBackupsScanClaim,
        scope: InventoryScope,
        state: CloudBackupOtherBackupsState,
    ) -> ActorResult<()> {
        if self.active_other_backups_scan != Some(claim) {
            return Produces::ok(());
        }

        self.active_other_backups_scan = None;
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        // a scan of an earlier configuration counted that one's other backups, so the current
        // configuration scans again
        if manager.apply_other_backups_state(scope, state) {
            self.ensure_supplemental_inventory_discovery(manager);
        }

        Produces::ok(())
    }

    /// Loads the inventory again after a configuration change dropped the old one's, when the
    /// detail is open
    pub async fn reload_inventory_for_new_scope(&mut self) -> ActorResult<()> {
        self.reload_open_inventory();
        Produces::ok(())
    }

    /// Loads the inventory again, with nobody tapping, when the detail refresh cycle is open
    fn reload_open_inventory(&mut self) {
        let Some(manager) = self.manager() else { return };
        if !self.detail_workflow.is_open() {
            return;
        }

        self.ensure_supplemental_inventory_discovery(manager.clone());
        let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Automatic);
        self.handle_detail_refresh_plan(manager, plan);
    }

    pub async fn start_master_key_upload_confirmation_grace(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        self.sync_health.start_master_key_upload_confirmation_grace(namespace_id);
        Produces::ok(())
    }

    pub async fn request_sync_health_refresh(&mut self) -> ActorResult<()> {
        self.sync_health.request_sync_health_refresh();
        Produces::ok(())
    }

    pub async fn schedule_wallet_upload(
        &mut self,
        wallet_id: WalletId,
        immediate: bool,
    ) -> ActorResult<()> {
        self.uploads.schedule_wallet_upload(wallet_id, immediate);
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
                self.resume_disable_automatically(AutomaticResume::CloudReachable).await?;
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
        self.uploads.resume_wallet_uploads_from_persisted_state();
        self.uploads.ensure_pending_upload_verification_loop();
        Produces::ok(())
    }

    pub async fn resume_wallet_uploads_from_persisted_state(&mut self) -> ActorResult<()> {
        self.uploads.resume_wallet_uploads_from_persisted_state();
        Produces::ok(())
    }

    pub async fn ensure_pending_upload_verification_loop(&mut self) -> ActorResult<()> {
        self.uploads.ensure_pending_upload_verification_loop();
        Produces::ok(())
    }

    pub async fn wake_pending_upload_verifier(&mut self) -> ActorResult<()> {
        self.uploads.wake_pending_upload_verifier();
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
                    // off the actor: the owner's record isn't reachable, and an unreadable or
                    // unnormalized record leaves the status to the owner's projection
                    let status = RustCloudBackupManager::runtime_status_for(
                        &RustCloudBackupManager::load_persisted_state(),
                    );
                    if let Some(status) = status {
                        let _ = operation.apply_status(status).await;
                    }
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
                    // off the actor: the owner's record isn't reachable, and an unreadable or
                    // unnormalized record leaves the status to the owner's projection
                    let status = RustCloudBackupManager::runtime_status_for(
                        &RustCloudBackupManager::load_persisted_state(),
                    );
                    if let Some(status) = status {
                        let _ = operation.apply_status(status).await;
                    }
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
        manager.reconcile_runtime_status(self.runtime.status());
        tracing::info!("restore_from_cloud_backup: cancellation requested for active restore");
        Produces::ok(())
    }

    pub async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.clear_worker_runtime_state();
        Produces::ok(())
    }

    /// Clears the upload and sync-health workers' timers and retry state
    ///
    /// The workers handle their mailboxes in order, so each clears before any message the
    /// supervisor sends it after this
    fn clear_worker_runtime_state(&mut self) {
        self.enable_continuation = None;
        self.sync_health.clear_upload_runtime_state();
        self.uploads.clear_upload_runtime_state();
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
        self.clear_worker_runtime_state();

        // the reset owns the work queued before it: a detail refresh still waiting on its rate
        // limit, or in flight, would list the cloud through a client that may ask the user for
        // consent, for a configuration the reset is clearing
        self.detail_workflow.suspend();
        Produces::ok(Ok(receiver))
    }

    /// Clears Cloud Backup's local keychain state for a full wipe, as the namespace's owner
    pub async fn clear_local_state_for_reset(
        &mut self,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        // the owner turns Cloud Backup off and records the clear before it starts, since even a
        // partial clear is final
        if let Err(refused) = self.runtime.clear_keys_in_reset() {
            return Produces::ok(Err(refused.into()));
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

    /// Resets Cloud Backup on this device under a ready reset, after its record was corrupted
    ///
    /// Clears this device's Cloud Backup keychain items and sync rows, then replaces the corrupted
    /// record with Cloud Backup off. Nothing in the cloud is touched, so the backup there can
    /// still be restored or enabled again
    pub async fn reset_corrupted_on_this_device(
        &mut self,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !matches!(*self.runtime, Runtime::Resetting { held: None, .. }) {
            return Produces::ok(Err(CloudBackupError::Internal(
                "cloud backup can only be reset on this device by a ready reset".into(),
            )));
        }

        let owner = OwnerToken::mint();
        if let Err(error) =
            RustCloudBackupManager::run_disable_local_cleanup(&LocalStateOwner::Lifecycle(&owner))
        {
            return Produces::ok(Err(error));
        }

        let reset = self.runtime.commit(LifecycleWriter::reset_corrupted).map_err(|source| {
            CloudBackupError::internal_context("reset corrupted cloud backup record", source)
        });

        let result = match reset {
            Ok(committed) if committed.outcome => Ok(()),
            Ok(_) => Err(CloudBackupError::Internal(
                "cloud backup record was no longer corrupted when it was reset".into(),
            )),
            Err(error) => Err(error),
        };

        Produces::ok(result)
    }

    /// A reset on this device failed before it committed; the failure shows by its control
    pub async fn reset_on_this_device_failed(
        &mut self,
        error: CloudBackupError,
    ) -> ActorResult<()> {
        warn!("Cloud Backup could not reset on this device: {error}");
        if self.screen_outcome.action_failed(CloudBackupScreenAction::ResetOnThisDevice, &error)
            && let Some(manager) = self.manager()
        {
            manager.request_screen_publish();
        }

        Produces::ok(())
    }

    /// Ends a local reset with the body it left, and reopens admission; `None` when no reset ran
    ///
    /// After a wipe Cloud Backup is off, whatever the record said before; the owner sets that
    /// here so an idle body never waits for the next admission to catch up with the wipe
    pub async fn finish_local_reset(
        &mut self,
        outcome: LocalResetOutcome,
    ) -> ActorResult<Option<ResetEnd>> {
        let end = self.runtime.finish_reset(outcome);
        debug!("Cloud Backup local reset finished: {outcome:?}, {end:?}");

        match (end, self.manager()) {
            // a reset that left Cloud Backup off cleared the keys and ended any switch with the
            // record, so a step that failed before it no longer needs the user
            (
                Some(ResetEnd::Wiped | ResetEnd::Abandoned(AbandonedReset::Disabled)),
                Some(manager),
            ) => {
                self.drive_account_switch_ended_by_reset(&manager);
            }

            // one that kept the keys keeps its switch, but dropped the paused work's material with
            // the rest of its in-memory state, and may have cut off a finalization between its
            // local promotion and its commit or rollback. The journal of both is taken up as a
            // relaunch takes it up, before admission reopens, and a switch whose promotion it
            // finishes goes on to its commit; a held recovery already owns that evidence
            (Some(ResetEnd::Abandoned(AbandonedReset::Resumed)), Some(manager))
                if self.runtime.admission_body() != AdmissionBody::PendingEnableRecovery =>
            {
                self.recover_journaled_enable(&manager);
            }

            _ => {}
        }

        // the reset dropped the cycle's queued work, and a scope reload it queued found the cycle
        // suspended, so a resumed backup loads its inventory again, with nobody tapping
        let resumed = end == Some(ResetEnd::Abandoned(AbandonedReset::Resumed));
        if self.detail_workflow.end_suspension(resumed) {
            self.reload_open_inventory();
        }

        Produces::ok(end)
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

/// What the owner left when a local reset ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetEnd {
    /// The database reset committed, so Cloud Backup is off
    Wiped,
    Abandoned(AbandonedReset),
}

/// What an abandoned local reset left
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbandonedReset {
    /// The local keys were kept, so the backup is as it was before the reset
    Resumed,
    /// The owner committed Cloud Backup off and the local keys were cleared
    Disabled,
}

/// Why the owner didn't let a reset clear the local keys
#[derive(Debug, thiserror::Error)]
pub(crate) enum KeyClearRefused {
    #[error("cloud backup local state can only be cleared by a ready reset")]
    NoReadyReset,
    /// The keys stay because the record could not say Cloud Backup is off before they went
    #[error("turn cloud backup off before clearing its local keys: {0}")]
    NotTurnedOff(#[from] DatabaseError),
}

impl From<KeyClearRefused> for CloudBackupError {
    fn from(refused: KeyClearRefused) -> Self {
        Self::internal_context("clear cloud backup local state for a reset", refused)
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::verification::VerificationRun;
    use super::*;
    use crate::manager::cloud_backup_manager::actors::uploads::CloudBackupUploadWorker;
    use crate::manager::cloud_backup_manager::turn_off::TurnOffReview;
    use crate::manager::{
        cloud_backup_manager::{
            CloudBackupPublishedScreen, CloudBackupReconcileMessage, CloudBackupScreenDelta,
            screen::CloudBackupScreen,
        },
        deferred_sender::SingleOrMany,
    };

    impl CloudBackupKeychain {
        /// Clears local state the way the lifecycle owner does, for test setup
        pub(crate) fn clear_local_state_for_test(
            &self,
        ) -> Result<(), crate::manager::cloud_backup_manager::keychain::CloudBackupKeychainError>
        {
            self.clear_local_state(&LocalStateOwner::Lifecycle(&OwnerToken::mint()))
        }
    }

    impl CloudBackupSupervisor {
        /// The screen this supervisor would publish now, for tests that drive it directly
        pub(crate) fn projected_screen(
            &self,
            manager: &RustCloudBackupManager,
        ) -> CloudBackupScreen {
            manager.project_screen(&self.runtime, &self.screen_inputs(manager))
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

    /// One dispatched intent, how many admissions its handler made, and what it started
    #[derive(Debug, Clone)]
    pub(crate) struct DispatchRecord {
        pub(crate) intent: UserIntent,
        pub(crate) admissions: u32,
        /// What the dispatch's admission decided
        pub(crate) admission: Option<Admission>,
        /// The operation running when the dispatch began, and when its turn ended
        pub(crate) running_before: Option<CloudBackupExclusiveOperationClaim>,
        pub(crate) running_after: Option<CloudBackupExclusiveOperationClaim>,
        /// A Verify that joined uploads already waiting for confirmation instead of starting
        /// its own operation
        pub(crate) joined_upload_confirmation: bool,
    }

    /// Admissions counted across every caller of `admit`, and the dispatches since the last read
    #[derive(Debug, Default)]
    pub(crate) struct DispatchLog {
        pub(crate) admissions: u32,
        /// Set by the dispatch in progress when its Verify joined an upload confirmation
        pub(crate) joined_upload_confirmation: bool,
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
        /// The screen as the platform sees it, while the manager is alive
        pub(crate) screen: Option<ScreenView>,
        /// A local reset holds the body, so the screen's exits are closed on purpose
        pub(crate) resetting: bool,
        /// A Turn Off review is gathering its evidence or waiting on Check Again's
        /// classification, which its held calls and its evidence deadline end
        pub(crate) preparing_turn_off_review: bool,
    }

    /// The published screen and the deltas that reached the platform, read in one handler so no
    /// publish lands between them
    #[derive(Debug)]
    pub(crate) struct ScreenView {
        /// What `state()` gave at the start of this view, before its publish and drain, as a
        /// platform starting now reads it: every delta sent before it is still queued
        pub(crate) snapshot: CloudBackupPublishedScreen,
        /// The screen last sent to the platform
        pub(crate) published: CloudBackupScreen,
        /// The revision `published` was sent at
        pub(crate) revision: u64,
        /// The screen a projection gives now
        pub(crate) fresh: CloudBackupScreen,
        /// Every screen delta sent since the last view, oldest first
        pub(crate) sent: Vec<CloudBackupScreenDelta>,
        /// The inventory rows the screen was drawn from, with the coverage the screen leaves out
        pub(crate) inventory:
            Option<crate::manager::cloud_backup_manager::inventory::InventoryRows>,
    }

    impl CloudBackupSupervisor {
        pub(crate) fn begin_dispatch_record(&mut self, intent: &UserIntent) -> OpenDispatch {
            self.invariant_log.joined_upload_confirmation = false;
            OpenDispatch(DispatchRecord {
                intent: intent.clone(),
                admissions: self.invariant_log.admissions,
                admission: None,
                running_before: self.runtime.claim(),
                running_after: None,
                joined_upload_confirmation: false,
            })
        }

        pub(crate) fn finish_dispatch_record(&mut self, OpenDispatch(mut record): OpenDispatch) {
            record.admissions = self.invariant_log.admissions - record.admissions;
            record.admission = self
                .last_outcome
                .as_ref()
                .filter(|outcome| outcome.intent == record.intent)
                .map(|outcome| outcome.admission);
            record.running_after = self.runtime.claim();
            record.joined_upload_confirmation =
                std::mem::take(&mut self.invariant_log.joined_upload_confirmation);
            self.invariant_log.dispatches.push(record);
        }

        /// Starts a verification operation outside dispatch, as detail entry does
        pub async fn start_verification(&mut self, force_discoverable: bool) -> ActorResult<()> {
            if let Some(manager) = self.manager() {
                let started = self.begin_manual_verification(manager, force_discoverable);
                assert!(started, "a test started a verification while {:?} runs", *self.runtime);
            }

            Produces::ok(())
        }

        /// Allocates a manual verification's operation and its run without running it, for tests
        /// that run the verification pass themselves
        pub async fn begin_verification_run_for_test(
            &mut self,
            force_discoverable: bool,
        ) -> ActorResult<VerificationRun> {
            let manager = self.manager().expect("cloud backup manager exists");
            let claim = self
                .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Verify)
                .expect("begin verification operation");

            Produces::ok(VerificationRun {
                claim,
                continuation: DeepVerificationContinuation::Manual {
                    force_discoverable,
                    attempt: VerificationAttempt::Initial,
                },
                basis: self.evidence_basis(),
                read_at: manager.inventory_read_at(),
            })
        }

        /// Reads the supervisor for the harness invariants, taking what it logged since the
        /// last read
        pub(crate) async fn harness_invariant_view(&mut self) -> ActorResult<SupervisorView> {
            let shown_operation =
                self.manager().and_then(|manager| manager.state.read().active_operation());

            let screen = self.manager().map(|manager| self.harness_screen_view(&manager));
            Produces::ok(SupervisorView {
                owner: self.runtime.take_view(),
                dispatches: std::mem::take(&mut self.invariant_log.dispatches),
                shown_operation,
                uploads: self.uploads.addr_for_test().clone(),
                screen,
                resetting: matches!(*self.runtime, Runtime::Resetting { .. }),
                preparing_turn_off_review: self.prepares_turn_off_review(),
            })
        }

        /// A preparing review is not an operation, but like one it waits only on held calls and
        /// on timers: its listing and Check Again's classification, each bounded by the
        /// evidence deadline
        fn prepares_turn_off_review(&self) -> bool {
            matches!(self.runtime.turn_off_review(), Some(TurnOffReview::Preparing { .. }))
        }

        /// The screen view at this boundary
        ///
        /// A publish already requested runs first, as its queued handler would once this one
        /// returns; one never requested is left out, so a change that forgets to publish still
        /// shows up as a published screen behind the projection
        fn harness_screen_view(&self, manager: &RustCloudBackupManager) -> ScreenView {
            let snapshot = manager.state();
            if manager.screen_publish_requested() {
                manager.publish_screen(&self.runtime, || self.screen_inputs(manager));
            }

            let (screens, others): (Vec<_>, Vec<_>) = manager
                .reconciler
                .receiver()
                .try_iter()
                .flat_map(|field| match field {
                    SingleOrMany::Single(message) => vec![message],
                    SingleOrMany::Many(messages) => messages,
                })
                .partition(|message| matches!(message, CloudBackupReconcileMessage::Screen(_)));
            let sent = screens
                .into_iter()
                .filter_map(|message| match message {
                    CloudBackupReconcileMessage::Screen(delta) => Some(*delta),
                    _ => None,
                })
                .collect();

            // a test playing the platform reads the requests this read passed over
            crate::manager::cloud_backup_manager::set_aside_reconcile_messages(others);

            let CloudBackupPublishedScreen { revision, screen: published } = manager.state();
            ScreenView {
                snapshot,
                published,
                revision,
                fresh: self.projected_screen(manager),
                sent,
                inventory: manager.inventory_facts().view().rows,
            }
        }

        /// What dispatch would do with `intent` now, without doing it
        ///
        /// `None` once the manager is gone, since no admission reason describes that
        pub(crate) async fn harness_admission(
            &mut self,
            intent: crate::manager::cloud_backup_manager::admission::UserIntent,
        ) -> ActorResult<Option<crate::manager::cloud_backup_manager::admission::Admission>>
        {
            let admission = self.manager().map(|manager| self.decide(&manager, &intent));
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

        /// Whether work that only held calls and timers end holds the runtime now: an
        /// operation, or a Turn Off review preparing its evidence
        pub(crate) async fn harness_holds_live_work(&mut self) -> ActorResult<bool> {
            Produces::ok(self.runtime.claim().is_some() || self.prepares_turn_off_review())
        }

        /// The interrupted-Enable recovery the runtime body holds now
        pub(crate) async fn harness_recovery(
            &mut self,
        ) -> ActorResult<Option<CloudBackupPendingEnableRecovery>> {
            Produces::ok(self.runtime.pending_enable_recovery().cloned())
        }

        /// The running operation's claim, which carries its `OperationId`
        pub(crate) async fn harness_running_claim(
            &mut self,
        ) -> ActorResult<Option<CloudBackupExclusiveOperationClaim>> {
            Produces::ok(self.runtime.claim())
        }

        /// Finishes `claim` at the owner's finish boundary, as a duplicate completion would
        pub(crate) async fn harness_finish(
            &mut self,
            claim: CloudBackupExclusiveOperationClaim,
        ) -> ActorResult<Finish> {
            Produces::ok(self.runtime.finish(claim))
        }

        pub async fn run_wallet_upload_inline_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<bool> {
            let uploads = self.uploads.addr_for_test();
            let ran = call!(uploads.run_wallet_upload_inline_for_test(wallet_id)).await?;
            Produces::ok(ran)
        }

        pub async fn wallet_upload_retry_count_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<u32> {
            let uploads = self.uploads.addr_for_test();
            let count = call!(uploads.wallet_upload_retry_count_for_test(wallet_id)).await?;
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
            Produces::ok(self.enable_continuation.take().map(EnableContinuation::into_session))
        }

        /// Stores `session` as the paused work of `owner`
        pub async fn replace_enable_continuation_for_test(
            &mut self,
            owner: ContinuationOwner,
            session: PendingEnableSession,
        ) -> ActorResult<()> {
            if let Some(manager) = self.manager() {
                manager.pending_enable.adopt(owner);
            }

            self.enable_continuation = Some(EnableContinuation::new(owner, session));
            Produces::ok(())
        }

        pub async fn has_awaiting_saved_passkey_confirmation_for_test(&self) -> ActorResult<bool> {
            Produces::ok(self.enable_continuation.is_some())
        }

        /// Runs Discard's cleanup on the held continuation, for journal states that are no
        /// saved-passkey pause, so admission offers no Discard in them
        pub async fn discard_held_continuation_for_test(&mut self) -> ActorResult<()> {
            if let Some(continuation) = self.enable_continuation.take() {
                self.discard_enable_continuation(continuation);
            }

            Produces::ok(())
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

        /// Answers once the upload and sync-health workers handled every message the supervisor
        /// sent them before this, since the supervisor only sends to them
        pub async fn workers_checkpoint_for_test(&mut self) -> ActorResult<()> {
            let uploads = self.uploads.addr_for_test();
            call!(uploads.checkpoint_for_test()).await?;
            let sync_health = self.sync_health.addr_for_test();
            call!(sync_health.checkpoint_for_test()).await?;
            Produces::ok(())
        }

        /// Whether this supervisor's upload verifier is stopped and its sync-health worker has no
        /// refresh in flight or queued
        ///
        /// Answers at once instead of waiting, so a refresh the verifier requests from this
        /// supervisor while a test polls is never held behind the poll in the mailbox
        pub(crate) async fn background_refreshes_idle_for_test(&mut self) -> ActorResult<bool> {
            let uploads = self.uploads.addr_for_test();
            if !call!(uploads.pending_upload_verifier_stopped_for_test()).await? {
                return Produces::ok(false);
            }

            let sync_health = self.sync_health.addr_for_test();
            let idle = call!(sync_health.sync_health_refresh_idle_for_test()).await?;
            Produces::ok(idle)
        }

        /// The claim of the operation that holds the runtime now
        pub(crate) async fn operation_claim_for_test(
            &mut self,
        ) -> ActorResult<Option<CloudBackupExclusiveOperationClaim>> {
            Produces::ok(self.runtime.claim())
        }

        pub async fn cleanup_idle_for_test(&mut self) -> ActorResult<bool> {
            let cleanup = self.cleanup.addr_for_test();
            let idle = call!(cleanup.is_idle_for_test()).await?;
            Produces::ok(idle)
        }

        pub async fn enqueue_cleanup_for_test(
            &mut self,
            job: CloudBackupCleanupJob,
        ) -> ActorResult<()> {
            let cleanup = self.cleanup.addr_for_test();
            call!(cleanup.enqueue_cleanup(job)).await?;
            Produces::ok(())
        }
    }
}
