use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, send};
use cove_tokio::DebouncedTask;
use tokio::sync::Notify;
use tracing::{error, info, warn};

use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobFailedState, CloudStorageIssue, PersistedCloudBlobState,
};
use crate::manager::cloud_backup_manager::actors::Decision;
use crate::manager::cloud_backup_manager::admission::PauseReason;
use crate::manager::cloud_backup_manager::pending::{
    MAX_PENDING_UPLOAD_VERIFICATION_DELAY, build_pending_upload_backoff,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupError, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, PendingUploadVerificationState,
    RustCloudBackupManager, SyncState, WalletId, live_upload_retry_delay_for_attempt,
};

/// How one dirty wallet upload ended, which decides what the scheduler does next
#[derive(Debug)]
pub(crate) enum WalletUploadOutcome {
    Uploaded,
    /// The write lane paused ordinary writes: the row stays Dirty with no retry timer, and the
    /// lane wakes the scheduler when the pause ends
    Paused(PauseReason),
    /// Waiting for a connection, retried on a timer
    Deferred(String),
    AuthorizationRequired(String),
    Failed(String),
}

impl From<Result<(), CloudBackupError>> for WalletUploadOutcome {
    fn from(result: Result<(), CloudBackupError>) -> Self {
        let error = match result {
            Ok(()) => return Self::Uploaded,
            Err(CloudBackupError::WritesPaused(reason)) => return Self::Paused(reason),
            Err(error) => error,
        };

        let message = error.to_string();
        if matches!(error, CloudBackupError::Deferred(_)) {
            return Self::Deferred(message);
        }

        match CloudStorageIssue::from(&error) {
            CloudStorageIssue::AuthorizationRequired => Self::AuthorizationRequired(message),
            _ => Self::Failed(message),
        }
    }
}

/// Why the pending upload verifier loop stopped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifierStop {
    /// Nothing is waiting for confirmation, or the manager is gone
    Idle,
    BlockedOnAuthorization,
    /// Ordinary writes are paused; the write lane restarts the loop when they run again
    WritesPaused,
}

#[derive(Debug)]
pub(crate) struct CloudBackupUploadWorker {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    pending_upload_verifier_running: bool,
    pending_upload_verifier_blocked_on_authorization: bool,
    pending_upload_verifier_wakeup: Arc<Notify>,
    wallet_upload_debouncers: HashMap<WalletId, DebouncedTask<()>>,
    wallet_upload_retry_counts: HashMap<WalletId, u32>,
    /// Uploads in flight, with whether a schedule arrived for the same wallet meanwhile
    ///
    /// A schedule for a wallet already uploading is coalesced into it, so a wake that arrives
    /// while that upload is being paused still runs the wallet once more
    active_wallet_uploads: HashMap<WalletId, bool>,
}

#[async_trait::async_trait]
impl Actor for CloudBackupUploadWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupUploadWorker {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self {
            addr: WeakAddr::default(),
            manager,
            pending_upload_verifier_running: false,
            pending_upload_verifier_blocked_on_authorization: false,
            pending_upload_verifier_wakeup: Arc::new(Notify::new()),
            wallet_upload_debouncers: HashMap::new(),
            wallet_upload_retry_counts: HashMap::new(),
            active_wallet_uploads: HashMap::new(),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Addr<Self> {
        self.addr.upgrade()
    }

    fn spawn_pending_upload_verification_loop_task(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
    ) {
        self.pending_upload_verifier_running = true;
        self.pending_upload_verifier_blocked_on_authorization = false;
        let manager = Arc::downgrade(&manager);
        let wakeup = Arc::clone(&self.pending_upload_verifier_wakeup);
        self.addr.send_fut_with(move |addr| async move {
            info!("Pending upload verification: started");
            let mut backoff = PendingUploadRetryBackoff::new();

            let stop = loop {
                let verification = {
                    let Some(manager) = manager.upgrade() else { break VerifierStop::Idle };

                    // the verifier records confirmations, not lane commands, so it asks the lane
                    // whether ordinary writes run before each pass
                    match manager.write_ticket().await {
                        Ok(ticket) if ticket.ordinary == Decision::Run => {}
                        Ok(_) => break VerifierStop::WritesPaused,
                        Err(error) => {
                            warn!("Pending upload verification: write lane unavailable: {error}");
                            break VerifierStop::Idle;
                        }
                    }

                    manager.verify_pending_uploads_once().await
                };
                match verification {
                    PendingUploadVerificationState::Idle => break VerifierStop::Idle,
                    PendingUploadVerificationState::BlockedOnAuthorization => {
                        break VerifierStop::BlockedOnAuthorization;
                    }
                    PendingUploadVerificationState::Confirming => {}
                }

                let delay = backoff.next_delay();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wakeup.notified() => backoff.reset(),
                }
            };

            send!(addr.pending_upload_verifier_finished(stop));
        });
    }

    fn schedule_wallet_upload_after(&mut self, wallet_id: WalletId, delay: Duration) {
        let task = DebouncedTask::new("cloud_wallet_backup_upload", delay);
        self.wallet_upload_debouncers.insert(wallet_id.clone(), task.clone());

        let addr = self.addr();
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
        let sync_state =
            match crate::database::Database::global().cloud_blob_sync_states.get(&record_id) {
                Ok(sync_state) => sync_state,
                Err(error) => {
                    error!("Failed to read wallet upload follow-up state: {error}");
                    return;
                }
            };

        let Some(sync_state) = sync_state else {
            return;
        };

        if sync_state.is_dirty() {
            self.reset_wallet_upload_retry_count(&wallet_id);
            {
                let addr = self.addr();
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

        manager.refresh_pending_upload_verification_state();
    }

    pub(crate) async fn schedule_wallet_upload(
        &mut self,
        wallet_id: WalletId,
        immediate: bool,
    ) -> ActorResult<()> {
        if immediate {
            let addr = self.addr();
            send!(addr.run_wallet_upload(wallet_id));
            return Produces::ok(());
        }

        self.schedule_wallet_upload_after(
            wallet_id,
            crate::manager::cloud_backup_manager::LIVE_UPLOAD_DEBOUNCE,
        );
        Produces::ok(())
    }

    /// Starts one upload for `wallet_id`, or coalesces into the one already running
    fn begin_wallet_upload(&mut self, wallet_id: &WalletId) -> bool {
        if let Some(rerun) = self.active_wallet_uploads.get_mut(wallet_id) {
            *rerun = true;
            return false;
        }

        self.active_wallet_uploads.insert(wallet_id.clone(), false);
        true
    }

    pub(crate) async fn run_wallet_upload(&mut self, wallet_id: WalletId) -> ActorResult<()> {
        if !self.begin_wallet_upload(&wallet_id) {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.active_wallet_uploads.remove(&wallet_id);
            return Produces::ok(());
        };

        // the write lane decides whether this upload runs, so the scheduler never checks a pause
        self.addr.send_fut_with(move |addr| async move {
            let outcome =
                WalletUploadOutcome::from(manager.do_upload_wallet_if_dirty(&wallet_id).await);
            send!(addr.complete_wallet_upload(wallet_id, outcome));
        });

        Produces::ok(())
    }

    pub(crate) async fn complete_wallet_upload(
        &mut self,
        wallet_id: WalletId,
        outcome: WalletUploadOutcome,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.finish_wallet_upload(&manager, wallet_id, outcome);
        Produces::ok(())
    }

    fn finish_wallet_upload(
        &mut self,
        manager: &RustCloudBackupManager,
        wallet_id: WalletId,
        outcome: WalletUploadOutcome,
    ) {
        let rerun = self.active_wallet_uploads.remove(&wallet_id).unwrap_or(false);

        match outcome {
            WalletUploadOutcome::Uploaded => self.reset_wallet_upload_retry_count(&wallet_id),

            // no timer and no follow-up: the write lane wakes the scheduler when the pause ends,
            // and a wake that arrived during this upload was coalesced into it
            WalletUploadOutcome::Paused(reason) => {
                info!("Cloud backup upload paused for wallet_id={wallet_id}: {reason:?}");
                if rerun {
                    let addr = self.addr();
                    send!(addr.run_wallet_upload(wallet_id));
                }
                return;
            }

            WalletUploadOutcome::Deferred(message) => {
                info!("Cloud backup upload deferred for wallet_id={wallet_id}: {message}");
                manager.refresh_pending_upload_verification_state();
                let delay = self.next_wallet_upload_retry_delay(&wallet_id);
                self.schedule_wallet_upload_after(wallet_id, delay);
                return;
            }

            WalletUploadOutcome::AuthorizationRequired(message) => {
                warn!("Cloud backup upload paused until authorization is restored: {message}");
                self.reset_wallet_upload_retry_count(&wallet_id);
                manager.refresh_sync_health();
                return;
            }

            WalletUploadOutcome::Failed(message) => {
                error!("Cloud backup upload failed: {message}");
            }
        }

        manager.refresh_sync_health();
        self.schedule_wallet_upload_follow_up(wallet_id);
    }

    pub(crate) async fn resume_wallet_uploads_from_persisted_state(&mut self) -> ActorResult<()> {
        let states = match crate::database::Database::global().cloud_blob_sync_states.list() {
            Ok(states) => states,
            Err(error) => {
                let message = format!("failed to load cloud blob sync states on startup: {error}");
                error!("{message}");
                if let Some(manager) = self.manager() {
                    manager.apply_sync_state(SyncState::Failed(
                        GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into(),
                    ));
                }
                return Err(CloudBackupError::Internal(message.into()).into());
            }
        };

        let addr = self.addr();
        let manager = self.manager();

        for sync_state in states {
            let wallet_id = match sync_state.record_key() {
                CloudBackupRecordKey::Wallet(wallet_id, _) => wallet_id.clone(),
                CloudBackupRecordKey::MasterKeyWrapper | CloudBackupRecordKey::Corrupted(_) => {
                    continue;
                }
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

    pub(crate) async fn ensure_pending_upload_verification_loop(&mut self) -> ActorResult<()> {
        if self.pending_upload_verifier_running {
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else { return Produces::ok(()) };

        // the loop asks the write lane before each pass, so a paused lane stops it right away
        self.spawn_pending_upload_verification_loop_task(manager);

        Produces::ok(())
    }

    pub(crate) async fn wake_pending_upload_verifier(&mut self) -> ActorResult<()> {
        if self.pending_upload_verifier_running {
            self.pending_upload_verifier_wakeup.notify_one();
        }

        Produces::ok(())
    }

    pub(crate) async fn pending_upload_verifier_finished(
        &mut self,
        stop: VerifierStop,
    ) -> ActorResult<()> {
        self.pending_upload_verifier_running = false;
        self.pending_upload_verifier_blocked_on_authorization =
            stop == VerifierStop::BlockedOnAuthorization;

        match stop {
            VerifierStop::BlockedOnAuthorization => {
                info!("Pending upload verification: paused until cloud authorization is restored");
                return Produces::ok(());
            }
            VerifierStop::WritesPaused => {
                info!("Pending upload verification: paused until cloud backup writes run again");
                return Produces::ok(());
            }
            VerifierStop::Idle => {}
        }

        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if manager.has_pending_cloud_upload_verification() {
            self.spawn_pending_upload_verification_loop_task(manager);
            return Produces::ok(());
        }

        info!("Pending upload verification: idle");
        Produces::ok(())
    }

    pub(crate) async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.wallet_upload_debouncers.clear();
        self.wallet_upload_retry_counts.clear();
        self.active_wallet_uploads.clear();
        Produces::ok(())
    }
}

fn is_authorization_failed_blob(failed_state: &CloudBlobFailedState) -> bool {
    failed_state.issue == Some(CloudStorageIssue::AuthorizationRequired)
}

fn should_retry_failed_blob(failed_state: &CloudBlobFailedState) -> bool {
    failed_state.retryable || is_authorization_failed_blob(failed_state)
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
    use crate::database::Database;
    use crate::database::cloud_backup::{
        CloudBlobUploadedPendingConfirmationState, PersistedBackupSyncState,
        PersistedBackupVerificationState, PersistedCloudBackupState, PersistedCloudBlobSyncState,
        PersistedConfiguredCloudBackup, PersistedDisablingCloudBackup, PersistedPasskeyState,
    };
    use crate::manager::cloud_backup_manager::CloudBackupKeychain;
    use crate::manager::cloud_backup_manager::ops::test_support::{
        async_test_lock, configure_enabled_cloud_backup, ensure_cloud_backup_test_tokio_runtime,
        test_globals,
    };

    impl CloudBackupUploadWorker {
        pub(crate) async fn run_wallet_upload_inline_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<()> {
            if !self.begin_wallet_upload(&wallet_id) {
                return Produces::ok(());
            }

            let Some(manager) = self.manager() else {
                self.active_wallet_uploads.remove(&wallet_id);
                return Produces::ok(());
            };

            let outcome =
                WalletUploadOutcome::from(manager.do_upload_wallet_if_dirty(&wallet_id).await);
            self.finish_wallet_upload(&manager, wallet_id, outcome);
            Produces::ok(())
        }

        /// How many backoff retries the worker armed for the wallet since its last success
        pub(crate) async fn wallet_upload_retry_count_for_test(
            &mut self,
            wallet_id: WalletId,
        ) -> ActorResult<u32> {
            Produces::ok(self.wallet_upload_retry_counts.get(&wallet_id).copied().unwrap_or(0))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verifier_paused_by_the_lane_waits_for_its_wake() {
        let _guard = async_test_lock().lock().await;
        ensure_cloud_backup_test_tokio_runtime();
        let globals = test_globals();
        let manager = crate::manager::cloud_backup_manager::initialize_cloud_backup_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
        let configured = PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification: PersistedBackupVerificationState::NotVerified {
                requested_at: None,
                dismissed_at: None,
            },
            sync: PersistedBackupSyncState { last_sync: None, wallet_count: None },
            pending_verification_completion: None,
            pending_restore_all: None,
            heal_notice: None,
        };
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::disabling_transition(PersistedDisablingCloudBackup {
                previous_configured: configured,
                namespace_id: namespace.clone(),
                disable_generation: 7,
                started_at: 1,
                delete_started_at: None,
                last_error: None,
                retry_after: None,
            }))
            .unwrap();
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState::master_key_wrapper(
                namespace,
                PersistedCloudBlobState::UploadedPendingConfirmation(
                    CloudBlobUploadedPendingConfirmationState {
                        revision_hash: "revision".into(),
                        uploaded_at: 1,
                        attempt_count: 0,
                        last_checked_at: None,
                    },
                ),
            ))
            .unwrap();

        let mut worker = CloudBackupUploadWorker::new(Arc::downgrade(&manager));
        worker.pending_upload_verifier_running = true;

        // pending evidence is still stored, but a loop the lane paused is restarted by the lane's
        // wake, not respawned here
        worker.pending_upload_verifier_finished(VerifierStop::WritesPaused).await.unwrap();

        assert!(!worker.pending_upload_verifier_running);
    }
}
