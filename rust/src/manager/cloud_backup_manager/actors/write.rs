//! Serialized cloud write lane for Cloud Backup
//!
//! The write supervisor orders remote writes, decides each command against the write policy the
//! lifecycle supervisor derives from its runtime, and applies local post-write state only after
//! the remote command succeeds. The worker is the leaf actor that talks to cloud storage and
//! saves the keychain writes producers proved off the actor

mod client;
mod keychain;
mod policy;
mod supervisor;
mod types;
mod worker;

pub(crate) use self::client::CloudBackupWriteClient;
pub(crate) use self::keychain::OwnedKeychainWrite;
#[cfg(test)]
pub(crate) use self::policy::CommandClass;
pub(crate) use self::policy::{
    ConfigGeneration, ConfiguredWriter, ConfiguredWrites, Decision, OffWrites, PolicyBody,
    VersionedPolicy, WritePolicy,
};
pub(crate) use self::supervisor::{CloudBackupWriteSupervisor, WriteTicket};
#[cfg(test)]
pub(crate) use self::supervisor::{LaneDecision, LaneQueue};
pub(crate) use self::types::{
    CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode, CloudBackupWalletCountRefresh,
    CloudBackupWriteCompletion,
};
pub(crate) use self::worker::{RowDeleteReport, WalletUploadOutcome};

pub(crate) type CloudBackupWriteError = crate::manager::cloud_backup_manager::CloudBackupError;
