//! Serialized cloud write lane for Cloud Backup
//!
//! The write supervisor orders remote writes, enforces disable blockers, and
//! applies local post-write state only after the remote command succeeds. The
//! worker is the leaf actor that talks to cloud storage

mod client;
mod supervisor;
mod types;
mod worker;

pub(crate) use self::client::CloudBackupWriteClient;
pub(crate) use self::supervisor::{
    CloudBackupWriteBlocker, CloudBackupWriteCommandResult, CloudBackupWriteResultReceiver,
    CloudBackupWriteSupervisor,
};
pub(crate) use self::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};

pub(crate) type CloudBackupWriteError = crate::manager::cloud_backup_manager::CloudBackupError;
