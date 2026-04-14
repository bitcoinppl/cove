use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, send};
use cove_tokio::DebouncedTask;
use tokio::sync::Notify;
use tracing::{error, info, warn};

use super::pending::{MAX_PENDING_UPLOAD_VERIFICATION_DELAY, build_pending_upload_backoff};
use super::{
    CloudBackupError, CloudBackupStatus, PendingEnableSession, PendingVerificationCompletion,
    RustCloudBackupManager, WalletId, live_upload_retry_delay_for_attempt,
};
use crate::database::cloud_backup::PersistedCloudBlobState;
use crate::manager::cloud_backup_detail_manager::RecoveryAction;

#[derive(Debug, Clone)]
pub(crate) enum CloudBackupOperation {
    Enable,
    EnableForceNew,
    EnableNoDiscovery,
    Verification { force_discoverable: bool },
    Recovery { action: RecoveryAction },
    RepairPasskey { no_discovery: bool },
    Sync,
    FetchCloudOnly,
    RestoreCloudWallet,
    DeleteCloudWallet,
    RefreshDetail,
}

#[derive(Clone, Debug)]
pub(crate) struct RestoreOperation {
    coordinator: RestoreOperationCoordinator,
    id: u64,
}

impl RestoreOperation {
    fn new(coordinator: RestoreOperationCoordinator) -> Self {
        let id = coordinator.next_operation_id();
        Self { coordinator, id }
    }

    pub(crate) fn ensure_current(&self) -> Result<(), CloudBackupError> {
        self.coordinator.ensure_current(self.id)
    }

    pub(crate) fn run<T>(&self, update: impl FnOnce() -> T) -> Result<T, CloudBackupError> {
        self.coordinator.with_current(self.id, update)
    }

    pub(crate) fn run_result<T>(
        &self,
        update: impl FnOnce() -> Result<T, CloudBackupError>,
    ) -> Result<T, CloudBackupError> {
        self.coordinator.with_current_result(self.id, update)
    }
}

#[derive(Clone, Debug, Default)]
struct RestoreOperationCoordinator(Arc<parking_lot::Mutex<u64>>);

impl RestoreOperationCoordinator {
    fn next_operation_id(&self) -> u64 {
        let mut operation_id = self.0.lock();
        *operation_id += 1;
        *operation_id
    }

    fn invalidate(&self) {
        let mut operation_id = self.0.lock();
        *operation_id += 1;
    }

    fn ensure_current(&self, operation_id: u64) -> Result<(), CloudBackupError> {
        let current_operation = *self.0.lock();
        if current_operation == operation_id { Ok(()) } else { Err(CloudBackupError::Cancelled) }
    }

    fn with_current<T>(
        &self,
        operation_id: u64,
        update: impl FnOnce() -> T,
    ) -> Result<T, CloudBackupError> {
        let current_operation = self.0.lock();
        if *current_operation != operation_id {
            return Err(CloudBackupError::Cancelled);
        }

        Ok(update())
    }

    fn with_current_result<T>(
        &self,
        operation_id: u64,
        update: impl FnOnce() -> Result<T, CloudBackupError>,
    ) -> Result<T, CloudBackupError> {
        let current_operation = self.0.lock();
        if *current_operation != operation_id {
            return Err(CloudBackupError::Cancelled);
        }

        update()
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupRuntimeActor {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    pending_enable_session: Option<PendingEnableSession>,
    pending_verification_completion: Option<PendingVerificationCompletion>,
    pending_upload_verifier_running: bool,
    pending_upload_verifier_wakeup: Arc<Notify>,
    wallet_upload_debouncers: HashMap<WalletId, DebouncedTask<()>>,
    wallet_upload_retry_counts: HashMap<WalletId, u32>,
    active_wallet_uploads: HashSet<WalletId>,
    restore_operations: RestoreOperationCoordinator,
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
            pending_upload_verifier_running: false,
            pending_upload_verifier_wakeup: Arc::new(Notify::new()),
            wallet_upload_debouncers: HashMap::new(),
            wallet_upload_retry_counts: HashMap::new(),
            active_wallet_uploads: HashSet::new(),
            restore_operations: RestoreOperationCoordinator::default(),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Option<Addr<Self>> {
        Some(self.addr.upgrade())
    }

    fn spawn_pending_upload_verification_loop_task(
        &mut self,
        addr: Addr<Self>,
        manager: Arc<RustCloudBackupManager>,
    ) {
        self.pending_upload_verifier_running = true;
        let wakeup = Arc::clone(&self.pending_upload_verifier_wakeup);
        cove_tokio::task::spawn(async move {
            info!("Pending upload verification: started");
            let mut backoff = PendingUploadRetryBackoff::new();

            loop {
                let manager_for_pass = Arc::clone(&manager);
                let has_pending = cove_tokio::task::spawn_blocking(move || {
                    manager_for_pass.verify_pending_uploads_once()
                })
                .await
                .unwrap_or_else(|error| {
                    error!("Pending upload verification task failed: {error}");
                    true
                });

                if !has_pending {
                    break;
                }

                let delay = backoff.next_delay();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wakeup.notified() => backoff.reset(),
                }
            }

            send!(addr.pending_upload_verifier_finished());
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
            PersistedCloudBlobState::Failed(failed_state) if failed_state.retryable => {
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
                cove_tokio::task::spawn_blocking(move || {
                    if let Err(error) = manager.do_enable_cloud_backup() {
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
                cove_tokio::task::spawn_blocking(move || {
                    if let Err(error) = manager.do_enable_cloud_backup_force_new() {
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
                cove_tokio::task::spawn_blocking(move || {
                    if let Err(error) = manager.do_enable_cloud_backup_no_discovery() {
                        error!("enable_cloud_backup_no_discovery failed: {error}");
                        manager.finish_background_operation_error(&error);
                    }
                });
            }
            CloudBackupOperation::Verification { force_discoverable } => {
                cove_tokio::task::spawn_blocking(move || {
                    manager.handle_start_verification(force_discoverable);
                });
            }
            CloudBackupOperation::Recovery { action } => {
                cove_tokio::task::spawn_blocking(move || manager.handle_recovery(action));
            }
            CloudBackupOperation::RepairPasskey { no_discovery } => {
                cove_tokio::task::spawn_blocking(move || {
                    manager.handle_repair_passkey(no_discovery)
                });
            }
            CloudBackupOperation::Sync => {
                cove_tokio::task::spawn_blocking(move || manager.handle_sync());
            }
            CloudBackupOperation::FetchCloudOnly => {
                cove_tokio::task::spawn_blocking(move || manager.handle_fetch_cloud_only());
            }
            CloudBackupOperation::RestoreCloudWallet => {
                let Some(record_id) = record_id else { return };
                cove_tokio::task::spawn_blocking(move || {
                    manager.handle_restore_cloud_wallet(&record_id)
                });
            }
            CloudBackupOperation::DeleteCloudWallet => {
                let Some(record_id) = record_id else { return };
                cove_tokio::task::spawn_blocking(move || {
                    manager.handle_delete_cloud_wallet(&record_id)
                });
            }
            CloudBackupOperation::RefreshDetail => {
                cove_tokio::task::spawn_blocking(move || manager.handle_refresh_detail());
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

    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn discard_pending_enable_session(&mut self) -> ActorResult<()> {
        if self.pending_enable_session.take().is_some() {
            cove_cspp::Cspp::new(cove_device::keychain::Keychain::global().clone())
                .delete_master_key();
        }

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

        cove_tokio::task::spawn_blocking(move || {
            let upload_result = manager.do_upload_wallet_if_dirty(&wallet_id);
            let deferred = matches!(upload_result, Err(super::CloudBackupError::Deferred(_)));
            let error_message = upload_result.as_ref().err().map(ToString::to_string);
            send!(addr.complete_wallet_upload(
                wallet_id,
                upload_result.is_ok(),
                error_message,
                deferred
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
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.finish_wallet_upload(&manager, wallet_id, succeeded, error_message, deferred);
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

        let upload_result = manager.do_upload_wallet_if_dirty(&wallet_id);
        let deferred = matches!(upload_result, Err(super::CloudBackupError::Deferred(_)));
        let error_message = upload_result.err().map(|error| error.to_string());
        self.finish_wallet_upload(
            &manager,
            wallet_id,
            error_message.is_none(),
            error_message,
            deferred,
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

            error!("Cloud backup upload failed for wallet_id={wallet_id}: {error_message}");
            manager.set_sync_error(Some(error_message));
        } else if succeeded {
            self.reset_wallet_upload_retry_count(&wallet_id);
            manager.clear_sync_error_if_no_failed_wallet_uploads();
        }

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

        for sync_state in states {
            let Some(wallet_id) = sync_state.wallet_id.clone() else {
                continue;
            };

            match &sync_state.state {
                PersistedCloudBlobState::Dirty(_) => {
                    send!(addr.schedule_wallet_upload(wallet_id, true));
                }
                PersistedCloudBlobState::Failed(failed_state) if failed_state.retryable => {
                    send!(addr.schedule_wallet_upload(wallet_id, true));
                }
                PersistedCloudBlobState::Uploading(_) => {
                    let Some(manager) = self.manager() else { continue };
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

    pub async fn pending_upload_verifier_finished(&mut self) -> ActorResult<()> {
        self.pending_upload_verifier_running = false;

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
        Produces::ok(RestoreOperation::new(self.restore_operations.clone()))
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

        let operation = RestoreOperation::new(self.restore_operations.clone());
        cove_tokio::task::spawn_blocking(move || {
            info!("restore_from_cloud_backup: task started");
            match manager.do_restore_from_cloud_backup(&operation) {
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
        self.wallet_upload_debouncers.clear();
        self.wallet_upload_retry_counts.clear();
        self.active_wallet_uploads.clear();
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

    #[test]
    fn pending_upload_retry_backoff_caps_at_max_delay() {
        let mut backoff = PendingUploadRetryBackoff::new();

        for _ in 0..10 {
            assert!(backoff.next_delay() <= MAX_PENDING_UPLOAD_VERIFICATION_DELAY);
        }
    }
}
