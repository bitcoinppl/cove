use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, call, send};
use cove_cspp::{backup_data::MASTER_KEY_RECORD_ID, master_key_crypto};
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_tokio::task::spawn_actor;
use cove_util::ResultExt as _;
use tracing::{error, info, warn};

use crate::database::Database;
use crate::database::cloud_backup::CloudBackupRecordKey;
use crate::database::cloud_backup::PersistedCloudBackupState;

use super::super::keychain::CloudBackupKeychain;
use super::super::model::{CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim};
use super::super::verify::{
    CloudBackupDeepVerificationAutoSyncCompletion, CloudBackupDeepVerificationStep,
    CloudBackupPasskeyRepairFinalization, CloudBackupPendingDeepVerificationAutoSyncResume,
    CloudBackupPendingDeepVerificationResume, CloudBackupPreparedDeepVerificationAutoSync,
    CloudBackupPreparedDeepVerificationWrapperRepair, CloudBackupPreparedPasskeyWrapperRepair,
    CloudBackupUploadedDeepVerificationAutoSync, CloudBackupUploadedPasskeyWrapperRepair,
    coordinator::CloudBackupVerificationCoordinator,
};
use super::super::wallets::WalletRestoreOutcome;
use super::super::{
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
use super::cleanup::{CleanupSourceNamespace, CloudBackupCleanupJob, CloudBackupCleanupWorker};
use super::restore::{self, CloudBackupRestoreEvent, RestoreOperation, RestoredPasskeyMaterial};
use super::sync_health::CloudBackupSyncHealthWorker;
use super::uploads::CloudBackupUploadWorker;
use super::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWriteBlocker,
    CloudBackupWriteClient, CloudBackupWriteCommandResult, CloudBackupWriteCompletion,
    CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

static NEXT_SUPERVISOR_OPERATION_ID: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimePasskeyAuthorization {
    namespace_id: String,
    credential_id: Vec<u8>,
    prf_salt: [u8; 32],
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
pub(crate) enum DetailRefreshAttempt {
    Initial,
    AutomaticConnectivityRetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationAttempt {
    Initial,
    AutomaticConnectivityRetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingEnableUploadSelection {
    RetryOnly,
    RetryOrForceNewConfirmation,
}

#[derive(Debug)]
pub(crate) struct EnableRecoveryFinalization {
    namespace_id: String,
    active_critical_key: zeroize::Zeroizing<[u8; 32]>,
    cleanup_sources: Vec<CleanupSourceNamespace>,
}

pub(crate) struct EnableUploadFinalization {
    master_key: zeroize::Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: zeroize::Zeroizing<super::super::wallets::UnpersistedPrfKey>,
    context: CloudBackupEnableContext,
    namespace_id: String,
    encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup,
    pending_uploads: Vec<PendingVerificationUpload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepVerificationContinuation {
    Manual { force_discoverable: bool, attempt: VerificationAttempt },
    RecreateManifest { attempt: VerificationAttempt },
    ReinitializeBackup { attempt: VerificationAttempt },
}

impl DeepVerificationContinuation {
    fn force_discoverable(self) -> bool {
        match self {
            Self::Manual { force_discoverable, .. } => force_discoverable,
            Self::RecreateManifest { .. } | Self::ReinitializeBackup { .. } => false,
        }
    }
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
        let result: CloudBackupWriteCommandResult<T> = receiver
            .await
            .map_err_prefix("wait for cloud backup write supervisor", CloudBackupError::Internal)?;
        let context = result.context();
        if context.origin() != Some(origin) {
            return Err(CloudBackupError::Internal(format!(
                "cloud backup write supervisor returned mismatched operation origin for command {:?}",
                context.id()
            )));
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
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        Self::await_cloud_backup_write_for_operation(receiver, origin).await.map_err(|error| {
            let error = match error {
                CloudBackupError::CloudStorage(source) => {
                    CloudBackupError::cloud_storage_context("delete wallet backup", source)
                }
                error => error,
            };

            blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error)
        })?;

        info!("Deleted cloud wallet {}", prepared.record_id);
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
        .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

        Self::await_cloud_backup_write_for_operation(receiver, origin).await
    }

    async fn apply_cloud_backup_write_completion_for_operation(
        write: Addr<CloudBackupWriteSupervisor>,
        completion: CloudBackupWriteCompletion,
        origin: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), CloudBackupError> {
        let receiver = call!(write.apply_completion_for_operation(completion, origin))
            .await
            .map_err_prefix("start cloud backup write supervisor", CloudBackupError::Internal)?;

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

    pub async fn ensure_exclusive_operation_current(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if self.active_operation == Some(claim) {
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
            .map_err(|error| CloudBackupError::Internal(format!("{context}: {error}")));
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
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };

        match operation {
            CloudBackupOperation::Enable(context) => {
                let Some(claim) =
                    self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
                else {
                    return;
                };

                match self.start_ready_enable_upload_if_present(
                    manager.clone(),
                    claim,
                    PendingEnableUploadSelection::RetryOnly,
                ) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        self.fail_enable_operation(&manager, claim, error);
                        return;
                    }
                }

                if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
                    return;
                }

                addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_enable(context).await;
                    send!(addr.complete_enable_preparation(claim, result));
                });
            }
            CloudBackupOperation::EnableForceNew(context) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::EnableForceNew,
                ) else {
                    return;
                };

                match self.start_ready_enable_upload_if_present(
                    manager.clone(),
                    claim,
                    PendingEnableUploadSelection::RetryOrForceNewConfirmation,
                ) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        self.fail_enable_operation(&manager, claim, error);
                        return;
                    }
                }

                manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
                self.schedule_enable_passkey_registration(
                    manager,
                    claim,
                    context,
                    EnablePasskeyRegistrationFlow::ForceNew,
                );
            }
            CloudBackupOperation::EnableNoDiscovery(context) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::EnableNoDiscovery,
                ) else {
                    return;
                };

                match self.start_ready_enable_upload_if_present(
                    manager.clone(),
                    claim,
                    PendingEnableUploadSelection::RetryOnly,
                ) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        self.fail_enable_operation(&manager, claim, error);
                        return;
                    }
                }

                if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
                    return;
                }
                if self
                    .finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim)
                {
                    return;
                }

                addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_no_discovery_enable(context).await;
                    send!(addr.complete_no_discovery_enable_preparation(claim, result));
                });
            }
            CloudBackupOperation::Recovery(action) => match action {
                RecoveryAction::RecreateManifest => {
                    let Some(claim) = self.begin_exclusive_operation(
                        &manager,
                        CloudBackupExclusiveOperation::RecreateManifest,
                    ) else {
                        return;
                    };

                    let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                    addr.send_fut_with(move |addr| async move {
                        let result = manager.prepare_reupload_all_wallets(writes).await;
                        send!(addr.complete_recreate_manifest_recovery(claim, result));
                    });
                }
                RecoveryAction::ReinitializeBackup => {
                    let Some(claim) = self.begin_exclusive_operation(
                        &manager,
                        CloudBackupExclusiveOperation::ReinitializeBackup,
                    ) else {
                        return;
                    };

                    manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Started(
                        RecoveryAction::ReinitializeBackup,
                    ));

                    match self.start_ready_enable_upload_if_present(
                        manager.clone(),
                        claim,
                        PendingEnableUploadSelection::RetryOnly,
                    ) {
                        Ok(true) => return,
                        Ok(false) => {}
                        Err(error) => {
                            self.fail_enable_operation(&manager, claim, error);
                            return;
                        }
                    }

                    if self
                        .finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim)
                    {
                        return;
                    }

                    addr.send_fut_with(move |addr| async move {
                        let result = manager
                            .prepare_enable(CloudBackupEnableContext::settings_manual())
                            .await;
                        send!(addr.complete_enable_preparation(claim, result));
                    });
                }
                RecoveryAction::RepairPasskey => {
                    self.spawn_operation(
                        CloudBackupOperation::RepairPasskey { no_discovery: false },
                        None,
                    );
                }
            },
            CloudBackupOperation::RepairPasskey { no_discovery } => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::RepairPasskey,
                ) else {
                    return;
                };

                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Started(
                    RecoveryAction::RepairPasskey,
                ));
                addr.send_fut_with(move |addr| async move {
                    let result = if no_discovery {
                        manager.prepare_passkey_wrapper_repair_no_discovery().await
                    } else {
                        manager.prepare_passkey_wrapper_repair().await
                    };
                    send!(addr.complete_repair_passkey_wrapper(claim, result));
                });
            }
            CloudBackupOperation::Sync => {
                self.start_sync_request(manager);
            }
            CloudBackupOperation::FetchCloudOnly => {
                self.start_cloud_only_fetch_request(manager);
            }
            CloudBackupOperation::Disable => {
                let Some(claim) = self
                    .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
                else {
                    return;
                };

                addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_disable_cloud_backup().await;
                    send!(addr.complete_disable_preparation(claim, result));
                });
            }
            CloudBackupOperation::RestoreCloudWallet => {
                let Some(record_id) = record_id else { return };
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::RestoreCloudWallet,
                ) else {
                    return;
                };

                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Started { record_id: record_id.clone() },
                );
                addr.send_fut_with(move |addr| async move {
                    let result = manager.do_restore_cloud_wallet(&record_id).await;
                    send!(addr.complete_restore_cloud_wallet(claim, record_id, result));
                });
            }
            CloudBackupOperation::DeleteCloudWallet => {
                let Some(record_id) = record_id else { return };
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::DeleteCloudWallet,
                ) else {
                    return;
                };

                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Started { record_id: record_id.clone() },
                );
                addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_delete_cloud_wallet(&record_id).await;
                    send!(addr.complete_delete_cloud_wallet_preparation(claim, result));
                });
            }
            CloudBackupOperation::RecoverOtherBackups => {
                let Some(claim) = Self::begin_other_backups_operation(
                    self,
                    &manager,
                    CloudBackupExclusiveOperation::RecoverOtherBackups,
                    CloudBackupOtherBackupsOutcome::Recovering,
                ) else {
                    return;
                };

                addr.send_fut_with(move |addr| async move {
                    let result = manager.do_recover_other_backups().await;
                    send!(addr.complete_recover_other_backups(claim, result));
                });
            }
            CloudBackupOperation::DeleteOtherBackups => {
                let Some(claim) = Self::begin_other_backups_operation(
                    self,
                    &manager,
                    CloudBackupExclusiveOperation::DeleteOtherBackups,
                    CloudBackupOtherBackupsOutcome::Deleting,
                ) else {
                    return;
                };

                addr.send_fut_with(move |addr| async move {
                    let result = manager.do_delete_other_backups().await;
                    send!(addr.complete_delete_other_backups(claim, result));
                });
            }
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

    fn start_cloud_only_fetch_request(&mut self, manager: Arc<RustCloudBackupManager>) {
        let request_id = self.next_request_id();
        self.active_cloud_only_fetch_request = Some(request_id);
        manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Started);

        self.addr.send_fut_with(move |addr| async move {
            let result = manager.do_fetch_cloud_only_wallets().await;
            send!(addr.complete_cloud_only_fetch_request(request_id, result));
        });
    }

    pub async fn complete_cloud_only_fetch_request(
        &mut self,
        request_id: u64,
        result: Result<Vec<CloudBackupWalletItem>, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_cloud_only_fetch_request != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_cloud_only_fetch_request = None;
            return Produces::ok(());
        };

        match result {
            Ok(items) => {
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(
                    items,
                ));
            }
            Err(error) => {
                error!("Failed to fetch cloud-only wallets: {error}");
                manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Failed(
                    error.to_string(),
                ));
            }
        }

        self.active_cloud_only_fetch_request = None;
        Produces::ok(())
    }

    fn begin_other_backups_operation(
        supervisor: &mut CloudBackupSupervisor,
        manager: &RustCloudBackupManager,
        operation: CloudBackupExclusiveOperation,
        outcome: CloudBackupOtherBackupsOutcome,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if !matches!(
            manager.state.read().other_backups_operation(),
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return None;
        }

        let claim = supervisor.begin_exclusive_operation(manager, operation)?;
        manager.apply_other_backups_outcome(outcome);
        Some(claim)
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
            apply_refresh_detail_result(&manager, &result);
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

    pub async fn start_verification(&mut self, force_discoverable: bool) -> ActorResult<()> {
        self.start_verification_with_context(force_discoverable, VerificationAttempt::Initial).await
    }

    async fn start_verification_with_context(
        &mut self,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_verification(manager, force_discoverable, attempt);

        Produces::ok(())
    }

    fn schedule_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(force_discoverable).await;
            send!(addr.complete_verification(result, force_discoverable, attempt));
        });
    }

    pub async fn complete_verification(
        &mut self,
        result: CloudBackupDeepVerificationStep,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::VerificationRepair,
                ) else {
                    let result = DeepVerificationResult::Failed(DeepVerificationFailure::retry(
                        "cloud backup verification repair is waiting for another operation",
                        None,
                        None,
                    ));
                    manager.handle_deep_verification_result(result);
                    return Produces::ok(());
                };

                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::Manual { force_discoverable, attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::VerificationRepair,
                ) else {
                    let result = DeepVerificationResult::Failed(DeepVerificationFailure::retry(
                        "cloud backup verification auto-sync is waiting for another operation",
                        None,
                        None,
                    ));
                    manager.handle_deep_verification_result(result);
                    return Produces::ok(());
                };

                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::Manual { force_discoverable, attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_verification(
                manager,
                force_discoverable,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        Produces::ok(())
    }

    fn start_deep_verification_wrapper_repair(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        prepared: CloudBackupPreparedDeepVerificationWrapperRepair,
        continuation: DeepVerificationContinuation,
    ) {
        if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey(prepared.credential_id(), prepared.prf_salt())
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)
        {
            self.finish_deep_verification_continuation_with_error(
                manager,
                claim,
                continuation,
                error,
            );
            return;
        }

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: prepared.namespace_id().to_owned(),
            credential_id: prepared.credential_id().to_vec(),
            prf_salt: prepared.prf_salt(),
        });

        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result =
                manager.upload_passkey_wrapper_repair(upload, writes).await.map(|_| resume);
            send!(addr.complete_deep_verification_wrapper_repair_upload(
                claim,
                continuation,
                result
            ));
        });
    }

    pub async fn complete_deep_verification_wrapper_repair_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<CloudBackupPendingDeepVerificationResume, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(resume) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager
                        .resume_deep_verify_after_wrapper_repair(
                            resume,
                            continuation.force_discoverable(),
                        )
                        .await;
                    send!(addr.complete_deep_verification_wrapper_repair_resume(
                        claim,
                        continuation,
                        result
                    ));
                });
            }
            Err(error) => {
                self.finish_deep_verification_continuation_with_error(
                    manager,
                    claim,
                    continuation,
                    error,
                );
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_wrapper_repair_resume(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: CloudBackupDeepVerificationStep,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            CloudBackupDeepVerificationStep::Complete(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(manager, claim, *prepared, continuation);
            }
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(_) => {
                self.finish_deep_verification_continuation_with_error(
                    manager,
                    claim,
                    continuation,
                    CloudBackupError::Internal(
                        "deep verification requested wrapper repair twice".into(),
                    ),
                );
            }
        }
        Produces::ok(())
    }

    fn start_deep_verification_auto_sync(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        prepared: CloudBackupPreparedDeepVerificationAutoSync,
        continuation: DeepVerificationContinuation,
    ) {
        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = match manager.upload_deep_verification_auto_sync(upload, writes).await {
                Ok(uploaded) => Ok((resume, uploaded)),
                Err(error) => Err(resume.upload_error_result(&error)),
            };
            send!(addr.complete_deep_verification_auto_sync_upload(claim, continuation, result));
        });
    }

    pub async fn complete_deep_verification_auto_sync_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                let namespace_id = uploaded.namespace_id().to_owned();
                let uploaded_wallets = uploaded.uploaded_wallets().to_vec();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = match writes
                        .finalize_uploaded_wallets(
                            CloudStorage::global_explicit_client(),
                            namespace_id,
                            uploaded_wallets,
                            CloudBackupUploadedWalletsStateMode::PreserveVerification,
                        )
                        .await
                    {
                        Ok(()) => Ok((resume, uploaded)),
                        Err(error) => Err(resume.upload_error_result(&error)),
                    };
                    send!(addr.complete_deep_verification_auto_sync_finalization(
                        claim,
                        continuation,
                        result
                    ));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                self.addr.send_fut_with(move |addr| async move {
                    let completion =
                        manager.resume_deep_verify_after_auto_sync(resume, uploaded).await;
                    send!(addr.complete_deep_verification_auto_sync_resume(
                        claim,
                        continuation,
                        completion
                    ));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_resume(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        completion: CloudBackupDeepVerificationAutoSyncCompletion,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let (result, pending_completion) = completion.into_parts();
        if let Some(pending_completion) = pending_completion {
            manager.replace_pending_verification_completion(pending_completion);
        }
        self.finish_deep_verification_continuation(manager, claim, continuation, result);
        Produces::ok(())
    }

    fn finish_deep_verification_continuation_with_error(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        error: CloudBackupError,
    ) {
        let result = RustCloudBackupManager::deep_verification_error_result(
            continuation.force_discoverable(),
            error,
        );
        self.finish_deep_verification_continuation(manager, claim, continuation, result);
    }

    fn finish_deep_verification_continuation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: DeepVerificationResult,
    ) {
        match continuation {
            DeepVerificationContinuation::Manual { force_discoverable, attempt } => {
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);

                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_verification(
                        manager,
                        force_discoverable,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
            }
            DeepVerificationContinuation::RecreateManifest { attempt } => {
                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_recreate_manifest_verification(
                        manager,
                        claim,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            DeepVerificationContinuation::ReinitializeBackup { attempt } => {
                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_reinitialize_verification(
                        manager,
                        claim,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }
    }

    pub async fn complete_recreate_manifest_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupReuploadedWallets, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(reuploaded) => {
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = writes
                        .finalize_uploaded_wallets(
                            CloudStorage::global_explicit_client(),
                            reuploaded.namespace_id,
                            reuploaded.uploaded_wallets,
                            CloudBackupUploadedWalletsStateMode::PreserveVerification,
                        )
                        .await;
                    send!(addr.complete_recreate_manifest_finalization(claim, result));
                });
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_recreate_manifest_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                self.start_recreate_manifest_verification(
                    manager,
                    claim,
                    VerificationAttempt::Initial,
                );
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    fn start_reinitialize_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_reinitialize_verification(manager, claim, attempt);
    }

    fn schedule_reinitialize_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(false).await;
            send!(addr.complete_reinitialize_verification(claim, result, attempt));
        });
    }

    pub async fn complete_reinitialize_verification(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupDeepVerificationStep,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::ReinitializeBackup { attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::ReinitializeBackup { attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_reinitialize_verification(
                manager,
                claim,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn complete_disable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupDisablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let disabling = match result {
            Ok(CloudBackupDisablePreparation::AlreadyDisabled) => {
                self.finish_disable_operation(&manager, claim);
                return Produces::ok(());
            }
            Ok(CloudBackupDisablePreparation::Ready(disabling)) => *disabling,
            Err(error) => {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
                return Produces::ok(());
            }
        };

        let blocker =
            CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };
        if let Err(error) = call!(self.write.block(blocker)).await {
            self.fail_disable_operation(
                &manager,
                claim,
                format!("install cloud backup disable fence: {error}"),
                manager.disable_can_keep_enabled(),
            );
            return Produces::ok(());
        }

        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        manager.apply_disable_outcome(CloudBackupDisableOutcome::Started);
        if let Err(error) = self.quiesce_disable_runtime(&manager).await {
            self.fail_disable_after_delete_started(&manager, claim, disabling, error.to_string());
            return Produces::ok(());
        }

        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        if disabling.delete_started_at.is_some() {
            self.schedule_disable_namespace_delete(claim, disabling);
        } else {
            self.schedule_disable_blocker_check(manager, claim, disabling);
        }

        Produces::ok(())
    }

    async fn quiesce_disable_runtime(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        self.pending_enable_session = None;
        self.pending_verification_completion = None;
        self.runtime_passkey_authorization = None;
        call!(self.sync_health.clear_upload_runtime_state())
            .await
            .map_err(|error| CloudBackupError::Internal(error.to_string()))?;
        call!(self.uploads.clear_upload_runtime_state())
            .await
            .map_err(|error| CloudBackupError::Internal(error.to_string()))?;

        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        Ok(())
    }

    fn schedule_disable_blocker_check(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let cloud = CloudStorage::global_explicit_client();
            let result = manager.check_disable_blockers(&cloud, &disabling).await;
            send!(addr.complete_disable_blocker_check(claim, disabling, result));
        });
    }

    pub async fn complete_disable_blocker_check(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        if let Err(error) = result {
            let message = error.to_string();
            if let Err(error) = manager.rollback_disable_before_delete(&disabling, message.clone())
            {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
            } else {
                self.finish_disable_operation(&manager, claim);
            }
            return Produces::ok(());
        }

        match manager.mark_disable_delete_started(&disabling) {
            Ok(disabling) => self.schedule_disable_namespace_delete(claim, disabling),
            Err(error) => {
                self.fail_disable_operation(
                    &manager,
                    claim,
                    error.to_string(),
                    manager.disable_can_keep_enabled(),
                );
            }
        }

        Produces::ok(())
    }

    fn schedule_disable_namespace_delete(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
    ) {
        let write = self.write.clone();
        self.addr.send_fut_with(move |addr| async move {
            let result = Self::delete_cloud_backup_namespace_for_operation(
                write,
                disabling.namespace_id.clone(),
                claim,
            )
            .await;
            send!(addr.complete_disable_namespace_delete(claim, disabling, result));
        });
    }

    pub async fn complete_disable_namespace_delete(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) | Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                self.schedule_disable_local_cleanup(manager, claim, disabling);
            }
            Err(CloudBackupError::CloudStorage(error)) => {
                let message =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error)
                        .to_string();
                self.fail_disable_after_delete_started(&manager, claim, disabling, message);
            }
            Err(error) => {
                self.fail_disable_after_delete_started(
                    &manager,
                    claim,
                    disabling,
                    error.to_string(),
                );
            }
        }

        Produces::ok(())
    }

    fn schedule_disable_local_cleanup(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.finish_disable_local_cleanup();
            send!(addr.complete_disable_local_cleanup(claim, disabling, result));
        });
    }

    pub async fn complete_disable_local_cleanup(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        if let Err(error) = result {
            self.fail_disable_after_delete_started(&manager, claim, disabling, error.to_string());
            return Produces::ok(());
        }

        if let Err(error) = manager.persist_disabled_after_remote_delete() {
            self.fail_disable_after_delete_started(&manager, claim, disabling, error.to_string());
            return Produces::ok(());
        }

        let blocker =
            CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };
        if let Err(error) = call!(self.write.unblock(blocker)).await {
            warn!("Failed to lift cloud backup disable fence: {error}");
        }

        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }

        info!("Disabled cloud backup and deleted active namespace");
        self.finish_disable_operation(&manager, claim);
        Produces::ok(())
    }

    fn fail_disable_after_delete_started(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        disabling: crate::database::cloud_backup::PersistedDisablingCloudBackup,
        message: String,
    ) {
        let message = match manager.persist_disabling_failure(disabling, message.clone()) {
            Ok(()) => message,
            Err(error) => error.to_string(),
        };
        self.fail_disable_operation(manager, claim, message, manager.disable_can_keep_enabled());
    }

    fn fail_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
        can_keep_enabled: bool,
    ) {
        error!("disable_cloud_backup failed: {message}");
        manager
            .apply_disable_outcome(CloudBackupDisableOutcome::Failed { message, can_keep_enabled });
        self.finish_disable_operation(manager, claim);
    }

    fn finish_disable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn start_recreate_manifest_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_recreate_manifest_verification(manager, claim, attempt);
    }

    fn schedule_recreate_manifest_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(false).await;
            send!(addr.complete_recreate_manifest_verification(claim, result, attempt));
        });
    }

    pub async fn complete_recreate_manifest_verification(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupDeepVerificationStep,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::RecreateManifest { attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::RecreateManifest { attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_recreate_manifest_verification(
                manager,
                claim,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
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

    pub async fn confirm_saved_passkey(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let pending = match self.pending_enable_session.take() {
            Some(session @ PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)) => session,
            other => {
                self.pending_enable_session = other;
                return Produces::ok(());
            }
        };
        let Some(addr) = self.addr() else {
            self.pending_enable_session = Some(pending);
            return Produces::ok(());
        };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        else {
            self.pending_enable_session = Some(pending);
            return Produces::ok(());
        };

        manager.apply_enable_outcome(CloudBackupEnableOutcome::ConfirmingSavedPasskey);
        addr.send_fut_with(move |addr| async move {
            let result = manager.confirm_saved_passkey_from_session(pending).await;
            send!(addr.complete_saved_passkey_confirmation(claim, result));
        });

        Produces::ok(())
    }

    pub async fn complete_saved_passkey_confirmation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupSavedPasskeyConfirmation,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            CloudBackupSavedPasskeyConfirmation::Confirmed(confirmed) => {
                self.pending_enable_session = Some(PendingEnableSession::retry_upload(
                    cove_cspp::master_key::MasterKey::from_bytes(*confirmed.master_key.as_bytes()),
                    confirmed.passkey.copy_for_retry(),
                    confirmed.context,
                ));
                manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
                self.schedule_enable_upload(manager, claim, confirmed);
            }
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error } => {
                warn!("Confirm saved passkey will retry: {error}");
                self.pending_enable_session = Some(pending);
                manager.apply_enable_outcome(
                    CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
                        SavedPasskeyConfirmationMode::Manual,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            CloudBackupSavedPasskeyConfirmation::Failed(error) => {
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_enable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePreparation::CreateNew { context }) => {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
                self.schedule_create_new_enable_passkey(manager, claim, context);
            }
            Ok(CloudBackupEnablePreparation::ExistingBackupFound { context, passkey_hint }) => {
                manager.present_existing_backup_found_prompt(context, passkey_hint);
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Ok(CloudBackupEnablePreparation::PasskeyChoice { context, passkey_hint }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context,
                    passkey_hint,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Ok(CloudBackupEnablePreparation::Recover { matches }) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_enable_recovery(matches).await;
                    send!(addr.complete_enable_recovery_preparation(claim, result));
                });
            }
            Err(error) => {
                error!("enable preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn schedule_create_new_enable_passkey(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule create-new enable passkey without supervisor addr");
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_create_new_enable_passkey(context).await;
            send!(addr.complete_create_new_enable_passkey(claim, result));
        });
    }

    pub async fn complete_create_new_enable_passkey(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePasskeyPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePasskeyPreparation::Ready(ready)) => {
                self.pending_enable_session = Some(PendingEnableSession::retry_upload(
                    cove_cspp::master_key::MasterKey::from_bytes(*ready.master_key.as_bytes()),
                    ready.passkey.copy_for_retry(),
                    ready.context,
                ));
                manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
                self.schedule_enable_upload(manager, claim, ready);
            }
            Ok(CloudBackupEnablePasskeyPreparation::Registered(registered)) => {
                self.accept_registered_enable_passkey(&manager, claim, registered);
            }
            Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Err(error) => {
                error!("create-new enable passkey failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_enable_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnableRecoveryCompletion, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(completion) => {
                if let Err(error) = self.start_enable_recovery_finalization(claim, completion) {
                    manager.rollback_enable_recovery_master_key();
                    self.fail_enable_operation(&manager, claim, error);
                }
            }
            Err(error) => {
                manager.rollback_enable_recovery_master_key();
                error!("enable recovery failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_enable_recovery_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnableRecoveryPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(preparation) => {
                if let Err(error) = manager.save_enable_recovery_master_key(&preparation) {
                    self.fail_enable_operation(&manager, claim, error);
                    return Produces::ok(());
                }

                let Some(addr) = self.addr() else {
                    manager.rollback_enable_recovery_master_key();
                    self.fail_enable_operation(
                        &manager,
                        claim,
                        CloudBackupError::Internal(
                            "could not schedule enable recovery completion".into(),
                        ),
                    );
                    return Produces::ok(());
                };

                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                addr.send_fut_with(move |addr| async move {
                    let result =
                        manager.prepare_enable_recovery_completion(preparation, writes).await;
                    send!(addr.complete_enable_recovery(claim, result));
                });
            }
            Err(error) => {
                error!("enable recovery preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn start_enable_recovery_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: CloudBackupEnableRecoveryCompletion,
    ) -> Result<(), CloudBackupError> {
        let CloudBackupEnableRecoveryCompletion {
            namespace_id,
            credential_id,
            prf_salt,
            active_critical_key,
            uploaded_wallets,
            cleanup_sources,
        } = completion;

        CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey_and_namespace(&credential_id, prf_salt, &namespace_id)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: namespace_id.clone(),
            credential_id,
            prf_salt,
        });

        let finalization = EnableRecoveryFinalization {
            namespace_id: namespace_id.clone(),
            active_critical_key,
            cleanup_sources,
        };
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = writes
                .finalize_uploaded_wallets(
                    CloudStorage::global_explicit_client(),
                    namespace_id,
                    uploaded_wallets,
                    CloudBackupUploadedWalletsStateMode::ResetVerification,
                )
                .await;
            send!(addr.complete_enable_recovery_finalization(claim, finalization, result));
        });

        Ok(())
    }

    pub async fn complete_enable_recovery_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        finalization: EnableRecoveryFinalization,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                let EnableRecoveryFinalization {
                    namespace_id,
                    active_critical_key,
                    cleanup_sources,
                } = finalization;
                call!(self.cleanup.enqueue_cleanup(CloudBackupCleanupJob {
                    cloud: CloudStorage::global_explicit_client(),
                    active_namespace_id: namespace_id,
                    active_critical_key: *active_critical_key,
                    sources: cleanup_sources,
                }))
                .await
                .map_err_str(CloudBackupError::Internal)?;

                self.pending_enable_session = None;
                manager.clear_enable_progress(CloudBackupStatus::Enabled);
                info!("Cloud backup enabled (recovered existing namespace)");
                self.finish_enable_operation(manager, claim);
            }
            Err(error) => {
                manager.rollback_enable_recovery_master_key();
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_no_discovery_enable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupNoDiscoveryEnablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupNoDiscoveryEnablePreparation::RegisterPasskey { context }) => {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
                self.schedule_enable_passkey_registration(
                    manager,
                    claim,
                    context,
                    EnablePasskeyRegistrationFlow::NoDiscovery,
                );
            }
            Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                context,
                passkey_hint,
            }) => {
                manager.present_existing_backup_found_prompt(context, passkey_hint);
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                error!("enable no-discovery preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn schedule_enable_passkey_registration(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
        flow: EnablePasskeyRegistrationFlow,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable passkey registration without supervisor addr");
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_new_enable_passkey_for_confirmation(context, flow).await;
            send!(addr.complete_enable_passkey_registration(claim, result));
        });
    }

    pub async fn complete_enable_passkey_registration(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePasskeyRegistration, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePasskeyRegistration::Registered(registered)) => {
                self.accept_registered_enable_passkey(&manager, claim, registered);
            }
            Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                error!("enable passkey registration failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn finish_awaiting_force_new_confirmation_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> bool {
        let Some(context) = self
            .pending_enable_session
            .as_ref()
            .filter(|session| session.is_awaiting_force_new_confirmation())
            .map(PendingEnableSession::context)
        else {
            return false;
        };

        manager.present_existing_backup_found_prompt(context, None);
        manager.clear_enable_progress(CloudBackupStatus::Disabled);
        self.finish_enable_operation(manager, claim);
        true
    }

    fn finish_awaiting_saved_passkey_confirmation_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> bool {
        if !self
            .pending_enable_session
            .as_ref()
            .is_some_and(PendingEnableSession::is_awaiting_saved_passkey_confirmation)
        {
            return false;
        }

        manager.apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));
        self.finish_enable_operation(manager, claim);
        true
    }

    fn accept_registered_enable_passkey(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        registered: CloudBackupRegisteredEnablePasskey,
    ) {
        let saved_passkey_confirmation = registered.context.saved_passkey_confirmation;
        self.pending_enable_session =
            Some(PendingEnableSession::awaiting_saved_passkey_confirmation(
                registered.master_key,
                registered.passkey,
                registered.context,
            ));
        manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
        self.schedule_enable_saved_passkey_wait(claim, saved_passkey_confirmation);
    }

    fn schedule_enable_saved_passkey_wait(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        mode: SavedPasskeyConfirmationMode,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable saved-passkey wait without supervisor addr");
            return;
        };

        cove_tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            send!(addr.complete_enable_saved_passkey_wait(claim, mode));
        });
    }

    pub async fn complete_enable_saved_passkey_wait(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        mode: SavedPasskeyConfirmationMode,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        manager
            .apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(mode));
        self.finish_enable_operation(manager, claim);
        Produces::ok(())
    }

    fn start_ready_enable_upload_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        selection: PendingEnableUploadSelection,
    ) -> Result<bool, CloudBackupError> {
        let Some(ready) = self.take_ready_enable_upload(selection)? else {
            return Ok(false);
        };

        manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
        self.schedule_enable_upload(manager, claim, ready);
        Ok(true)
    }

    fn take_ready_enable_upload(
        &mut self,
        selection: PendingEnableUploadSelection,
    ) -> Result<Option<CloudBackupReadyEnableUpload>, CloudBackupError> {
        let Some(pending) = self.pending_enable_session.take() else {
            return Ok(None);
        };
        let should_use = match selection {
            PendingEnableUploadSelection::RetryOnly => pending.is_retry_upload(),
            PendingEnableUploadSelection::RetryOrForceNewConfirmation => {
                pending.is_retry_upload() || pending.is_awaiting_force_new_confirmation()
            }
        };

        if !should_use {
            self.pending_enable_session = Some(pending);
            return Ok(None);
        }

        let context = pending.context();
        let (master_key, passkey) = pending.into_ready_parts()?;
        Ok(Some(CloudBackupReadyEnableUpload { master_key, passkey, context }))
    }

    fn schedule_enable_upload(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        ready: CloudBackupReadyEnableUpload,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable upload without supervisor addr");
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        cove_tokio::task::spawn(async move {
            let result = manager.upload_ready_enable_backup(ready, writes).await;
            send!(addr.complete_enable_upload(claim, result));
        });
    }

    pub async fn complete_enable_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupUploadedEnableBackup, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(upload) => {
                if let Err(error) = self.start_enable_upload_finalization(claim, upload) {
                    self.fail_enable_operation(&manager, claim, error);
                }
            }
            Err(error) => self.fail_enable_operation(&manager, claim, error),
        }

        Produces::ok(())
    }

    fn start_enable_upload_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        upload: CloudBackupUploadedEnableBackup,
    ) -> Result<(), CloudBackupError> {
        info!("Enable: persisting cloud backup state");
        CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey_and_namespace(
                &upload.passkey.credential_id,
                upload.passkey.prf_salt,
                &upload.namespace_id,
            )
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        let completion = CloudBackupWriteCompletion::mark_uploaded_pending_confirmation(
            upload.namespace_id.clone(),
            CloudBackupRecordKey::MasterKeyWrapper,
            upload.master_key_wrapper_revision.clone(),
            upload.uploaded_at,
        );

        let uploaded_wallets = upload
            .uploaded_wallets
            .into_iter()
            .map(|wallet| {
                CloudBackupUploadedWallet::new(
                    wallet.metadata.id,
                    wallet.record_id,
                    wallet.revision_hash,
                )
            })
            .collect();
        let finalization = EnableUploadFinalization {
            master_key: upload.master_key,
            passkey: upload.passkey,
            context: upload.context,
            namespace_id: upload.namespace_id.clone(),
            encrypted_master: upload.encrypted_master,
            pending_uploads: upload.pending_uploads,
        };
        let write = self.write.clone();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = async {
                Self::apply_cloud_backup_write_completion_for_operation(write, completion, claim)
                    .await?;

                writes
                    .finalize_uploaded_wallets(
                        CloudStorage::global_explicit_client(),
                        upload.namespace_id,
                        uploaded_wallets,
                        CloudBackupUploadedWalletsStateMode::ResetVerification,
                    )
                    .await
            }
            .await;
            send!(addr.complete_enable_upload_finalization(claim, finalization, result));
        });

        Ok(())
    }

    pub async fn complete_enable_upload_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        finalization: EnableUploadFinalization,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let EnableUploadFinalization {
            master_key,
            passkey,
            context,
            namespace_id,
            encrypted_master,
            mut pending_uploads,
        } = finalization;

        if let Err(error) = result {
            self.fail_enable_operation(&manager, claim, error);
            return Produces::ok(());
        }

        let decrypted_master =
            master_key_crypto::decrypt_master_key(&encrypted_master, &passkey.prf_key)
                .map_err_str(CloudBackupError::Crypto);
        let decrypted_master = match decrypted_master {
            Ok(decrypted_master) => decrypted_master,
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return Produces::ok(());
            }
        };
        if decrypted_master.as_bytes() != master_key.as_bytes() {
            self.fail_enable_operation(
                &manager,
                claim,
                CloudBackupError::Crypto(
                    "fresh passkey material decrypted the wrong master key".into(),
                ),
            );
            return Produces::ok(());
        }

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: namespace_id.clone(),
            credential_id: passkey.credential_id.clone(),
            prf_salt: passkey.prf_salt,
        });

        pending_uploads.insert(0, PendingVerificationUpload::master_key_wrapper());
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        manager.replace_pending_verification_completion_for_source(
            PendingVerificationCompletion::new(report, namespace_id, pending_uploads),
            context.verification_source,
        );
        manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.pending_enable_session = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        self.finish_enable_operation(manager, claim);

        Produces::ok(())
    }

    fn fail_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            self.fail_reinitialize_enable_operation(manager, claim, error);
            return;
        }

        warn!("Enable failed: {error}");
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        manager
            .reconcile_runtime_status(RustCloudBackupManager::status_for_operation_error(&error));
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn fail_reinitialize_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        warn!("Reinitialize backup enable failed: {error}");
        match error {
            CloudBackupError::UnsupportedPasskeyProvider => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
            }
            error => {
                let runtime_status = RustCloudBackupManager::runtime_status_for(
                    &RustCloudBackupManager::load_persisted_state(),
                );
                manager.reconcile_runtime_status(runtime_status);
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: error.to_string(),
                });
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn finish_enable_operation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
            let runtime_status = RustCloudBackupManager::runtime_status_for(
                &RustCloudBackupManager::load_persisted_state(),
            );
            if matches!(runtime_status, CloudBackupStatus::Enabled) {
                self.start_reinitialize_verification(manager, claim, VerificationAttempt::Initial);
                return;
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn clear_runtime_passkey_authorization(&mut self) -> ActorResult<()> {
        self.runtime_passkey_authorization = None;
        Produces::ok(())
    }

    pub async fn complete_delete_cloud_wallet_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedCloudWalletDelete, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(prepared) => {
                let record_id = prepared.record_id.clone();
                let write = self.write.clone();
                self.addr.send_fut_with(move |addr| async move {
                    let result =
                        Self::delete_prepared_cloud_wallet_for_operation(write, prepared, claim)
                            .await;
                    send!(addr.complete_delete_cloud_wallet(claim, record_id, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.to_string(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_delete_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Deleted { record_id },
                );
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.to_string(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_restore_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
        result: Result<WalletRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(outcome) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Restored {
                        record_id,
                        warning: cloud_only_restore_warning(outcome),
                    },
                );
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Failed(
                    error.to_string(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_recover_other_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupRestoreReport, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(report) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Recovered {
                    wallets_restored: report.wallets_restored,
                    wallets_failed: report.wallets_failed,
                    failed_wallet_errors: report.failed_wallet_errors,
                });
                manager.apply_sync_outcome(CloudBackupSyncOutcome::Started);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.do_sync_unsynced_wallets().await;
                    send!(addr.complete_operation_sync(claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.to_string(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_delete_other_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Deleted);
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_other_backups_outcome(CloudBackupOtherBackupsOutcome::Failed(
                    error.to_string(),
                ));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_sync(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.refresh_sync_health();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_operation_sync_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(error.to_string()));
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_operation_sync_refresh_detail(
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
            apply_refresh_detail_result(&manager, &result);
        }

        manager.apply_sync_outcome(CloudBackupSyncOutcome::Completed);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedPasskeyWrapperRepair, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(preparation) => {
                if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
                    .save_passkey(&preparation.credential_id, preparation.prf_salt)
                    .map_err_prefix("save cspp credentials", CloudBackupError::Internal)
                {
                    manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.to_string(),
                    });
                    self.active_operation = None;
                    manager.project_exclusive_operation_finished(claim);
                    return Produces::ok(());
                }

                self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
                    namespace_id: preparation.namespace_id.clone(),
                    credential_id: preparation.credential_id.clone(),
                    prf_salt: preparation.prf_salt,
                });

                let upload = preparation.into_upload();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.upload_passkey_wrapper_repair(upload, writes).await;
                    send!(addr.complete_repair_passkey_wrapper_upload(claim, result));
                });
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager
                    .present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::RepairPasskey);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupUploadedPasskeyWrapperRepair, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(uploaded) => {
                manager.finish_passkey_wrapper_repair(uploaded);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_passkey_repair_finalization().await;
                    send!(addr.complete_repair_passkey_finalization(claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPasskeyRepairFinalization, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result
            .and_then(|finalization| manager.apply_passkey_repair_finalization(finalization))
        {
            Ok(()) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_repair_passkey_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_refresh_detail(
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

        match result {
            Some(CloudBackupDetailResult::Success(detail)) => {
                manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
            }
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Failed to refresh detail after passkey repair: {error}");
            }
            None => {}
        }

        manager.refresh_sync_health();
        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
        manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn discard_pending_enable_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(pending) = self.pending_enable_session.take() else {
            if let Some(manager) = self.manager() {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
                manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
            }
            return Produces::ok(());
        };

        let should_delete_remote = pending.is_retry_upload();
        let namespace_id = pending.namespace_id();

        if let Err(error) = CloudBackupKeychain::global().clear_local_state() {
            warn!("Discard pending enable failed to clear local cloud backup state: {error}");
        }

        if let Some(manager) = self.manager() {
            manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
            manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
        }

        if should_delete_remote {
            let writes = self.write.clone();
            cove_tokio::task::spawn(async move {
                let cloud = CloudStorage::global_explicit_client();
                let receiver = call!(writes.delete_wallet_backup(
                    cloud,
                    namespace_id,
                    MASTER_KEY_RECORD_ID.to_string()
                ))
                .await;
                let receiver = match receiver {
                    Ok(receiver) => receiver,
                    Err(error) => {
                        warn!(
                            "Discard pending enable failed to start remote master key delete: {error}"
                        );
                        return;
                    }
                };
                match receiver.await {
                    Ok(result) => {
                        if let Err(error) = result.into_result() {
                            warn!(
                                "Discard pending enable failed to delete remote master key: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        warn!(
                            "Discard pending enable remote master key delete stopped before completion: {error}"
                        );
                    }
                }
            });
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
        call!(self.sync_health.clear_upload_runtime_state()).await?;
        call!(self.uploads.clear_upload_runtime_state()).await?;
        Produces::ok(())
    }

    pub async fn keep_cloud_backup_enabled(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_keep_cloud_backup_enabled().await;
            send!(addr.complete_keep_cloud_backup_enabled(result));
        });
        Produces::ok(())
    }

    pub async fn complete_keep_cloud_backup_enabled(
        &mut self,
        result: Result<CloudBackupKeepEnabledPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        match result {
            Ok(CloudBackupKeepEnabledPreparation::AlreadyConfigured) => {
                manager.clear_stale_disable_failure_if_configured();
            }
            Ok(CloudBackupKeepEnabledPreparation::AlreadyDisabled) => {}
            Ok(CloudBackupKeepEnabledPreparation::Ready(disabling)) => {
                let restored =
                    match manager.restore_configured_cloud_backup_after_disable(&disabling) {
                        Ok(restored) => restored,
                        Err(error) => {
                            manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                                message: error.to_string(),
                                can_keep_enabled: false,
                            });
                            return Produces::ok(());
                        }
                    };

                if !restored {
                    return Produces::ok(());
                }

                if let Err(error) = self
                    .unblock_cloud_backup_writes(CloudBackupWriteBlocker::Disabling {
                        operation_id: disabling.disable_generation,
                    })
                    .await
                {
                    warn!("Failed to lift cloud backup disable fence: {error}");
                }

                manager.finish_keep_cloud_backup_enabled();
            }
            Err(error) => {
                manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.to_string(),
                    can_keep_enabled: false,
                });
            }
        }

        Produces::ok(())
    }
}

fn cloud_only_restore_warning(
    outcome: WalletRestoreOutcome,
) -> Option<CloudBackupCloudOnlyOperationWarning> {
    outcome.labels_warning.map(|warning| CloudBackupCloudOnlyOperationWarning {
        message: format!(
            "{} was restored, but its labels could not be imported",
            warning.wallet_name
        ),
        error: warning.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::PersistedDisablingCloudBackup;
    use crate::manager::cloud_backup_manager::ops::test_support::{
        async_test_lock, reset_cloud_backup_test_state, test_globals,
    };
    use crate::manager::cloud_backup_manager::wallets::{StagedPrfKey, UnpersistedPrfKey};
    use crate::manager::cloud_backup_manager::{CloudBackupStore, PendingEnableSessionMaterial};

    fn test_supervisor_manager() -> Arc<RustCloudBackupManager> {
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);
        manager
    }

    fn test_disabling_state() -> PersistedDisablingCloudBackup {
        CloudBackupStore::global().persist_enabled(0).unwrap();
        let PersistedCloudBackupState::Configured(previous_configured) =
            Database::global().cloud_backup_state.get().unwrap()
        else {
            panic!("expected configured cloud backup state");
        };

        PersistedDisablingCloudBackup {
            previous_configured,
            namespace_id: "namespace".into(),
            disable_generation: 7,
            started_at: 1,
            delete_started_at: Some(2),
            last_error: None,
            retry_after: None,
        }
    }

    fn test_enable_passkey(credential_id: Vec<u8>) -> UnpersistedPrfKey {
        UnpersistedPrfKey {
            prf_key: [7; 32],
            prf_salt: [9; 32],
            credential_id,
            provider_hint: None,
        }
    }

    fn test_staged_passkey(credential_id: Vec<u8>) -> StagedPrfKey {
        StagedPrfKey { prf_salt: [9; 32], credential_id, provider_hint: None }
    }

    fn test_enable_upload_finalization() -> EnableUploadFinalization {
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace_id = master_key.namespace_id();
        let passkey = test_enable_passkey(vec![1, 2, 3]);
        let encrypted_master = master_key_crypto::encrypt_master_key_with_remote_metadata(
            &master_key,
            &passkey.prf_key,
            &passkey.prf_salt,
            passkey.provider_hint.clone(),
            cove_cspp::backup_data::remote_payload::RemotePayloadMetadata::master_key(
                &namespace_id,
                0,
            ),
        )
        .unwrap();

        EnableUploadFinalization {
            master_key: zeroize::Zeroizing::new(master_key),
            passkey: zeroize::Zeroizing::new(passkey),
            context: CloudBackupEnableContext::settings_manual(),
            namespace_id,
            encrypted_master,
            pending_uploads: Vec::new(),
        }
    }

    fn awaiting_force_new_session(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
    ) -> PendingEnableSession {
        PendingEnableSession::AwaitingForceNewConfirmation(PendingEnableSessionMaterial::new(
            master_key,
            passkey,
            CloudBackupEnableContext::settings_manual(),
        ))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_rejects_second_exclusive_operation_while_active() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let first = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let second =
            supervisor.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore);

        assert!(second.is_none());
        assert_eq!(supervisor.active_operation, Some(first));
        assert_eq!(manager.projected_exclusive_operation(), Some(first));

        supervisor.complete_exclusive_operation(first).await.unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_exclusive_operation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor.complete_exclusive_operation(stale).await.unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor.complete_exclusive_operation(current).await.unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_delete_cloud_wallet_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::DeleteCloudWallet,
            u64::MAX,
        );

        supervisor
            .complete_delete_cloud_wallet(
                stale,
                "wallet-record".into(),
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_delete_cloud_wallet(
                current,
                "wallet-record".into(),
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_restore_cloud_wallet_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RestoreCloudWallet,
            u64::MAX,
        );

        supervisor
            .complete_restore_cloud_wallet(stale, "wallet-record".into(), Ok(Default::default()))
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_restore_cloud_wallet(
                current,
                "wallet-record".into(),
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_repair_passkey_wrapper_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RepairPasskey,
            u64::MAX,
        );

        supervisor
            .complete_repair_passkey_wrapper(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_repair_passkey_wrapper(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_repair_passkey_finalization_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RepairPasskey,
            u64::MAX,
        );
        let finalization = CloudBackupPasskeyRepairFinalization { wallet_count: 2 };

        supervisor.complete_repair_passkey_finalization(stale, Ok(finalization)).await.unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_repair_passkey_finalization(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_repair_passkey_wrapper_upload_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RepairPasskey,
            u64::MAX,
        );

        supervisor
            .complete_repair_passkey_wrapper_upload(
                stale,
                Ok(CloudBackupUploadedPasskeyWrapperRepair { namespace_id: "stale".into() }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_repair_passkey_wrapper_upload(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_repair_passkey_refresh_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RepairPasskey,
            u64::MAX,
        );

        supervisor.complete_repair_passkey_refresh_detail(stale, None).await.unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor.complete_repair_passkey_refresh_detail(current, None).await.unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_recreate_manifest_recovery_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RecreateManifest,
            u64::MAX,
        );

        supervisor
            .complete_recreate_manifest_recovery(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_recreate_manifest_recovery(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_recreate_manifest_finalization_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RecreateManifest,
            u64::MAX,
        );

        supervisor
            .complete_recreate_manifest_finalization(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_recreate_manifest_finalization(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_sync_request_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        supervisor.active_sync_request = Some(7);

        supervisor
            .complete_sync_request(6, Err(CloudBackupError::Internal("stale completion".into())))
            .await
            .unwrap();

        assert_eq!(supervisor.active_sync_request, Some(7));

        supervisor
            .complete_sync_request(7, Err(CloudBackupError::Internal("current completion".into())))
            .await
            .unwrap();

        assert_eq!(supervisor.active_sync_request, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_sync_refresh_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        supervisor.active_sync_request = Some(7);

        supervisor.complete_sync_request_refresh_detail(6, None).await.unwrap();

        assert_eq!(supervisor.active_sync_request, Some(7));

        supervisor.complete_sync_request_refresh_detail(7, None).await.unwrap();

        assert_eq!(supervisor.active_sync_request, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_cloud_only_fetch_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        supervisor.active_cloud_only_fetch_request = Some(7);

        supervisor.complete_cloud_only_fetch_request(6, Ok(Vec::new())).await.unwrap();

        assert_eq!(supervisor.active_cloud_only_fetch_request, Some(7));

        supervisor.complete_cloud_only_fetch_request(7, Ok(Vec::new())).await.unwrap();

        assert_eq!(supervisor.active_cloud_only_fetch_request, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_recreate_manifest_verification_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RecreateManifest,
            u64::MAX,
        );

        supervisor
            .complete_recreate_manifest_verification(
                stale,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                VerificationAttempt::Initial,
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_recreate_manifest_verification(
                current,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                VerificationAttempt::Initial,
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_deep_verification_wrapper_repair_upload_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::VerificationRepair,
            u64::MAX,
        );
        let continuation = DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        };

        supervisor
            .complete_deep_verification_wrapper_repair_upload(
                stale,
                continuation,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_deep_verification_wrapper_repair_upload(
                current,
                continuation,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_deep_verification_wrapper_repair_resume_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::VerificationRepair,
            u64::MAX,
        );
        let continuation = DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        };

        supervisor
            .complete_deep_verification_wrapper_repair_resume(
                stale,
                continuation,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_deep_verification_wrapper_repair_resume(
                current,
                continuation,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_deep_verification_auto_sync_upload_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::VerificationRepair,
            u64::MAX,
        );
        let continuation = DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        };

        supervisor
            .complete_deep_verification_auto_sync_upload(
                stale,
                continuation,
                Err(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_deep_verification_auto_sync_upload(
                current,
                continuation,
                Err(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_deep_verification_auto_sync_finalization_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::VerificationRepair,
            u64::MAX,
        );
        let continuation = DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        };

        supervisor
            .complete_deep_verification_auto_sync_finalization(
                stale,
                continuation,
                Err(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_deep_verification_auto_sync_finalization(
                current,
                continuation,
                Err(DeepVerificationResult::NotEnabled),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_deep_verification_auto_sync_resume_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::VerificationRepair,
            u64::MAX,
        );
        let continuation = DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        };

        supervisor
            .complete_deep_verification_auto_sync_resume(
                stale,
                continuation,
                CloudBackupDeepVerificationAutoSyncCompletion::complete(
                    DeepVerificationResult::NotEnabled,
                ),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_deep_verification_auto_sync_resume(
                current,
                continuation,
                CloudBackupDeepVerificationAutoSyncCompletion::complete(
                    DeepVerificationResult::NotEnabled,
                ),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_reinitialize_backup_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::ReinitializeBackup,
            u64::MAX,
        );

        supervisor
            .complete_enable_preparation(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_preparation(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_reinitialize_verification_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::ReinitializeBackup,
            u64::MAX,
        );

        supervisor
            .complete_reinitialize_verification(
                stale,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                VerificationAttempt::Initial,
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_reinitialize_verification(
                current,
                CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                VerificationAttempt::Initial,
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_saved_passkey_confirmation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_saved_passkey_confirmation(
                stale,
                CloudBackupSavedPasskeyConfirmation::Failed(CloudBackupError::Internal(
                    "stale completion".into(),
                )),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_saved_passkey_confirmation(
                current,
                CloudBackupSavedPasskeyConfirmation::Failed(CloudBackupError::Internal(
                    "current completion".into(),
                )),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_passkey_registration_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::EnableForceNew,
            u64::MAX,
        );

        supervisor
            .complete_enable_passkey_registration(
                stale,
                Ok(CloudBackupEnablePasskeyRegistration::Cancelled {
                    context: CloudBackupEnableContext::settings_manual(),
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_passkey_registration(
                current,
                Ok(CloudBackupEnablePasskeyRegistration::Cancelled {
                    context: CloudBackupEnableContext::settings_manual(),
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_preparation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_preparation(
                stale,
                Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                    context: CloudBackupEnableContext::settings_manual(),
                    passkey_hint: None,
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_preparation(
                current,
                Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                    context: CloudBackupEnableContext::settings_manual(),
                    passkey_hint: None,
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_create_new_enable_passkey_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_create_new_enable_passkey(
                stale,
                Ok(CloudBackupEnablePasskeyPreparation::Cancelled {
                    context: CloudBackupEnableContext::settings_manual(),
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_create_new_enable_passkey(
                current,
                Ok(CloudBackupEnablePasskeyPreparation::Cancelled {
                    context: CloudBackupEnableContext::settings_manual(),
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_recovery_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_recovery(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_recovery(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_recovery_finalization_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_recovery_finalization(
                stale,
                EnableRecoveryFinalization {
                    namespace_id: "stale-namespace".into(),
                    active_critical_key: zeroize::Zeroizing::new([0; 32]),
                    cleanup_sources: Vec::new(),
                },
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_recovery_finalization(
                current,
                EnableRecoveryFinalization {
                    namespace_id: "current-namespace".into(),
                    active_critical_key: zeroize::Zeroizing::new([0; 32]),
                    cleanup_sources: Vec::new(),
                },
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_recovery_preparation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_recovery_preparation(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_recovery_preparation(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_no_discovery_enable_preparation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableNoDiscovery)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::EnableNoDiscovery,
            u64::MAX,
        );

        supervisor
            .complete_no_discovery_enable_preparation(
                stale,
                Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                    context: CloudBackupEnableContext::settings_manual(),
                    passkey_hint: None,
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_no_discovery_enable_preparation(
                current,
                Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                    context: CloudBackupEnableContext::settings_manual(),
                    passkey_hint: None,
                }),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_accepts_registered_enable_passkey_confirmation() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
            .unwrap();
        let registered = CloudBackupRegisteredEnablePasskey {
            master_key: zeroize::Zeroizing::new(master_key),
            passkey: zeroize::Zeroizing::new(test_staged_passkey(expected_credential_id.clone())),
            context: CloudBackupEnableContext::settings_manual(),
        };

        supervisor
            .complete_enable_passkey_registration(
                current,
                Ok(CloudBackupEnablePasskeyRegistration::Registered(registered)),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));
        let pending = supervisor.pending_enable_session.take().unwrap();
        let (pending_master_key, pending_passkey) = pending.into_staged_parts().unwrap();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_consumes_retry_pending_enable_upload() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];

        supervisor.pending_enable_session = Some(PendingEnableSession::retry_upload(
            master_key,
            test_enable_passkey(expected_credential_id.clone()),
            CloudBackupEnableContext::settings_manual(),
        ));

        let ready = supervisor
            .take_ready_enable_upload(PendingEnableUploadSelection::RetryOnly)
            .unwrap()
            .unwrap();

        assert_eq!(ready.master_key.namespace_id(), expected_namespace);
        assert_eq!(ready.passkey.credential_id, expected_credential_id);
        assert!(supervisor.pending_enable_session.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_preserves_force_new_confirmation_for_plain_enable_retry() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        let master_key = cove_cspp::master_key::MasterKey::generate();

        supervisor.pending_enable_session =
            Some(awaiting_force_new_session(master_key, test_enable_passkey(vec![1, 2, 3])));

        let ready =
            supervisor.take_ready_enable_upload(PendingEnableUploadSelection::RetryOnly).unwrap();

        assert!(ready.is_none());
        assert!(supervisor.pending_enable_session.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_consumes_force_new_confirmation_upload_for_force_new() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];

        supervisor.pending_enable_session = Some(awaiting_force_new_session(
            master_key,
            test_enable_passkey(expected_credential_id.clone()),
        ));

        let ready = supervisor
            .take_ready_enable_upload(PendingEnableUploadSelection::RetryOrForceNewConfirmation)
            .unwrap()
            .unwrap();

        assert_eq!(ready.master_key.namespace_id(), expected_namespace);
        assert_eq!(ready.passkey.credential_id, expected_credential_id);
        assert!(supervisor.pending_enable_session.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_upload_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_upload(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_upload(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_enable_upload_finalization_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Enable,
            u64::MAX,
        );

        supervisor
            .complete_enable_upload_finalization(
                stale,
                test_enable_upload_finalization(),
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_enable_upload_finalization(
                current,
                test_enable_upload_finalization(),
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_disable_preparation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            u64::MAX,
        );

        supervisor
            .complete_disable_preparation(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_disable_preparation(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_disable_blocker_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            u64::MAX,
        );
        let disabling = test_disabling_state();

        supervisor
            .complete_disable_blocker_check(
                stale,
                disabling.clone(),
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_disable_blocker_check(
                current,
                disabling,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_disable_delete_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            u64::MAX,
        );
        let disabling = test_disabling_state();

        supervisor
            .complete_disable_namespace_delete(
                stale,
                disabling.clone(),
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_disable_namespace_delete(
                current,
                disabling,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_disable_local_cleanup_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Disable,
            u64::MAX,
        );
        let disabling = test_disabling_state();

        supervisor
            .complete_disable_local_cleanup(
                stale,
                disabling.clone(),
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_disable_local_cleanup(
                current,
                disabling,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_recover_other_backups_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = CloudBackupSupervisor::begin_other_backups_operation(
            &mut supervisor,
            &manager,
            CloudBackupExclusiveOperation::RecoverOtherBackups,
            CloudBackupOtherBackupsOutcome::Recovering,
        )
        .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RecoverOtherBackups,
            u64::MAX,
        );
        supervisor
            .complete_recover_other_backups(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_recover_other_backups(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_delete_cloud_wallet_preparation_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = supervisor
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
            .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::DeleteCloudWallet,
            u64::MAX,
        );

        supervisor
            .complete_delete_cloud_wallet_preparation(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_delete_cloud_wallet_preparation(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_ignores_stale_delete_other_backups_completion() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let mut supervisor = CloudBackupSupervisor::new(
            Arc::downgrade(&manager),
            spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
        );

        let current = CloudBackupSupervisor::begin_other_backups_operation(
            &mut supervisor,
            &manager,
            CloudBackupExclusiveOperation::DeleteOtherBackups,
            CloudBackupOtherBackupsOutcome::Deleting,
        )
        .unwrap();
        let stale = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::DeleteOtherBackups,
            u64::MAX,
        );

        supervisor
            .complete_delete_other_backups(
                stale,
                Err(CloudBackupError::Internal("stale completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, Some(current));
        assert_eq!(manager.projected_exclusive_operation(), Some(current));

        supervisor
            .complete_delete_other_backups(
                current,
                Err(CloudBackupError::Internal("current completion".into())),
            )
            .await
            .unwrap();

        assert_eq!(supervisor.active_operation, None);
        assert_eq!(manager.projected_exclusive_operation(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_routes_write_blocker_commands_to_write_supervisor() {
        let _guard = async_test_lock().lock().await;
        let manager = test_supervisor_manager();
        let writes = spawn_actor(CloudBackupWriteSupervisor::new(Weak::new()));
        let mut supervisor = CloudBackupSupervisor::new(Arc::downgrade(&manager), writes.clone());
        let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 9 };

        call!(writes.block(blocker)).await.unwrap();

        let blocked_write = call!(writes.upload_wallet_backup(
            CloudStorage::global_explicit_client(),
            "namespace".into(),
            "record".into(),
            vec![1, 2, 3]
        ))
        .await
        .unwrap()
        .await
        .unwrap();
        let blocked_write = blocked_write.into_result();
        assert!(matches!(blocked_write, Err(CloudBackupError::Deferred(_))));

        supervisor.unblock_cloud_backup_writes(blocker).await.unwrap();

        let allowed_write = call!(writes.upload_wallet_backup(
            CloudStorage::global_explicit_client(),
            "namespace".into(),
            "record".into(),
            vec![1, 2, 3]
        ))
        .await
        .unwrap()
        .await
        .unwrap();
        let allowed_write = allowed_write.into_result();
        assert!(allowed_write.is_ok());
    }
}

#[cfg(test)]
pub(crate) mod test_support {
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
