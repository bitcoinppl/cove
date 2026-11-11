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
#[cfg(test)]
pub(crate) use self::supervisor::dispatch_test_support;
pub(crate) use self::supervisor::{CloudBackupSupervisor, PermitTicket};
#[cfg(test)]
pub(crate) use self::uploads::UploadFinish;
pub(crate) use self::write::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteClient, CloudBackupWriteCompletion, CloudBackupWriteSupervisor,
    ConfigGeneration, Decision, OwnedKeychainWrite, Relist, RowDeleteReport, WriteTicket,
};
