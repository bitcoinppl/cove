use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, send};
use cove_device::cloud_storage::CloudSyncHealth;
use cove_util::{GenerationToken, GenerationTracker};

use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::manager::cloud_backup_manager::pending::MAX_PENDING_UPLOAD_VERIFICATION_DELAY;

#[derive(Debug)]
pub(crate) struct CloudBackupSyncHealthWorker {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    master_key_upload_grace: Option<MasterKeyUploadGrace>,
    master_key_upload_grace_generations: GenerationTracker,
    sync_health_refresh_state: SyncHealthRefreshState,
    sync_health_refresh_generations: GenerationTracker,
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
            sync_health_refresh_generations: GenerationTracker::new(),
        }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn spawn_refresh_task(&mut self) {
        self.sync_health_refresh_state = SyncHealthRefreshState::Running;
        let Some(manager) = self.manager() else {
            self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
            return;
        };

        let generation = self.sync_health_refresh_generations.advance();
        self.addr.send_fut_with(move |addr| async move {
            let sync_health = manager.compute_sync_health().await;
            send!(addr.complete_sync_health_refresh(generation, sync_health));
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
            tokio::time::sleep(MAX_PENDING_UPLOAD_VERIFICATION_DELAY).await;
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

        let Some(_) = self.manager() else { return Produces::ok(()) };
        self.spawn_refresh_task();
        Produces::ok(())
    }

    pub(crate) async fn complete_sync_health_refresh(
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

        let Some(_) = self.manager() else { return Produces::ok(()) };
        self.spawn_refresh_task();
        Produces::ok(())
    }

    pub(crate) async fn clear_upload_runtime_state(&mut self) -> ActorResult<()> {
        self.master_key_upload_grace = None;
        self.master_key_upload_grace_generations.invalidate();
        self.sync_health_refresh_state = SyncHealthRefreshState::Idle;
        self.sync_health_refresh_generations.invalidate();
        Produces::ok(())
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SyncHealthRefreshState {
    Idle,
    Running,
    RunningQueued,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn queued_sync_health_refresh_invalidates_in_flight_generation() {
        let mut actor = CloudBackupSyncHealthWorker::new(Weak::new());
        let generation = actor.sync_health_refresh_generations.advance();
        actor.sync_health_refresh_state = SyncHealthRefreshState::Running;

        actor.request_sync_health_refresh().await.expect("queue refresh");

        assert_eq!(actor.sync_health_refresh_state, SyncHealthRefreshState::RunningQueued);
        assert!(!actor.sync_health_refresh_generations.is_current(generation));
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
