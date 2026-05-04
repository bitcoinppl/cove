use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, call, send};
use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use cove_device::cloud_storage::{CloudStorage, CloudSyncHealth};
use cove_device::keychain::Keychain;
use cove_tokio::DebouncedTask;
use cove_util::ResultExt as _;
use cove_util::{GenerationClaim, GenerationToken, GenerationTracker};
use tokio::sync::Notify;
use tracing::{error, info, warn};

use super::keychain::CloudBackupKeychain;
use super::pending::{
    MAX_PENDING_UPLOAD_VERIFICATION_DELAY, PendingUploadVerificationStatus,
    build_pending_upload_backoff,
};
use super::{
    CloudBackupDetailResult, CloudBackupError, CloudBackupRestoreProgress,
    CloudBackupRestoreReport, CloudBackupStatus, DeepVerificationResult, PendingEnableSession,
    PendingVerificationCompletion, RecoveryAction, RustCloudBackupManager, VerificationState,
    WalletId, live_upload_retry_delay_for_attempt,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobFailedState, CloudBlobFailureIssue, PersistedCloudBackupState, PersistedCloudBlobState,
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

#[derive(Clone, Debug)]
pub(crate) struct RestoreOperation {
    active_restore_generation: GenerationClaim,
    runtime: Addr<CloudBackupRuntimeActor>,
}

impl RestoreOperation {
    fn new(tracker: GenerationTracker, runtime: Addr<CloudBackupRuntimeActor>) -> Self {
        Self { active_restore_generation: tracker.claim(), runtime }
    }

    fn generation(&self) -> GenerationToken {
        self.active_restore_generation.token()
    }

    pub(crate) async fn ensure_current(&self) -> Result<(), CloudBackupError> {
        call!(self.runtime.ensure_restore_current(self.generation()))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn set_status(
        &self,
        status: CloudBackupStatus,
    ) -> Result<(), CloudBackupError> {
        call!(self.runtime.set_restore_status(self.generation(), status))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn set_progress(
        &self,
        progress: Option<CloudBackupRestoreProgress>,
    ) -> Result<(), CloudBackupError> {
        call!(self.runtime.set_restore_progress(self.generation(), progress))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn set_report(
        &self,
        report: Option<CloudBackupRestoreReport>,
    ) -> Result<(), CloudBackupError> {
        call!(self.runtime.set_restore_report(self.generation(), report))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn persist_cloud_backup_state(
        &self,
        state: PersistedCloudBackupState,
        context: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.runtime.persist_restore_cloud_backup_state(self.generation(), state, context))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn save_keychain_state(
        &self,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.runtime.save_restore_keychain_state(
            self.generation(),
            master_key,
            passkey,
            namespace_id
        ))
        .await
        .map_err(|_| CloudBackupError::Cancelled)?
    }
}

#[derive(Debug)]
pub(crate) struct RestoredPasskeyMaterial {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
}

fn is_authorization_failed_blob(failed_state: &CloudBlobFailedState) -> bool {
    failed_state.issue == Some(CloudBlobFailureIssue::AuthorizationRequired)
}

fn should_retry_failed_blob(failed_state: &CloudBlobFailedState) -> bool {
    failed_state.retryable || is_authorization_failed_blob(failed_state)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SyncHealthRefreshState {
    Idle,
    Running,
    RunningQueued,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SyncHealthRuntimeState {
    master_key_upload_grace_namespace: Option<String>,
}

impl SyncHealthRuntimeState {
    pub(crate) fn master_key_upload_in_grace(&self, namespace_id: &str) -> bool {
        self.master_key_upload_grace_namespace.as_deref() == Some(namespace_id)
    }

    #[cfg(test)]
    pub(crate) fn with_master_key_upload_grace(namespace_id: String) -> Self {
        Self { master_key_upload_grace_namespace: Some(namespace_id) }
    }
}

#[derive(Debug)]
struct MasterKeyUploadGrace {
    namespace_id: String,
    generation: GenerationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimePasskeyProof {
    namespace_id: String,
    credential_id: Vec<u8>,
    prf_salt: [u8; 32],
}

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
pub(crate) struct CloudBackupRuntimeActor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    pending_enable_session: Option<PendingEnableSession>,
    pending_verification_completion: Option<PendingVerificationCompletion>,
    runtime_passkey_proof: Option<RuntimePasskeyProof>,
    pending_upload_verifier_running: bool,
    pending_upload_verifier_blocked_on_authorization: bool,
    pending_upload_verifier_wakeup: Arc<Notify>,
    master_key_upload_grace: Option<MasterKeyUploadGrace>,
    master_key_upload_grace_generations: GenerationTracker,
    sync_health_refresh_state: SyncHealthRefreshState,
    sync_health_refresh_generations: GenerationTracker,
    wallet_upload_debouncers: HashMap<WalletId, DebouncedTask<()>>,
    wallet_upload_retry_counts: HashMap<WalletId, u32>,
    active_wallet_uploads: HashSet<WalletId>,
    restore_operations: GenerationTracker,
}

#[async_trait::async_trait]
impl Actor for CloudBackupRuntimeActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupRuntimeActor {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self {
            addr: WeakAddr::default(),
            manager,
            pending_enable_session: None,
            pending_verification_completion: None,
            runtime_passkey_proof: None,
            pending_upload_verifier_running: false,
            pending_upload_verifier_blocked_on_authorization: false,
            pending_upload_verifier_wakeup: Arc::new(Notify::new()),
            master_key_upload_grace: None,
            master_key_upload_grace_generations: GenerationTracker::new(),
            sync_health_refresh_state: SyncHealthRefreshState::Idle,
            sync_health_refresh_generations: GenerationTracker::new(),
            wallet_upload_debouncers: HashMap::new(),
            wallet_upload_retry_counts: HashMap::new(),
            active_wallet_uploads: HashSet::new(),
            restore_operations: GenerationTracker::new(),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Option<Addr<Self>> {
        Some(self.addr.upgrade())
    }

    fn sync_health_runtime_state(&self) -> SyncHealthRuntimeState {
        SyncHealthRuntimeState {
            master_key_upload_grace_namespace: self
                .master_key_upload_grace
                .as_ref()
                .map(|grace| grace.namespace_id.clone()),
        }
    }

    fn spawn_sync_health_refresh_task(&mut self, addr: Addr<Self>) {
        self.sync_health_refresh_state = SyncHealthRefreshState::Running;
        let Some(manager) = self.manager() else {
            self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
            return;
        };

        let generation = self.sync_health_refresh_generations.advance();
        let runtime_state = self.sync_health_runtime_state();
        cove_tokio::task::spawn(async move {
            let sync_health = manager.compute_sync_health(runtime_state).await;
            send!(addr.complete_sync_health_refresh(generation, sync_health));
        });
    }

    fn spawn_pending_upload_verification_loop_task(
        &mut self,
        addr: Addr<Self>,
        manager: Arc<RustCloudBackupManager>,
    ) {
        self.pending_upload_verifier_running = true;
        self.pending_upload_verifier_blocked_on_authorization = false;
        let wakeup = Arc::clone(&self.pending_upload_verifier_wakeup);
        cove_tokio::task::spawn(async move {
            info!("Pending upload verification: started");
            let mut backoff = PendingUploadRetryBackoff::new();
            let mut blocked_on_authorization = false;

            loop {
                match manager.verify_pending_uploads_once().await {
                    PendingUploadVerificationStatus::Idle => break,
                    PendingUploadVerificationStatus::BlockedOnAuthorization => {
                        blocked_on_authorization = true;
                        break;
                    }
                    PendingUploadVerificationStatus::Pending => {}
                }

                let delay = backoff.next_delay();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wakeup.notified() => backoff.reset(),
                }
            }

            send!(addr.pending_upload_verifier_finished(blocked_on_authorization));
        });
    }

    fn schedule_wallet_upload_after(&mut self, wallet_id: WalletId, delay: Duration) {
        let task = DebouncedTask::new("cloud_wallet_backup_upload", delay);
        self.wallet_upload_debouncers.insert(wallet_id.clone(), task.clone());

        let Some(addr) = self.addr() else { return };
        task.replace(async move {
            send!(addr.run_wallet_upload(wallet_id));
        });
    }

    fn next_wallet_upload_retry_delay(&mut self, wallet_id: &WalletId) -> Duration {
        let retry_count = self.wallet_upload_retry_counts.entry(wallet_id.clone()).or_default();
        let delay = live_upload_retry_delay_for_attempt(*retry_count);
        *retry_count = retry_count.saturating_add(1);
        delay
    }

    fn reset_wallet_upload_retry_count(&mut self, wallet_id: &WalletId) {
        self.wallet_upload_retry_counts.remove(wallet_id);
    }

    fn schedule_wallet_upload_follow_up(&mut self, wallet_id: WalletId) {
        let Some(manager) = self.manager() else { return };
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        let sync_state = match crate::database::Database::global()
            .cloud_blob_sync_states
            .get(&record_id)
        {
            Ok(sync_state) => sync_state,
            Err(error) => {
                error!(
                    "Failed to read wallet upload follow-up state for record_id={record_id}: {error}"
                );
                return;
            }
        };

        let Some(sync_state) = sync_state else {
            return;
        };

        if sync_state.is_dirty() {
            self.reset_wallet_upload_retry_count(&wallet_id);
            if let Some(addr) = self.addr() {
                send!(addr.run_wallet_upload(wallet_id));
            }
            return;
        }

        match sync_state.state {
            PersistedCloudBlobState::Failed(failed_state)
                if should_retry_failed_blob(&failed_state) =>
            {
                let delay = self.next_wallet_upload_retry_delay(&wallet_id);
                self.schedule_wallet_upload_after(wallet_id, delay);
            }
            PersistedCloudBlobState::Failed(_) => {
                self.reset_wallet_upload_retry_count(&wallet_id);
            }
            PersistedCloudBlobState::Uploading(_)
            | PersistedCloudBlobState::UploadedPendingConfirmation(_)
            | PersistedCloudBlobState::Confirmed(_) => {}
            PersistedCloudBlobState::Dirty(_) => {
                warn!("dirty upload follow-up should have been handled earlier");
            }
        }

        manager.set_pending_upload_verification(manager.has_pending_cloud_upload_verification());
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

    fn restore_generation_is_current(&self, generation: GenerationToken) -> bool {
        self.restore_operations.is_current(generation)
    }

    pub async fn ensure_restore_current(
        &mut self,
        generation: GenerationToken,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if self.restore_generation_is_current(generation) {
            Produces::ok(Ok(()))
        } else {
            Produces::ok(Err(CloudBackupError::Cancelled))
        }
    }

    pub async fn set_restore_status(
        &mut self,
        generation: GenerationToken,
        status: CloudBackupStatus,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        manager.set_status(status);
        Produces::ok(Ok(()))
    }

    pub async fn set_restore_progress(
        &mut self,
        generation: GenerationToken,
        progress: Option<CloudBackupRestoreProgress>,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        manager.set_restore_progress(progress);
        Produces::ok(Ok(()))
    }

    pub async fn set_restore_report(
        &mut self,
        generation: GenerationToken,
        report: Option<CloudBackupRestoreReport>,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        manager.set_restore_report(report);
        Produces::ok(Ok(()))
    }

    pub async fn persist_restore_cloud_backup_state(
        &mut self,
        generation: GenerationToken,
        state: PersistedCloudBackupState,
        context: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = Database::global()
            .cloud_backup_state
            .set(&state)
            .map_err(|error| CloudBackupError::Internal(format!("{context}: {error}")));
        if result.is_ok() {
            manager.set_status(RustCloudBackupManager::runtime_status_for(&state));
            manager.refresh_persisted_flags();
        }

        Produces::ok(result)
    }

    pub async fn save_restore_keychain_state(
        &mut self,
        generation: GenerationToken,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = save_restore_keychain_entries(master_key, passkey, namespace_id);
        Produces::ok(result)
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
}

fn save_restore_keychain_entries(
    master_key: cove_cspp::master_key::MasterKey,
    passkey: Option<RestoredPasskeyMaterial>,
    namespace_id: String,
) -> Result<(), CloudBackupError> {
    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
    let cspp = cove_cspp::Cspp::new(keychain.clone());

    if let Some(passkey) = passkey {
        cloud_keychain
            .save_passkey_and_namespace(&passkey.credential_id, passkey.prf_salt, &namespace_id)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?
    } else {
        cloud_keychain
            .save_namespace_id(&namespace_id)
            .map_err_prefix("save namespace_id", CloudBackupError::Internal)?
    };

    if let Err(error) = cspp.save_master_key(&master_key) {
        if let Err(rollback) = cloud_keychain.clear_local_state() {
            return Err(CloudBackupError::Internal(format!(
                "save master key: {error}; rollback failed: {rollback}"
            )));
        }

        return Err(CloudBackupError::Internal(format!("save master key: {error}")));
    }

    Ok(())
}

impl CloudBackupRuntimeActor {
    pub async fn start_master_key_upload_confirmation_grace(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        let generation = self.master_key_upload_grace_generations.advance();
        self.master_key_upload_grace =
            Some(MasterKeyUploadGrace { namespace_id: namespace_id.clone(), generation });

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        cove_tokio::task::spawn(async move {
            tokio::time::sleep(MAX_PENDING_UPLOAD_VERIFICATION_DELAY).await;
            send!(addr.expire_master_key_upload_confirmation_grace(namespace_id, generation));
        });

        self.request_sync_health_refresh().await
    }

    pub async fn expire_master_key_upload_confirmation_grace(
        &mut self,
        namespace_id: String,
        generation: GenerationToken,
    ) -> ActorResult<()> {
        if !self.master_key_upload_grace.as_ref().is_some_and(|grace| {
            grace.namespace_id == namespace_id
                && grace.generation == generation
                && self.master_key_upload_grace_generations.is_current(generation)
        }) {
            return Produces::ok(());
        }

        self.master_key_upload_grace = None;
        self.request_sync_health_refresh().await
    }

    pub async fn request_sync_health_refresh(&mut self) -> ActorResult<()> {
        match self.sync_health_refresh_state {
            SyncHealthRefreshState::Idle => {}
            SyncHealthRefreshState::Running => {
                self.sync_health_refresh_state = SyncHealthRefreshState::RunningQueued;
                self.sync_health_refresh_generations.invalidate();
                return Produces::ok(());
            }
            SyncHealthRefreshState::RunningQueued => {
                self.sync_health_refresh_generations.invalidate();
                return Produces::ok(());
            }
        }

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(_) = self.manager() else { return Produces::ok(()) };
        self.spawn_sync_health_refresh_task(addr);
        Produces::ok(())
    }

    pub async fn complete_sync_health_refresh(
        &mut self,
        generation: GenerationToken,
        sync_health: CloudSyncHealth,
    ) -> ActorResult<()> {
        let rerun_queued =
            matches!(self.sync_health_refresh_state, SyncHealthRefreshState::RunningQueued);
        let is_current = self.sync_health_refresh_generations.is_current(generation);
        self.sync_health_refresh_state = SyncHealthRefreshState::Idle;

        if is_current && let Some(manager) = self.manager() {
            manager.set_sync_health(sync_health);
        }

        if !rerun_queued {
            return Produces::ok(());
        }

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(_) = self.manager() else { return Produces::ok(()) };
        self.spawn_sync_health_refresh_task(addr);
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
        if immediate {
            let Some(addr) = self.addr() else { return Produces::ok(()) };
            send!(addr.run_wallet_upload(wallet_id));
            return Produces::ok(());
        }

        self.schedule_wallet_upload_after(wallet_id, super::LIVE_UPLOAD_DEBOUNCE);
        Produces::ok(())
    }

    pub async fn run_wallet_upload(&mut self, wallet_id: WalletId) -> ActorResult<()> {
        if !self.active_wallet_uploads.insert(wallet_id.clone()) {
            return Produces::ok(());
        }

        let Some(addr) = self.addr() else {
            self.active_wallet_uploads.remove(&wallet_id);
            return Produces::ok(());
        };
        let Some(manager) = self.manager() else {
            self.active_wallet_uploads.remove(&wallet_id);
            return Produces::ok(());
        };

        cove_tokio::task::spawn(async move {
            let upload_result = manager.do_upload_wallet_if_dirty(&wallet_id).await;
            let deferred = matches!(upload_result, Err(super::CloudBackupError::Deferred(_)));
            let authorization_required = upload_result.as_ref().err().is_some_and(|error| {
                matches!(
                    manager.cloud_backup_issue(error),
                    super::CloudStorageIssue::AuthorizationRequired
                )
            });
            let error_message = upload_result.as_ref().err().map(ToString::to_string);
            send!(addr.complete_wallet_upload(
                wallet_id,
                upload_result.is_ok(),
                error_message,
                deferred,
                authorization_required
            ));
        });

        Produces::ok(())
    }

    pub async fn complete_wallet_upload(
        &mut self,
        wallet_id: WalletId,
        succeeded: bool,
        error_message: Option<String>,
        deferred: bool,
        authorization_required: bool,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.finish_wallet_upload(
            &manager,
            wallet_id,
            succeeded,
            error_message,
            deferred,
            authorization_required,
        );
        Produces::ok(())
    }

    #[cfg(test)]
    pub async fn run_wallet_upload_inline_for_test(
        &mut self,
        wallet_id: WalletId,
    ) -> ActorResult<()> {
        if !self.active_wallet_uploads.insert(wallet_id.clone()) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_wallet_uploads.remove(&wallet_id);
            return Produces::ok(());
        };

        let upload_result = manager.do_upload_wallet_if_dirty(&wallet_id).await;
        let deferred = matches!(upload_result, Err(super::CloudBackupError::Deferred(_)));
        let authorization_required = upload_result.as_ref().err().is_some_and(|error| {
            matches!(
                manager.cloud_backup_issue(error),
                super::CloudStorageIssue::AuthorizationRequired
            )
        });
        let error_message = upload_result.err().map(|error| error.to_string());
        self.finish_wallet_upload(
            &manager,
            wallet_id,
            error_message.is_none(),
            error_message,
            deferred,
            authorization_required,
        );
        Produces::ok(())
    }

    fn finish_wallet_upload(
        &mut self,
        manager: &RustCloudBackupManager,
        wallet_id: WalletId,
        succeeded: bool,
        error_message: Option<String>,
        deferred: bool,
        authorization_required: bool,
    ) {
        self.active_wallet_uploads.remove(&wallet_id);

        if let Some(error_message) = error_message {
            if deferred {
                info!("Cloud backup upload deferred for wallet_id={wallet_id}: {error_message}");
                manager.set_pending_upload_verification(
                    manager.has_pending_cloud_upload_verification(),
                );
                let delay = self.next_wallet_upload_retry_delay(&wallet_id);
                self.schedule_wallet_upload_after(wallet_id, delay);
                return;
            }

            if authorization_required {
                warn!(
                    "Cloud backup upload paused until authorization is restored for wallet_id={wallet_id}: {error_message}"
                );
                self.reset_wallet_upload_retry_count(&wallet_id);
                manager.set_sync_error(Some(error_message));
                manager.refresh_sync_health();
                return;
            }

            error!("Cloud backup upload failed for wallet_id={wallet_id}: {error_message}");
            manager.set_sync_error(Some(error_message));
        } else if succeeded {
            self.reset_wallet_upload_retry_count(&wallet_id);
            manager.clear_sync_error_if_no_failed_wallet_uploads();
        }

        manager.refresh_sync_health();
        self.schedule_wallet_upload_follow_up(wallet_id);
    }

    pub async fn resume_wallet_uploads_from_persisted_state(&mut self) -> ActorResult<()> {
        let states = match crate::database::Database::global().cloud_blob_sync_states.list() {
            Ok(states) => states,
            Err(error) => {
                error!("Failed to load cloud blob sync states on startup: {error}");
                return Produces::ok(());
            }
        };

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let manager = self.manager();

        for sync_state in states {
            let Some(wallet_id) = sync_state.wallet_id.clone() else {
                continue;
            };

            match &sync_state.state {
                PersistedCloudBlobState::Dirty(_) => {
                    send!(addr.schedule_wallet_upload(wallet_id, true));
                }
                PersistedCloudBlobState::Failed(failed_state)
                    if should_retry_failed_blob(failed_state) =>
                {
                    if is_authorization_failed_blob(failed_state)
                        && let Some(manager) = &manager
                    {
                        manager.set_sync_error(Some(failed_state.error.clone()));
                        manager.refresh_sync_health();
                    }
                    send!(addr.schedule_wallet_upload(wallet_id, true));
                }
                PersistedCloudBlobState::Uploading(_) => {
                    let Some(manager) = &manager else { continue };
                    if !manager.downgrade_interrupted_upload_to_dirty(&sync_state) {
                        continue;
                    }
                    send!(addr.schedule_wallet_upload(wallet_id, true));
                }
                PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_)
                | PersistedCloudBlobState::Failed(_) => {}
            }
        }

        Produces::ok(())
    }

    pub async fn ensure_pending_upload_verification_loop(&mut self) -> ActorResult<()> {
        if self.pending_upload_verifier_running {
            return Produces::ok(());
        }

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.spawn_pending_upload_verification_loop_task(addr, manager);

        Produces::ok(())
    }

    pub async fn wake_pending_upload_verifier(&mut self) -> ActorResult<()> {
        if self.pending_upload_verifier_running {
            self.pending_upload_verifier_wakeup.notify_one();
        }

        Produces::ok(())
    }

    pub async fn pending_upload_verifier_finished(
        &mut self,
        blocked_on_authorization: bool,
    ) -> ActorResult<()> {
        self.pending_upload_verifier_running = false;
        self.pending_upload_verifier_blocked_on_authorization = blocked_on_authorization;

        if blocked_on_authorization {
            info!("Pending upload verification: paused until cloud authorization is restored");
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else { return Produces::ok(()) };
        if manager.has_pending_cloud_upload_verification() {
            let Some(addr) = self.addr() else { return Produces::ok(()) };
            self.spawn_pending_upload_verification_loop_task(addr, manager);
            return Produces::ok(());
        }

        info!("Pending upload verification: idle");
        Produces::ok(())
    }

    #[cfg(test)]
    pub async fn new_restore_operation(&mut self) -> ActorResult<RestoreOperation> {
        let Some(addr) = self.addr() else {
            return Produces::ok(RestoreOperation::new(
                self.restore_operations.clone(),
                self.addr.upgrade(),
            ));
        };
        Produces::ok(RestoreOperation::new(self.restore_operations.clone(), addr))
    }

    #[cfg(test)]
    pub async fn invalidate_restore_operation(&mut self) -> ActorResult<()> {
        self.restore_operations.invalidate();
        Produces::ok(())
    }

    pub async fn start_restore_from_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let status = manager.state.read().status.clone();
        if matches!(status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring) {
            warn!("restore_from_cloud_backup called while {status:?}, ignoring");
            return Produces::ok(());
        }

        let Some(addr) = self.addr() else { return Produces::ok(()) };
        let operation = RestoreOperation::new(self.restore_operations.clone(), addr);
        cove_tokio::task::spawn(async move {
            info!("restore_from_cloud_backup: task started");
            match manager.do_restore_from_cloud_backup(&operation).await {
                Ok(()) => {}
                Err(CloudBackupError::Cancelled) => {
                    info!("restore_from_cloud_backup: task cancelled");
                }
                Err(error) => {
                    error!("restore_from_cloud_backup failed: {error}");
                    manager.finish_background_operation_error(&error);
                }
            }
        });

        Produces::ok(())
    }

    pub async fn cancel_restore(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let status = manager.state.read().status.clone();
        if !matches!(status, CloudBackupStatus::Restoring) {
            return Produces::ok(());
        }

        self.restore_operations.invalidate();
        manager.set_progress(None);
        manager.set_restore_progress(None);
        manager.set_restore_report(None);
        manager.set_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));
        info!("restore_from_cloud_backup: cancelled active restore");
        Produces::ok(())
    }

    pub async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        self.master_key_upload_grace = None;
        self.master_key_upload_grace_generations.invalidate();
        self.wallet_upload_debouncers.clear();
        self.wallet_upload_retry_counts.clear();
        self.active_wallet_uploads.clear();
        self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
        self.sync_health_refresh_generations.invalidate();
        Produces::ok(())
    }
}

struct PendingUploadRetryBackoff(backon::FibonacciBackoff);

impl PendingUploadRetryBackoff {
    fn new() -> Self {
        Self(build_pending_upload_backoff())
    }

    fn next_delay(&mut self) -> Duration {
        self.0
            .next()
            .map(|delay| delay.min(MAX_PENDING_UPLOAD_VERIFICATION_DELAY))
            .unwrap_or(MAX_PENDING_UPLOAD_VERIFICATION_DELAY)
    }

    fn reset(&mut self) {
        self.0 = build_pending_upload_backoff();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::{test_globals, test_lock};

    #[test]
    fn pending_upload_retry_backoff_caps_at_max_delay() {
        let mut backoff = PendingUploadRetryBackoff::new();

        for _ in 0..10 {
            assert!(backoff.next_delay() <= MAX_PENDING_UPLOAD_VERIFICATION_DELAY);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queued_sync_health_refresh_invalidates_in_flight_generation() {
        let mut actor = CloudBackupRuntimeActor::new(Weak::new());
        let generation = actor.sync_health_refresh_generations.advance();
        actor.sync_health_refresh_state = SyncHealthRefreshState::Running;

        actor.request_sync_health_refresh().await.expect("queue refresh");

        assert_eq!(actor.sync_health_refresh_state, SyncHealthRefreshState::RunningQueued);
        assert!(!actor.sync_health_refresh_generations.is_current(generation));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_master_key_upload_grace_generation_does_not_expire_new_grace() {
        let mut actor = CloudBackupRuntimeActor::new(Weak::new());
        let namespace_id = "namespace-id".to_string();
        let stale_generation = actor.master_key_upload_grace_generations.advance();
        let current_generation = actor.master_key_upload_grace_generations.advance();
        actor.master_key_upload_grace = Some(MasterKeyUploadGrace {
            namespace_id: namespace_id.clone(),
            generation: current_generation,
        });

        actor
            .expire_master_key_upload_confirmation_grace(namespace_id, stale_generation)
            .await
            .expect("expire stale grace");

        assert!(actor.master_key_upload_grace.is_some());
    }

    #[test]
    fn restore_keychain_save_rolls_back_metadata_when_master_key_save_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.fail_save_at(4);

        let result = save_restore_keychain_entries(
            cove_cspp::master_key::MasterKey::generate(),
            Some(RestoredPasskeyMaterial { credential_id: vec![1, 2, 3], prf_salt: [4; 32] }),
            "namespace-id".into(),
        );

        assert!(result.is_err());
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_none());
    }
}
