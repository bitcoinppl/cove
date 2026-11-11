//! Serialized cloud write lane for Cloud Backup
//!
//! The write supervisor orders remote writes, decides each command against the write policy the
//! lifecycle supervisor derives from its runtime, and applies local post-write state only after
//! the remote command succeeds. The worker is the leaf actor that talks to cloud storage

mod client;
mod policy;
mod supervisor;
mod types;
mod worker;

pub(crate) use self::client::{CloudBackupWriteClient, WriteOwner};
#[cfg(test)]
pub(crate) use self::policy::CommandClass;
pub(crate) use self::policy::{
    ConfigGeneration, ConfiguredWriter, ConfiguredWrites, Decision, OffWrites, PolicyBody,
    VersionedPolicy, WritePolicy,
};
#[cfg(test)]
pub(crate) use self::supervisor::LaneDecision;
pub(crate) use self::supervisor::{CloudBackupWriteSupervisor, WriteTicket};
pub(crate) use self::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};

pub(crate) type CloudBackupWriteError = crate::manager::cloud_backup_manager::CloudBackupError;
