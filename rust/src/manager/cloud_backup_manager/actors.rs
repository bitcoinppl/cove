//! Cloud Backup actors and child work lanes
//!
//! The supervisor actor owns exclusive operation lifecycles, while child
//! actors own narrower queued work such as restore progress, wallet uploads,
//! sync health checks, cleanup, and serialized cloud writes

pub(crate) mod cleanup;
pub(crate) mod restore;
pub(crate) mod supervisor;
mod sync_health_worker;
mod uploads;
pub(crate) mod write;

pub(crate) use self::cleanup::{CleanupExpectedWalletRecord, CleanupSourceNamespace};
pub(crate) use self::restore::CloudBackupRestoreEvent;
pub(crate) use self::supervisor::{CloudBackupOperation, CloudBackupSupervisor};
pub(crate) use self::sync_health_worker::CloudBackupSyncHealthWorker;
pub(crate) use self::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteBlocker, CloudBackupWriteClient, CloudBackupWriteCompletion,
    CloudBackupWriteResultReceiver, CloudBackupWriteSupervisor,
};
