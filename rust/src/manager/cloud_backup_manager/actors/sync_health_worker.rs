use std::mem::{Discriminant, discriminant};
use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, send};
use cove_device::cloud_storage::CloudSyncHealth;
use cove_util::{GenerationToken, GenerationTracker};
use tracing::{info, warn};

use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::manager::cloud_backup_manager::pending::MASTER_KEY_UPLOAD_CONFIRMATION_GRACE;

#[derive(Debug)]
pub(crate) struct CloudBackupSyncHealthWorker {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    master_key_upload_grace: Option<MasterKeyUploadGrace>,
    master_key_upload_grace_generations: GenerationTracker,
    sync_health_refresh_state: SyncHealthRefreshState,
    refreshes_started: u64,
    /// Kind of the last observed sync health, so the roughly once a second refresh logs only
    /// when the kind changes
    last_logged_sync_health: Option<Discriminant<CloudSyncHealth>>,
}

#[derive(Debug)]
struct MasterKeyUploadGrace {
    namespace_id: String,
    generation: GenerationToken,
}

#[async_trait::async_trait]
impl Actor for CloudBackupSyncHealthWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupSyncHealthWorker {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self {
            addr: WeakAddr::default(),
            manager,
            master_key_upload_grace: None,
            master_key_upload_grace_generations: GenerationTracker::new(),
            sync_health_refresh_state: SyncHealthRefreshState::Idle,
            refreshes_started: 0,
            last_logged_sync_health: None,
        }
    }

    fn log_sync_health_change(&mut self, sync_health: &CloudSyncHealth) {
        let kind = discriminant(sync_health);
        if self.last_logged_sync_health == Some(kind) {
            return;
        }

        self.last_logged_sync_health = Some(kind);
        match sync_health {
            CloudSyncHealth::Failed(_)
            | CloudSyncHealth::AuthorizationRequired(_)
            | CloudSyncHealth::Unavailable => warn!("Cloud sync health changed: {sync_health:?}"),

            CloudSyncHealth::Unknown
            | CloudSyncHealth::AllUploaded
            | CloudSyncHealth::Uploading
            | CloudSyncHealth::NoFiles => info!("Cloud sync health changed: {sync_health:?}"),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn spawn_refresh_task(&mut self) {
        let Some(manager) = self.manager() else {
            self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
            return;
        };

        self.refreshes_started += 1;
        let refresh = RefreshId(self.refreshes_started);
        self.sync_health_refresh_state =
            SyncHealthRefreshState::Running { refresh, on_finish: OnRefreshFinish::Observe };

        let master_key_upload_grace_namespace =
            self.master_key_upload_grace.as_ref().map(|grace| grace.namespace_id.clone());
        self.addr.send_fut_with(move |addr| async move {
            let sync_health = match master_key_upload_grace_namespace.as_deref() {
                Some(namespace) => {
                    manager.compute_sync_health_with_master_key_grace(Some(namespace)).await
                }
                None => manager.compute_sync_health().await,
            };
            send!(addr.complete_sync_health_refresh(refresh, sync_health));
        });
    }

    pub(crate) async fn start_master_key_upload_confirmation_grace(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        let generation = self.master_key_upload_grace_generations.advance();
        self.master_key_upload_grace =
            Some(MasterKeyUploadGrace { namespace_id: namespace_id.clone(), generation });

        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep(MASTER_KEY_UPLOAD_CONFIRMATION_GRACE).await;
            send!(addr.expire_master_key_upload_confirmation_grace(namespace_id, generation));
        });

        self.request_sync_health_refresh().await
    }

    pub(crate) async fn expire_master_key_upload_confirmation_grace(
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

    pub(crate) async fn request_sync_health_refresh(&mut self) -> ActorResult<()> {
        // the running refresh may have read the state before this request's change, so one more
        // runs after it instead of beside it
        if let SyncHealthRefreshState::Running { on_finish, .. } =
            &mut self.sync_health_refresh_state
        {
            *on_finish = OnRefreshFinish::Rerun;
            return Produces::ok(());
        }

        self.spawn_refresh_task();
        Produces::ok(())
    }

    async fn complete_sync_health_refresh(
        &mut self,
        refresh: RefreshId,
        sync_health: CloudSyncHealth,
    ) -> ActorResult<()> {
        // only the refresh the worker runs may change its state
        let on_finish = match self.sync_health_refresh_state {
            SyncHealthRefreshState::Running { refresh: running, on_finish }
                if running == refresh =>
            {
                on_finish
            }
            _ => return Produces::ok(()),
        };

        match on_finish {
            OnRefreshFinish::Observe => {
                self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
                if let Some(manager) = self.manager() {
                    self.log_sync_health_change(&sync_health);
                    manager.observe_sync_health(sync_health);
                }
            }
            OnRefreshFinish::Rerun => self.spawn_refresh_task(),
            OnRefreshFinish::Discard => {
                self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
            }
        }

        Produces::ok(())
    }

    pub(crate) async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.master_key_upload_grace = None;
        self.master_key_upload_grace_generations.invalidate();

        // a running refresh keeps running, so the worker stays Running until it lands and drops
        // its result, and a later request runs after it rather than beside it
        if let SyncHealthRefreshState::Running { on_finish, .. } =
            &mut self.sync_health_refresh_state
        {
            *on_finish = OnRefreshFinish::Discard;
        }

        Produces::ok(())
    }
}

/// Identity of one started sync health refresh, which its completion carries back
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct RefreshId(u64);

/// Whether a refresh is running, and which one
///
/// At most one refresh runs at a time, so `Idle` always means none is in flight
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SyncHealthRefreshState {
    Idle,
    Running { refresh: RefreshId, on_finish: OnRefreshFinish },
}

/// What the worker does when the running refresh lands
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum OnRefreshFinish {
    /// Nothing changed since it started, so its result is current
    Observe,
    /// A request arrived while it ran, so its result is dropped and a new refresh starts
    Rerun,
    /// Upload runtime state was cleared while it ran, so its result is dropped
    Discard,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::initialize_cloud_backup_manager;
    use crate::manager::cloud_backup_manager::ops::test_support::{async_test_lock, test_globals};

    impl CloudBackupSyncHealthWorker {
        /// Answers after every message sent to the worker before it
        pub(crate) async fn checkpoint_for_test(&mut self) -> ActorResult<()> {
            Produces::ok(())
        }

        /// Whether no refresh is in flight or queued
        pub(crate) async fn sync_health_refresh_idle_for_test(&mut self) -> ActorResult<bool> {
            Produces::ok(self.sync_health_refresh_state == SyncHealthRefreshState::Idle)
        }
    }

    fn running_refresh(actor: &CloudBackupSyncHealthWorker) -> (RefreshId, OnRefreshFinish) {
        match actor.sync_health_refresh_state {
            SyncHealthRefreshState::Running { refresh, on_finish } => (refresh, on_finish),
            SyncHealthRefreshState::Idle => panic!("expected a running refresh"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_request_while_a_refresh_runs_queues_one_rerun() {
        let mut actor = CloudBackupSyncHealthWorker::new(Weak::new());
        let refresh = RefreshId(1);
        actor.sync_health_refresh_state =
            SyncHealthRefreshState::Running { refresh, on_finish: OnRefreshFinish::Observe };

        actor.request_sync_health_refresh().await.expect("queue refresh");
        actor.request_sync_health_refresh().await.expect("coalesce refresh");

        assert_eq!(running_refresh(&actor), (refresh, OnRefreshFinish::Rerun));
    }

    // a clear while a refresh runs, then a request, then that refresh landing: the worker must
    // stay Running for the refresh that request started, or a reader trusting Idle misses it
    #[tokio::test(flavor = "current_thread")]
    async fn a_refresh_cleared_while_running_does_not_end_the_next_one() {
        let _guard = async_test_lock().lock().await;
        test_globals();
        let manager = initialize_cloud_backup_manager();

        // the worker is never spawned, so its refresh tasks never run and the test lands them
        let mut actor = CloudBackupSyncHealthWorker::new(Arc::downgrade(&manager));
        actor.request_sync_health_refresh().await.expect("start refresh");
        let (first, _) = running_refresh(&actor);

        actor.clear_upload_runtime_state().await.expect("clear upload runtime state");
        actor.request_sync_health_refresh().await.expect("request after the clear");
        assert_eq!(running_refresh(&actor), (first, OnRefreshFinish::Rerun));

        let stale = CloudSyncHealth::Failed("read before the clear".into());
        actor.complete_sync_health_refresh(first, stale.clone()).await.expect("land first");

        let (second, on_finish) = running_refresh(&actor);
        assert_ne!(second, first);
        assert_eq!(on_finish, OnRefreshFinish::Observe);

        // a completion of a refresh the worker no longer runs leaves the running one alone
        actor.complete_sync_health_refresh(first, stale).await.expect("land first again");
        assert_eq!(running_refresh(&actor), (second, OnRefreshFinish::Observe));

        actor.clear_upload_runtime_state().await.expect("clear before the second lands");
        actor
            .complete_sync_health_refresh(second, CloudSyncHealth::Unknown)
            .await
            .expect("land second");
        assert_eq!(actor.sync_health_refresh_state, SyncHealthRefreshState::Idle);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_master_key_upload_grace_generation_does_not_expire_new_grace() {
        let mut actor = CloudBackupSyncHealthWorker::new(Weak::new());
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
}
