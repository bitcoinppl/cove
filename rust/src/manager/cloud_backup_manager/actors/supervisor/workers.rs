//! The supervisor's handles to the workers it starts
//!
//! Waits go one way only: workers may wait on the supervisor, and the supervisor only tells its
//! workers what to do. A handle sends each message and never returns a future, so a supervisor
//! handler can't hold a reset or a lifecycle completion behind a worker's mailbox. A worker handles
//! its mailbox in order, so it handles a message the supervisor sent before any it sends later

use std::sync::Weak;

use act_zero::{Addr, send};
use cove_tokio::task::spawn_actor;

use super::super::cleanup::{CloudBackupCleanupJob, CloudBackupCleanupWorker};
use super::super::sync_health_worker::CloudBackupSyncHealthWorker;
use super::super::uploads::CloudBackupUploadWorker;
use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::wallet::metadata::WalletId;

/// Tells the upload worker to schedule, resume or clear wallet uploads and their verifier
#[derive(Debug)]
pub(crate) struct UploadWorker(Addr<CloudBackupUploadWorker>);

impl UploadWorker {
    pub(crate) fn spawn(manager: Weak<RustCloudBackupManager>) -> Self {
        Self(spawn_actor(CloudBackupUploadWorker::new(manager)))
    }

    pub(crate) fn schedule_wallet_upload(&self, wallet_id: WalletId, immediate: bool) {
        send!(self.0.schedule_wallet_upload(wallet_id, immediate));
    }

    pub(crate) fn resume_wallet_uploads_from_persisted_state(&self) {
        send!(self.0.resume_wallet_uploads_from_persisted_state());
    }

    pub(crate) fn ensure_pending_upload_verification_loop(&self) {
        send!(self.0.ensure_pending_upload_verification_loop());
    }

    pub(crate) fn wake_pending_upload_verifier(&self) {
        send!(self.0.wake_pending_upload_verifier());
    }

    pub(crate) fn clear_upload_runtime_state(&self) {
        send!(self.0.clear_upload_runtime_state());
    }
}

/// Tells the sync-health worker to refresh or to track a master key upload's confirmation
#[derive(Debug)]
pub(crate) struct SyncHealthWorker(Addr<CloudBackupSyncHealthWorker>);

impl SyncHealthWorker {
    pub(crate) fn spawn(manager: Weak<RustCloudBackupManager>) -> Self {
        Self(spawn_actor(CloudBackupSyncHealthWorker::new(manager)))
    }

    pub(crate) fn start_master_key_upload_confirmation_grace(&self, namespace_id: String) {
        send!(self.0.start_master_key_upload_confirmation_grace(namespace_id));
    }

    pub(crate) fn request_sync_health_refresh(&self) {
        send!(self.0.request_sync_health_refresh());
    }

    pub(crate) fn clear_upload_runtime_state(&self) {
        send!(self.0.clear_upload_runtime_state());
    }
}

/// Tells the cleanup worker to delete the sources a recovered Enable merged
#[derive(Debug)]
pub(crate) struct CleanupWorker(Addr<CloudBackupCleanupWorker>);

impl CleanupWorker {
    pub(crate) fn spawn(manager: Weak<RustCloudBackupManager>) -> Self {
        Self(spawn_actor(CloudBackupCleanupWorker::new(manager)))
    }

    pub(crate) fn enqueue_cleanup(&self, job: CloudBackupCleanupJob) {
        send!(self.0.enqueue_cleanup(job));
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use act_zero::Addr;

    use super::{CleanupWorker, SyncHealthWorker, UploadWorker};
    use crate::manager::cloud_backup_manager::actors::cleanup::CloudBackupCleanupWorker;
    use crate::manager::cloud_backup_manager::actors::sync_health_worker::CloudBackupSyncHealthWorker;
    use crate::manager::cloud_backup_manager::actors::uploads::CloudBackupUploadWorker;

    impl UploadWorker {
        /// The worker's address, for tests that wait on it directly
        pub(crate) fn addr_for_test(&self) -> &Addr<CloudBackupUploadWorker> {
            &self.0
        }
    }

    impl SyncHealthWorker {
        /// The worker's address, for tests that wait on it directly
        pub(crate) fn addr_for_test(&self) -> &Addr<CloudBackupSyncHealthWorker> {
            &self.0
        }
    }

    impl CleanupWorker {
        /// The worker's address, for tests that wait on it directly
        pub(crate) fn addr_for_test(&self) -> &Addr<CloudBackupCleanupWorker> {
            &self.0
        }
    }
}
