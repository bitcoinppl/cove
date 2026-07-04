//! Top-level Cloud Backup operation supervisor
//!
//! This actor owns exclusive operation lifecycles and delegates slow work to
//! child actors or spawned tasks. Each exclusive operation receives a claim that
//! every async completion must present before it can update manager state

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, call, send};
use cove_cspp::{backup_data::MASTER_KEY_RECORD_ID, master_key_crypto};
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_tokio::task::spawn_actor;
use tracing::{error, info, warn};

use super::CloudBackupSyncHealthWorker;
use super::cleanup::{CleanupSourceNamespace, CloudBackupCleanupJob, CloudBackupCleanupWorker};
use super::restore::{self, CloudBackupRestoreEvent, RestoreOperation, RestoredPasskeyMaterial};
use super::uploads::CloudBackupUploadWorker;
use super::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteBlocker,
    CloudBackupWriteClient, CloudBackupWriteCommandResult, CloudBackupWriteCompletion,
    CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor,
};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::database::cloud_backup::{CloudBackupRecordKey, PersistedDisablingCloudBackup};
use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::verify::coordinator::CloudBackupVerificationCoordinator;
use crate::manager::cloud_backup_manager::verify::{
    CloudBackupDeepVerificationAutoSyncCompletion, CloudBackupDeepVerificationStep,
    CloudBackupPasskeyRepairFinalization, CloudBackupPendingDeepVerificationAutoSyncResume,
    CloudBackupPendingDeepVerificationResume, CloudBackupPreparedDeepVerificationAutoSync,
    CloudBackupPreparedDeepVerificationWrapperRepair, CloudBackupPreparedPasskeyWrapperRepair,
    CloudBackupUploadedDeepVerificationAutoSync, CloudBackupUploadedPasskeyWrapperRepair,
};
use crate::manager::cloud_backup_manager::wallets::{
    UnpersistedPrfKey, WalletRestoreOutcome, delay_before_new_passkey_auth,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupCloudOnlyFetchOutcome, CloudBackupCloudOnlyOperationWarning,
    CloudBackupCloudOnlyWalletOutcome, CloudBackupDetailOutcome, CloudBackupDetailResult,
    CloudBackupDisableOutcome, CloudBackupDisablePreparation, CloudBackupEnableContext,
    CloudBackupEnableOutcome, CloudBackupEnablePasskeyPreparation,
    CloudBackupEnablePasskeyRegistration, CloudBackupEnablePreparation,
    CloudBackupEnableRecoveryCompletion, CloudBackupEnableRecoveryPreparation, CloudBackupError,
    CloudBackupKeepEnabledPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupOtherBackupsOutcome, CloudBackupPasskeyChoiceIntent,
    CloudBackupPreparedCloudWalletDelete, CloudBackupReadyEnableUpload, CloudBackupRecoveryOutcome,
    CloudBackupRegisteredEnablePasskey, CloudBackupRestoreOutcome, CloudBackupRestoreReport,
    CloudBackupReuploadedWallets, CloudBackupSavedPasskeyConfirmation, CloudBackupStatus,
    CloudBackupSyncOutcome, CloudBackupUploadedEnableBackup, CloudBackupVerificationOutcome,
    CloudBackupVerificationPresentation, CloudBackupVerificationSource, CloudBackupWalletItem,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult,
    EnablePasskeyRegistrationFlow, OtherBackupsOperation, PendingEnableSession,
    PendingUploadVerificationState, PendingVerificationCompletion, PendingVerificationUpload,
    RecoveryAction, RustCloudBackupManager, SavedPasskeyConfirmationMode, VerificationState,
    WalletId, blocking_cloud_error,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

mod cloud_only;
mod disable;
mod enable;
mod verification;

mod tests {
    #![cfg(test)]

    include!("supervisor/tests.rs");
}

static NEXT_SUPERVISOR_OPERATION_ID: AtomicU64 = AtomicU64::new(0);

/// User or system operation requested of the Cloud Backup supervisor
#[derive(Debug, Clone)]
pub(crate) enum CloudBackupOperation {
    Enable(CloudBackupEnableContext),
    EnableForceNew(CloudBackupEnableContext),
    EnableNoDiscovery(CloudBackupEnableContext),
    Recovery(RecoveryAction),
    RepairPasskey { no_discovery: bool },
    Sync,
    FetchCloudOnly,
    Disable,
    RestoreCloudWallet,
    DeleteCloudWallet,
    RecoverOtherBackups,
    DeleteOtherBackups,
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
        CloudBackupDetailResult::Success(detail) => {
            manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail.clone()));
        }
        CloudBackupDetailResult::AccessError(error) => {
            error!("Failed to refresh detail: {error}");
        }
    }
}

fn apply_cloud_only_operation_refresh_detail_result(
    manager: &RustCloudBackupManager,
    result: &CloudBackupDetailResult,
) {
    match result {
        CloudBackupDetailResult::Success(detail) => {
            manager.apply_detail_outcome_preserving_cloud_only_if_consistent(
                CloudBackupDetailOutcome::Refreshed(detail.clone()),
            );
        }
        CloudBackupDetailResult::AccessError(error) => {
            error!("Failed to refresh detail: {error}");
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
    disabling: PersistedDisablingCloudBackup,
}

/// Actor that owns Cloud Backup operation exclusivity and async completions
#[derive(Debug)]
pub(crate) struct CloudBackupSupervisor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    cleanup: Addr<CloudBackupCleanupWorker>,
    sync_health: Addr<CloudBackupSyncHealthWorker>,
    uploads: Addr<CloudBackupUploadWorker>,
    write: Addr<CloudBackupWriteSupervisor>,
    active_operation: Option<CloudBackupExclusiveOperationClaim>,
    pending_enable_session: Option<PendingEnableSession>,
    pending_verification_completion: Option<PendingVerificationCompletion>,
    next_request_id: u64,
    active_sync_request: Option<u64>,
    active_cloud_only_fetch_request: Option<u64>,
    pending_disable_write_drain: Option<PendingDisableWriteDrain>,
    // runtime-only authorization produced by this app session for the active namespace
    // clearing it when the supervisor is recreated makes detail entry re-check passkey availability
    runtime_passkey_authorization: Option<RuntimePasskeyAuthorization>,
}

#[async_trait::async_trait]
impl Actor for CloudBackupSupervisor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupSupervisor {
    pub(crate) fn new(
        manager: Weak<RustCloudBackupManager>,
        cloud_writes: Addr<CloudBackupWriteSupervisor>,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            cleanup: spawn_actor(CloudBackupCleanupWorker::new(manager.clone())),
            sync_health: spawn_actor(CloudBackupSyncHealthWorker::new(manager.clone())),
            uploads: spawn_actor(CloudBackupUploadWorker::new(manager.clone())),
            write: cloud_writes,
            manager,
            active_operation: None,
            pending_enable_session: None,
            pending_verification_completion: None,
            next_request_id: 0,
            active_sync_request: None,
            active_cloud_only_fetch_request: None,
            pending_disable_write_drain: None,
            runtime_passkey_authorization: None,
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Option<Addr<Self>> {
        Some(self.addr.upgrade())
    }

    async fn await_cloud_backup_write_for_operation<T>(
        receiver: CloudBackupWriteResultReceiver<T>,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<T, CloudBackupError> {
        let result: CloudBackupWriteCommandResult<T> = receiver.await.map_err(|source| {
            CloudBackupError::internal_context("wait for cloud backup write supervisor", source)
        })?;
        let context = result.context();
        if context.origin() != Some(origin) {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor returned mismatched operation origin for command {:?}",
                context.id()
            )
            .into()));
        }

        result.into_result()
    }

    async fn delete_prepared_cloud_wallet_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        prepared: CloudBackupPreparedCloudWalletDelete,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        let receiver = call!(write.delete_active_wallet_backup_for_operation(
            prepared.cloud,
            prepared.namespace,
            prepared.record_id.clone(),
            origin
        ))
        .await
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        Self::await_cloud_backup_write_for_operation(receiver, origin).await.map_err(|error| {
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
        let receiver = call!(write.delete_namespace_for_operation(
            CloudStorage::global_explicit_client(),
            namespace,
            origin
        ))
        .await
        .map_err(|source| {
            CloudBackupError::internal_context("start cloud backup write supervisor", source)
        })?;

        Self::await_cloud_backup_write_for_operation(receiver, origin).await
    }

    async fn apply_cloud_backup_write_completion_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        let receiver = call!(write.apply_completion_for_operation(completion, origin))
            .await
            .map_err(|source| {
                CloudBackupError::internal_context("start cloud backup write supervisor", source)
            })?;

        Self::await_cloud_backup_write_for_operation(receiver, origin).await
    }

    fn begin_exclusive_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        operation: CloudBackupExclusiveOperation,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if self.active_operation.is_some() {
            return None;
        }

        let operation_id = NEXT_SUPERVISOR_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
        let claim = CloudBackupExclusiveOperationClaim::new(operation, operation_id);
        manager.project_exclusive_operation_started(claim);
        self.active_operation = Some(claim);
        Some(claim)
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        self.active_operation = None;
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        self.active_operation = None;
        if let Some(manager) = self.manager() {
            manager.project_exclusive_operation_failed(claim, &error);
        }

        Produces::ok(())
    }

    fn restore_operation_is_current(&self, claim: CloudBackupExclusiveOperationClaim) -> bool {
        self.active_operation == Some(claim)
            && claim.operation() == CloudBackupExclusiveOperation::Restore
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

    pub async fn apply_restore_enable_outcome(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        outcome: CloudBackupEnableOutcome,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        manager.apply_enable_outcome(outcome);
        Produces::ok(Ok(()))
    }

    pub async fn persist_restore_cloud_backup_state(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        state: PersistedCloudBackupState,
        context: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };

        let result = Database::global()
            .cloud_backup_state
            .set(&state)
            .map_err(|error| CloudBackupError::Internal(format!("{context}: {error}").into()));
        if result.is_ok() {
            manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(&state));
            manager.refresh_persisted_flags();
        }

        Produces::ok(result)
    }

    pub async fn save_restore_keychain_state(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_operation_is_current(claim) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = restore::save_restore_keychain_entries(master_key, passkey, namespace_id);
        Produces::ok(result)
    }

    fn spawn_operation(&mut self, operation: CloudBackupOperation, record_id: Option<String>) {
        match operation {
            CloudBackupOperation::Enable(context) => self.start_enable_operation(context),
            CloudBackupOperation::EnableForceNew(context) => {
                self.start_enable_force_new_operation(context);
            }
            CloudBackupOperation::EnableNoDiscovery(context) => {
                self.start_enable_no_discovery_operation(context);
            }
            CloudBackupOperation::Recovery(action) => self.start_recovery_operation(action),
            CloudBackupOperation::RepairPasskey { no_discovery } => {
                self.start_repair_passkey_operation(no_discovery);
            }
            CloudBackupOperation::Sync => {
                let Some(manager) = self.manager() else { return };
                self.start_sync_request(manager);
            }
            CloudBackupOperation::FetchCloudOnly => self.start_cloud_only_fetch_request(),
            CloudBackupOperation::Disable => self.start_disable_operation(),
            CloudBackupOperation::RestoreCloudWallet => {
                let Some(record_id) = record_id else { return };
                self.start_restore_cloud_wallet_operation(record_id);
            }
            CloudBackupOperation::DeleteCloudWallet => {
                let Some(record_id) = record_id else { return };
                self.start_delete_cloud_wallet_operation(record_id);
            }
            CloudBackupOperation::RecoverOtherBackups => {
                self.start_recover_other_backups_operation()
            }
            CloudBackupOperation::DeleteOtherBackups => self.start_delete_other_backups_operation(),
        }
    }

    fn start_sync_request(&mut self, manager: Arc<RustCloudBackupManager>) {
        let request_id = self.next_request_id();
        self.active_sync_request = Some(request_id);
        manager.apply_sync_outcome(CloudBackupSyncOutcome::Started);

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
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_sync_request_refresh_detail(request_id, result));
                });
            }
            Err(error) => {
                manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(error.to_string()));
                self.active_sync_request = None;
            }
        }

        Produces::ok(())
    }

    pub async fn complete_sync_request_refresh_detail(
        &mut self,
        request_id: u64,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        if self.active_sync_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_sync_request = None;
            return Produces::ok(());
        };

        if let Some(result) = result {
            apply_refresh_detail_result(&manager, &result);
        }

        manager.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        self.active_sync_request = None;
        Produces::ok(())
    }

    pub async fn start_operation(
        &mut self,
        operation: CloudBackupOperation,
        record_id: Option<String>,
    ) -> ActorResult<()> {
        self.spawn_operation(operation, record_id);
        Produces::ok(())
    }

    pub async fn start_refresh_detail(&mut self) -> ActorResult<()> {
        self.start_refresh_detail_with_context(DetailRefreshAttempt::Initial).await
    }

    async fn start_refresh_detail_with_context(
        &mut self,
        attempt: DetailRefreshAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.schedule_refresh_detail(manager, attempt);

        Produces::ok(())
    }

    fn schedule_refresh_detail(
        &self,
        manager: Arc<RustCloudBackupManager>,
        attempt: DetailRefreshAttempt,
    ) {
        manager.refresh_sync_health();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.refresh_cloud_backup_detail().await;
            send!(addr.complete_refresh_detail(result, attempt));
        });
    }

    pub async fn complete_refresh_detail(
        &mut self,
        result: Option<CloudBackupDetailResult>,
        attempt: DetailRefreshAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if refresh_detail_needs_connectivity_retry(&manager, attempt, &result) {
            self.schedule_refresh_detail(manager, DetailRefreshAttempt::AutomaticConnectivityRetry);
            return Produces::ok(());
        }

        if let Some(result) = result {
            apply_refresh_detail_result(&manager, &result);
        }

        Produces::ok(())
    }

    pub async fn complete_operation_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        if let Some(result) = result {
            if matches!(
                claim.operation(),
                CloudBackupExclusiveOperation::RestoreCloudWallet
                    | CloudBackupExclusiveOperation::DeleteCloudWallet
            ) {
                apply_cloud_only_operation_refresh_detail_result(&manager, &result);
            } else {
                apply_refresh_detail_result(&manager, &result);
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn start_enter_detail(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let plan = self.detail_entry_plan(&manager);
        match plan {
            DetailEntryPlan::StartPasskeyVerification { force_discoverable } => {
                if let Some(addr) = self.addr() {
                    send!(addr.start_verification(force_discoverable));
                }
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

        manager.refresh_sync_health();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.refresh_cloud_backup_detail().await;
            send!(addr.complete_enter_detail(result));
        });

        Produces::ok(())
    }

    pub async fn complete_enter_detail(
        &mut self,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        if let Some(result) = result {
            apply_refresh_detail_result(&manager, &result);
        }

        Produces::ok(())
    }

    fn detail_entry_plan(&self, manager: &RustCloudBackupManager) -> DetailEntryPlan {
        let state = manager.state.read();
        if !matches!(state.status(), CloudBackupStatus::Enabled) {
            return DetailEntryPlan::RefreshOnly;
        }

        if matches!(
            state.verification(),
            VerificationState::Verifying
                | VerificationState::Verified(_)
                | VerificationState::PasskeyConfirmed
        ) {
            return DetailEntryPlan::ContinueRustOwnedVerification;
        }

        if let Some(completion) = self.pending_verification_completion.clone() {
            return DetailEntryPlan::ResumePendingUploadConfirmation(completion);
        }

        if let Some(authorization) = self.runtime_passkey_authorization_for_current_manager(manager)
        {
            return DetailEntryPlan::UseFreshEnableProof(authorization);
        }

        DetailEntryPlan::StartPasskeyVerification { force_discoverable: true }
    }

    fn runtime_passkey_authorization_for_current_manager(
        &self,
        manager: &RustCloudBackupManager,
    ) -> Option<RuntimePasskeyAuthorization> {
        let authorization = self.runtime_passkey_authorization.as_ref()?;
        let Ok(namespace_id) = manager.current_namespace_id() else {
            return None;
        };

        let cloud_keychain = CloudBackupKeychain::global();
        let credential_id = cloud_keychain.load_credential_id()?;
        let prf_salt = cloud_keychain.load_prf_salt()?;

        (authorization.namespace_id == namespace_id
            && authorization.credential_id == credential_id
            && authorization.prf_salt == prf_salt)
            .then(|| authorization.clone())
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
        self.pending_verification_completion = Some(completion);
        Produces::ok(())
    }

    pub async fn clear_pending_verification_completion(&mut self) -> ActorResult<()> {
        self.pending_verification_completion = None;
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

    pub async fn unblock_cloud_backup_writes(
        &mut self,
        blocker: CloudBackupWriteBlocker,
    ) -> ActorResult<()> {
        call!(self.write.unblock(blocker)).await?;
        Produces::ok(())
    }

    pub async fn start_restore_from_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
        else {
            warn!("restore_from_cloud_backup called while another operation is active, ignoring");
            return Produces::ok(());
        };

        let operation = RestoreOperation::new(claim, addr.clone());
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
        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
        else {
            warn!("restore_from_cloud_backup called while another operation is active, ignoring");
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

        let operation = RestoreOperation::new_with_events(claim, addr.clone(), sender);
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
                        .send_event_if_current(CloudBackupRestoreEvent::Failed(error.to_string()))
                        .await;
                    send!(addr.fail_exclusive_operation(claim, error));
                }
            }
        });

        Produces::ok(())
    }

    pub async fn cancel_restore(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Some(claim) = self.active_operation else {
            return Produces::ok(());
        };
        if claim.operation() != CloudBackupExclusiveOperation::Restore {
            return Produces::ok(());
        }

        let status = manager.state.read().status().clone();
        if !matches!(status, CloudBackupStatus::Restoring) {
            return Produces::ok(());
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));
        tracing::info!("restore_from_cloud_backup: cancelled active restore");
        Produces::ok(())
    }

    pub async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        self.pending_verification_completion = None;
        call!(self.sync_health.clear_upload_runtime_state()).await?;
        call!(self.uploads.clear_upload_runtime_state()).await?;
        Produces::ok(())
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::*;

    impl CloudBackupSupervisor {
        pub async fn run_wallet_upload_inline_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<()> {
            call!(self.uploads.run_wallet_upload_inline_for_test(wallet_id)).await?;
            Produces::ok(())
        }

        pub async fn new_restore_operation(&mut self) -> ActorResult<RestoreOperation> {
            let manager = self.manager().expect("cloud backup manager exists");
            let addr = self.addr().expect("cloud backup supervisor address exists");
            if let Some(claim) = self.active_operation.take() {
                manager.project_exclusive_operation_finished(claim);
            }
            let claim = self
                .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore)
                .expect("begin restore operation");
            let operation = RestoreOperation::new(claim, addr);
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
