//! The write policy: which command classes the write lane runs, derived from the runtime
//!
//! The supervisor derives a [`WritePolicy`] from its `Runtime` after every change and pushes it,
//! versioned, to the write supervisor. The write supervisor decides each command at the head of
//! its queue, when it is about to run, against the latest policy it received
//!
//! Nothing here is stored state that someone sets or clears: there is no holder set and no
//! unblock. A pause ends because the runtime moved on, so an old operation can't lift a newer
//! pause, and a command from an older configuration can't recreate remote data a newer one
//! deleted

use std::sync::atomic::{AtomicU64, Ordering};

use crate::manager::cloud_backup_manager::admission::PauseReason;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

/// Identifies one configuration of the backup within this process
///
/// A new generation starts whenever the lifecycle body changes kind (off, configured, finishing a
/// Turn Off, switching Drive accounts, resetting) or a reinitialize replaces the configuration.
/// Commands that carry no operation claim carry the generation they read their configuration
/// under, so a command from an older configuration is dropped instead of writing to the new one.
/// Generations are process-local: a relaunch starts a fresh lane with no queued commands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConfigGeneration(u64);

static NEXT_CONFIG_GENERATION: AtomicU64 = AtomicU64::new(1);

impl ConfigGeneration {
    pub(crate) fn next() -> Self {
        Self(NEXT_CONFIG_GENERATION.fetch_add(1, Ordering::Relaxed))
    }
}

/// What a write command is, which decides when the policy lets it run
///
/// Operation-owned classes carry the claim of the operation that sent them: an operation holds
/// its body until it ends, so its claim also pins the configuration it read. Classes with no
/// operation carry the [`ConfigGeneration`] instead
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandClass {
    /// A dirty wallet upload or another background blob write
    Ordinary { generation: ConfigGeneration },
    /// A write by the running operation: Enable, an account switch's reinitialize, verification,
    /// passkey repair and the other configured operations, or Turn Off's drain barrier
    Operation(CloudBackupExclusiveOperationClaim),
    /// Turn Off's delete of the namespace it committed to delete
    TurnOffDelete { claim: CloudBackupExclusiveOperationClaim, namespace_id: String },
    /// A row or batch delete of wallet records in the active namespace
    RowDelete(CloudBackupExclusiveOperationClaim),
    /// Recover Other Backups or Delete Other Backups deleting another namespace
    OtherBackupDelete(CloudBackupExclusiveOperationClaim),
    /// Discarding the namespace an interrupted Enable wrote
    EnableDiscard(CloudBackupExclusiveOperationClaim),
    /// Cleanup of a namespace a merge made redundant, proven under `generation`
    SourceCleanup { generation: ConfigGeneration },
}

impl CommandClass {
    /// The operation that sent the command, if any
    pub(crate) fn claim(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        match self {
            Self::Operation(claim)
            | Self::TurnOffDelete { claim, .. }
            | Self::RowDelete(claim)
            | Self::OtherBackupDelete(claim)
            | Self::EnableDiscard(claim) => Some(*claim),
            Self::Ordinary { .. } | Self::SourceCleanup { .. } => None,
        }
    }
}

/// What the write lane does with a command at the head of its queue
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    /// Not now; the command's local state stays as it was and the lane wakes its producer when
    /// the pause ends
    Paused(PauseReason),
    /// The command's operation ended or its configuration was replaced; it is dropped
    Stale,
}

/// The lifecycle body as the write lane needs it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyBody {
    Configured {
        op: Option<ConfiguredWrites>,
    },
    /// Disabled, corrupted, or recovering an interrupted Enable
    Off {
        op: Option<OffWrites>,
    },
    FinishingDisable {
        namespace_id: String,
        op: Option<CloudBackupExclusiveOperationClaim>,
    },
    AccountSwitch {
        op: Option<CloudBackupExclusiveOperationClaim>,
    },
    /// Nothing runs: the record failed to load or a reset is wiping local state
    Closed(PauseReason),
}

/// The running configured operation and what it may write
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfiguredWrites {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) kind: ConfiguredWriter,
}

/// How a configured operation writes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfiguredWriter {
    /// Turn Off before its commit: background writes pause until it commits or ends
    TurnOff,
    /// A reinitialize replacing the configuration: background writes and source cleanup pause
    /// until it ends, and it drains the lane before it reads the namespaces it may promote
    Reinitialize,
    RowDelete,
    OtherBackups,
    /// Discarding a stopped reinitialize deletes the namespace it wrote, never the active one
    Discard,
    /// Every other configured operation writes only through its claim
    Other,
}

/// The running operation while the backup is off
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OffWrites {
    Enable(CloudBackupExclusiveOperationClaim),
    Discard(CloudBackupExclusiveOperationClaim),
    /// A restore writes nothing through the lane until it activates the namespace
    Restore(CloudBackupExclusiveOperationClaim),
}

/// Which command classes may run, derived from the runtime; see the module docs
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritePolicy {
    pub(crate) generation: ConfigGeneration,
    pub(crate) body: PolicyBody,
}

/// A policy with the version the supervisor published it as
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedPolicy {
    pub(crate) version: u64,
    pub(crate) policy: WritePolicy,
}

impl WritePolicy {
    /// The policy before the supervisor published one: nothing runs yet
    pub(crate) fn starting() -> Self {
        Self { generation: ConfigGeneration(0), body: PolicyBody::Closed(PauseReason::BackupOff) }
    }

    /// Whether background (ordinary) writes run under this policy
    pub(crate) fn runs_ordinary(&self) -> bool {
        self.decide(&CommandClass::Ordinary { generation: self.generation }) == Decision::Run
    }

    /// Decides a command against this policy; see the table in `docs/cloud_backup.md`
    pub(crate) fn decide(&self, class: &CommandClass) -> Decision {
        use CommandClass as C;
        use PolicyBody as B;

        match class {
            C::Ordinary { generation } | C::SourceCleanup { generation } => {
                if *generation != self.generation {
                    return Decision::Stale;
                }

                match &self.body {
                    B::Configured {
                        op: Some(ConfiguredWrites { kind: ConfiguredWriter::TurnOff, .. }),
                    } => Decision::Paused(PauseReason::TurnOff),
                    B::Configured {
                        op: Some(ConfiguredWrites { kind: ConfiguredWriter::Reinitialize, .. }),
                    } => Decision::Paused(PauseReason::Reinitialize),
                    B::Configured { .. } => Decision::Run,
                    B::Off { .. } => Decision::Paused(PauseReason::BackupOff),
                    B::FinishingDisable { .. } => Decision::Paused(PauseReason::TurnOff),
                    B::AccountSwitch { .. } => Decision::Paused(PauseReason::AccountSwitch),
                    B::Closed(reason) => Decision::Paused(*reason),
                }
            }

            C::Operation(claim) => match &self.body {
                B::Configured { op: Some(op) } if op.claim == *claim => Decision::Run,
                B::Off { op: Some(OffWrites::Enable(running)) }
                | B::AccountSwitch { op: Some(running) }
                | B::FinishingDisable { op: Some(running), .. }
                    if running == claim =>
                {
                    Decision::Run
                }
                _ => Decision::Stale,
            },

            C::TurnOffDelete { claim, namespace_id } => match &self.body {
                B::FinishingDisable { namespace_id: committed, op: Some(running) }
                    if running == claim && committed == namespace_id =>
                {
                    Decision::Run
                }
                _ => Decision::Stale,
            },

            C::RowDelete(claim) => match &self.body {
                B::Configured {
                    op: Some(ConfiguredWrites { claim: running, kind: ConfiguredWriter::RowDelete }),
                } if running == claim => Decision::Run,
                _ => Decision::Stale,
            },

            C::OtherBackupDelete(claim) => match &self.body {
                B::Configured {
                    op:
                        Some(ConfiguredWrites { claim: running, kind: ConfiguredWriter::OtherBackups }),
                } if running == claim => Decision::Run,
                _ => Decision::Stale,
            },

            C::EnableDiscard(claim) => match &self.body {
                B::Off { op: Some(OffWrites::Discard(running)) }
                | B::Configured {
                    op: Some(ConfiguredWrites { claim: running, kind: ConfiguredWriter::Discard }),
                } if running == claim => Decision::Run,
                _ => Decision::Stale,
            },
        }
    }
}
