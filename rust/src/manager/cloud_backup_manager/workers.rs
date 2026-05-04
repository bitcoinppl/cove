mod restore;
mod sync_health;
mod uploads;

use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, call, send};
use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use cove_device::cloud_storage::CloudStorage;
use cove_tokio::task::spawn_actor;
use tracing::{error, warn};

use self::restore::CloudBackupRestoreWorker;
pub(crate) use self::restore::{RestoreOperation, RestoredPasskeyMaterial};
use self::sync_health::CloudBackupSyncHealthWorker;
pub(crate) use self::sync_health::SyncHealthWorkerState;
use self::uploads::CloudBackupUploadWorker;
use super::keychain::CloudBackupKeychain;
use super::{
    CloudBackupDetailResult, CloudBackupStatus, DeepVerificationResult, PendingEnableSession,
    PendingVerificationCompletion, RecoveryAction, RustCloudBackupManager, VerificationState,
    WalletId,
};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimePasskeyProof {
    namespace_id: String,
    credential_id: Vec<u8>,
    prf_salt: [u8; 32],
}

/// Detail entry decision captured before async refresh
///
/// This preserves the entry-time verification intent while detail refresh updates
/// cloud-backed state in the background
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailEntryDecision {
    RefreshOnly,
    ContinueRustOwnedVerification,
    StartPasskeyVerification,
}

fn apply_refresh_detail_result(
    manager: &RustCloudBackupManager,
    result: Option<CloudBackupDetailResult>,
) {
    let Some(result) = result else { return };
    match result {
        CloudBackupDetailResult::Success(detail) => {
            manager.set_detail(Some(detail));
        }
        CloudBackupDetailResult::AccessError(error) => {
            error!("Failed to refresh detail: {error}");
        }
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupSupervisor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    restore: Addr<CloudBackupRestoreWorker>,
    sync_health: Addr<CloudBackupSyncHealthWorker>,
    uploads: Addr<CloudBackupUploadWorker>,
    pending_enable_session: Option<PendingEnableSession>,
    pending_verification_completion: Option<PendingVerificationCompletion>,
    // runtime-only proof that this app session just produced matching passkey material
    // clearing it when the supervisor is recreated makes detail entry re-check passkey availability
    runtime_passkey_proof: Option<RuntimePasskeyProof>,
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
            restore: spawn_actor(CloudBackupRestoreWorker::new(manager.clone())),
            sync_health: spawn_actor(CloudBackupSyncHealthWorker::new(manager.clone())),
            uploads: spawn_actor(CloudBackupUploadWorker::new(manager.clone())),
            manager,
            pending_enable_session: None,
            pending_verification_completion: None,
            runtime_passkey_proof: None,
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
        }
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
        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        manager.refresh_sync_health();
        cove_tokio::task::spawn(async move {
            let result = manager.refresh_cloud_backup_detail().await;
            send!(addr.complete_refresh_detail(result));
        });

        Produces::ok(())
    }

    pub async fn complete_refresh_detail(
        &mut self,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        apply_refresh_detail_result(&manager, result);
        Produces::ok(())
    }

    pub async fn start_enter_detail(&mut self) -> ActorResult<()> {
        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        // decide before refresh so a fresh passkey enable is not immediately re-prompted
        let decision = self.detail_entry_decision(&manager);

        manager.refresh_sync_health();
        cove_tokio::task::spawn(async move {
            let result = manager.refresh_cloud_backup_detail().await;
            send!(addr.complete_enter_detail(result, decision));
        });

        Produces::ok(())
    }

    pub async fn complete_enter_detail(
        &mut self,
        result: Option<CloudBackupDetailResult>,
        decision: DetailEntryDecision,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        apply_refresh_detail_result(&manager, result);

        if matches!(decision, DetailEntryDecision::StartPasskeyVerification)
            && let Some(addr) = self.addr()
        {
            send!(addr.start_verification(true));
        }

        Produces::ok(())
    }

    pub async fn start_verification(&mut self, force_discoverable: bool) -> ActorResult<()> {
        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.pending_verification_completion = None;
        manager.set_verification(VerificationState::Verifying);
        cove_tokio::task::spawn(async move {
            let result = manager.deep_verify_cloud_backup(force_discoverable).await;
            send!(addr.complete_verification(result));
        });

        Produces::ok(())
    }

    pub async fn complete_verification(
        &mut self,
        result: DeepVerificationResult,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        manager.apply_deep_verification_result(result);
        Produces::ok(())
    }

    fn detail_entry_decision(&self, manager: &RustCloudBackupManager) -> DetailEntryDecision {
        let state = manager.state.read().clone();
        if !matches!(state.status, CloudBackupStatus::Enabled) {
            return DetailEntryDecision::RefreshOnly;
        }

        let can_continue_without_passkey_prompt = state.has_pending_upload_verification
            || self.pending_verification_completion.is_some()
            || self.runtime_passkey_proof_matches_current_manager(manager)
            || matches!(
                state.verification,
                VerificationState::Verifying
                    | VerificationState::Verified(_)
                    | VerificationState::PasskeyConfirmed
            );
        if can_continue_without_passkey_prompt {
            return DetailEntryDecision::ContinueRustOwnedVerification;
        }

        DetailEntryDecision::StartPasskeyVerification
    }

    fn runtime_passkey_proof_matches_current_manager(
        &self,
        manager: &RustCloudBackupManager,
    ) -> bool {
        let Some(proof) = self.runtime_passkey_proof.as_ref() else {
            return false;
        };
        let Ok(namespace_id) = manager.current_namespace_id() else {
            return false;
        };

        let cloud_keychain = CloudBackupKeychain::global();
        let Some(credential_id) = cloud_keychain.load_credential_id() else {
            return false;
        };
        let Some(prf_salt) = cloud_keychain.load_prf_salt() else {
            return false;
        };

        proof.namespace_id == namespace_id
            && proof.credential_id == credential_id
            && proof.prf_salt == prf_salt
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

    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn set_runtime_passkey_proof(
        &mut self,
        namespace_id: String,
        credential_id: Vec<u8>,
        prf_salt: [u8; 32],
    ) -> ActorResult<()> {
        self.runtime_passkey_proof =
            Some(RuntimePasskeyProof { namespace_id, credential_id, prf_salt });
        Produces::ok(())
    }

    pub async fn clear_runtime_passkey_proof(&mut self) -> ActorResult<()> {
        self.runtime_passkey_proof = None;
        Produces::ok(())
    }

    pub async fn discard_pending_enable_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(pending) = self.pending_enable_session.take() else {
            return Produces::ok(());
        };

        let should_delete_remote = pending.is_retry_upload();
        let namespace_id = pending.master_key.namespace_id();

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
}
