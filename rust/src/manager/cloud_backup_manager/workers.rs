mod cleanup;
mod restore;
mod sync_health;
mod uploads;

use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, call, send};
use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use cove_device::cloud_storage::CloudStorage;
use cove_tokio::task::spawn_actor;
use tracing::{error, warn};

use self::cleanup::CloudBackupCleanupWorker;
pub(crate) use self::cleanup::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob,
};
use self::restore::CloudBackupRestoreWorker;
pub(crate) use self::restore::{RestoreOperation, RestoredPasskeyMaterial};
use self::sync_health::CloudBackupSyncHealthWorker;
use self::uploads::CloudBackupUploadWorker;
use super::keychain::CloudBackupKeychain;
use super::{
    CloudBackupDetailResult, CloudBackupStatus, DeepVerificationResult, OtherBackupsOperation,
    PendingEnableSession, PendingVerificationCompletion, RecoveryAction, RustCloudBackupManager,
    VerificationState, WalletId,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

#[derive(Debug, Clone)]
pub(crate) enum CloudBackupOperation {
    Enable,
    EnableForceNew,
    EnableNoDiscovery,
    Recovery { action: RecoveryAction },
    RepairPasskey { no_discovery: bool },
    Sync,
    FetchCloudOnly,
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

fn should_retry_connectivity_failure(status: ConnectivityStatus) -> bool {
    matches!(status, ConnectivityStatus::Unknown | ConnectivityStatus::Connected)
}

fn apply_refresh_detail_result(manager: &RustCloudBackupManager, result: &CloudBackupDetailResult) {
    match result {
        CloudBackupDetailResult::Success(detail) => {
            manager.set_detail(Some(detail.clone()));
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
    restore: Addr<CloudBackupRestoreWorker>,
    sync_health: Addr<CloudBackupSyncHealthWorker>,
    uploads: Addr<CloudBackupUploadWorker>,
    pending_enable_session: Option<PendingEnableSession>,
    pending_verification_completion: Option<PendingVerificationCompletion>,
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
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self {
            addr: WeakAddr::default(),
            cleanup: spawn_actor(CloudBackupCleanupWorker::new(manager.clone())),
            restore: spawn_actor(CloudBackupRestoreWorker::new(manager.clone())),
            sync_health: spawn_actor(CloudBackupSyncHealthWorker::new(manager.clone())),
            uploads: spawn_actor(CloudBackupUploadWorker::new(manager.clone())),
            manager,
            pending_enable_session: None,
            pending_verification_completion: None,
            runtime_passkey_authorization: None,
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Option<Addr<Self>> {
        Some(self.addr.upgrade())
    }

    fn spawn_operation(&self, operation: CloudBackupOperation, record_id: Option<String>) {
        let Some(manager) = self.manager() else { return };

        match operation {
            CloudBackupOperation::Enable => {
                if !manager.begin_background_operation(
                    "enable_cloud_backup",
                    Some(CloudBackupStatus::Enabling),
                ) {
                    return;
                }
                cove_tokio::task::spawn(async move {
                    if let Err(error) = manager.do_enable_cloud_backup().await {
                        error!("enable_cloud_backup failed: {error}");
                        manager.finish_background_operation_error(&error);
                    }
                });
            }
            CloudBackupOperation::EnableForceNew => {
                if !manager.begin_background_operation(
                    "enable_cloud_backup_force_new",
                    Some(CloudBackupStatus::Enabling),
                ) {
                    return;
                }
                cove_tokio::task::spawn(async move {
                    if let Err(error) = manager.do_enable_cloud_backup_force_new().await {
                        error!("enable_cloud_backup_force_new failed: {error}");
                        manager.finish_background_operation_error(&error);
                    }
                });
            }
            CloudBackupOperation::EnableNoDiscovery => {
                if !manager.begin_background_operation(
                    "enable_cloud_backup_no_discovery",
                    Some(CloudBackupStatus::Enabling),
                ) {
                    return;
                }
                cove_tokio::task::spawn(async move {
                    if let Err(error) = manager.do_enable_cloud_backup_no_discovery().await {
                        error!("enable_cloud_backup_no_discovery failed: {error}");
                        manager.finish_background_operation_error(&error);
                    }
                });
            }
            CloudBackupOperation::Recovery { action } => {
                cove_tokio::task::spawn(async move { manager.handle_recovery(action).await });
            }
            CloudBackupOperation::RepairPasskey { no_discovery } => {
                cove_tokio::task::spawn(async move {
                    manager.handle_repair_passkey(no_discovery).await
                });
            }
            CloudBackupOperation::Sync => {
                cove_tokio::task::spawn(async move { manager.handle_sync().await });
            }
            CloudBackupOperation::FetchCloudOnly => {
                cove_tokio::task::spawn(async move { manager.handle_fetch_cloud_only().await });
            }
            CloudBackupOperation::RestoreCloudWallet => {
                let Some(record_id) = record_id else { return };
                cove_tokio::task::spawn(async move {
                    manager.handle_restore_cloud_wallet(&record_id).await
                });
            }
            CloudBackupOperation::DeleteCloudWallet => {
                let Some(record_id) = record_id else { return };
                cove_tokio::task::spawn(async move {
                    manager.handle_delete_cloud_wallet(&record_id).await
                });
            }
            CloudBackupOperation::RecoverOtherBackups => {
                if !Self::begin_other_backups_operation(&manager, OtherBackupsOperation::Recovering)
                {
                    return;
                }

                cove_tokio::task::spawn(
                    async move { manager.handle_recover_other_backups().await },
                );
            }
            CloudBackupOperation::DeleteOtherBackups => {
                if !Self::begin_other_backups_operation(&manager, OtherBackupsOperation::Deleting) {
                    return;
                }

                cove_tokio::task::spawn(async move { manager.handle_delete_other_backups().await });
            }
        }
    }

    fn begin_other_backups_operation(
        manager: &RustCloudBackupManager,
        operation: OtherBackupsOperation,
    ) -> bool {
        if !matches!(
            &manager.state.read().other_backups_operation,
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return false;
        }

        manager.set_other_backups_operation(operation);
        true
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
        manager.set_verification(VerificationState::Verifying);
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
            let result = manager.deep_verify_cloud_backup(force_discoverable).await;
            send!(addr.complete_verification(result, force_discoverable, attempt));
        });
    }

    pub async fn complete_verification(
        &mut self,
        result: DeepVerificationResult,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            self.schedule_verification(
                manager,
                force_discoverable,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.handle_deep_verification_result(result);
        Produces::ok(())
    }

    fn detail_entry_plan(&self, manager: &RustCloudBackupManager) -> DetailEntryPlan {
        let state = manager.state.read().clone();
        if !matches!(state.status, CloudBackupStatus::Enabled) {
            return DetailEntryPlan::RefreshOnly;
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

    pub async fn replace_pending_enable_session(
        &mut self,
        session: PendingEnableSession,
    ) -> ActorResult<()> {
        self.pending_enable_session = Some(session);
        Produces::ok(())
    }

    pub async fn take_pending_enable_session(
        &mut self,
    ) -> ActorResult<Option<PendingEnableSession>> {
        Produces::ok(self.pending_enable_session.take())
    }

    pub async fn take_retry_pending_enable_session(
        &mut self,
    ) -> ActorResult<Option<PendingEnableSession>> {
        let pending = self.pending_enable_session.take();
        if pending.as_ref().is_some_and(PendingEnableSession::is_retry_upload) {
            return Produces::ok(pending);
        }

        self.pending_enable_session = pending;
        Produces::ok(None)
    }

    pub async fn has_awaiting_force_new_pending_enable_session(&self) -> ActorResult<bool> {
        Produces::ok(
            self.pending_enable_session
                .as_ref()
                .is_some_and(PendingEnableSession::is_awaiting_force_new_confirmation),
        )
    }

    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn record_runtime_passkey_authorization(
        &mut self,
        namespace_id: String,
        credential_id: Vec<u8>,
        prf_salt: [u8; 32],
    ) -> ActorResult<()> {
        self.runtime_passkey_authorization =
            Some(RuntimePasskeyAuthorization { namespace_id, credential_id, prf_salt });
        Produces::ok(())
    }

    pub async fn clear_runtime_passkey_authorization(&mut self) -> ActorResult<()> {
        self.runtime_passkey_authorization = None;
        Produces::ok(())
    }

    pub async fn discard_pending_enable_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(pending) = self.pending_enable_session.take() else {
            return Produces::ok(());
        };

        let should_delete_remote = pending.is_retry_upload();
        let namespace_id = pending.namespace_id();

        if let Err(error) = CloudBackupKeychain::global().clear_local_state() {
            warn!("Discard pending enable failed to clear local cloud backup state: {error}");
        }

        if should_delete_remote {
            cove_tokio::task::spawn(async move {
                if let Err(error) = CloudStorage::global_explicit_client()
                    .delete_wallet_backup(namespace_id, MASTER_KEY_RECORD_ID.to_string())
                    .await
                {
                    warn!("Discard pending enable failed to delete remote master key: {error}");
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

    #[cfg(test)]
    pub async fn run_wallet_upload_inline_for_test(
        &mut self,
        wallet_id: WalletId,
    ) -> ActorResult<()> {
        call!(self.uploads.run_wallet_upload_inline_for_test(wallet_id)).await?;
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

    #[cfg(test)]
    pub async fn new_restore_operation(&mut self) -> ActorResult<RestoreOperation> {
        let operation = call!(self.restore.new_restore_operation()).await?;
        Produces::ok(operation)
    }

    #[cfg(test)]
    pub async fn invalidate_restore_operation(&mut self) -> ActorResult<()> {
        call!(self.restore.invalidate_restore_operation()).await?;
        Produces::ok(())
    }

    pub async fn start_restore_from_cloud_backup(&mut self) -> ActorResult<()> {
        call!(self.restore.start_restore_from_cloud_backup()).await?;
        Produces::ok(())
    }

    pub async fn cancel_restore(&mut self) -> ActorResult<()> {
        call!(self.restore.cancel_restore()).await?;
        Produces::ok(())
    }

    pub async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        call!(self.sync_health.clear_upload_runtime_state()).await?;
        call!(self.uploads.clear_upload_runtime_state()).await?;
        Produces::ok(())
    }

    pub async fn enqueue_cleanup(&mut self, job: CloudBackupCleanupJob) -> ActorResult<()> {
        call!(self.cleanup.enqueue_cleanup(job)).await?;
        Produces::ok(())
    }
}
