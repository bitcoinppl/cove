//! The notices the screen shows: the heal notice from the stored record and the outcome of the
//! last control that didn't do what it was asked

use super::types::{CloudBackupHealNotice, CloudBackupLastOutcome};
use crate::manager::cloud_backup_manager::CloudBackupStateReducer;

/// The notices the screen projection reads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Notices {
    pub(crate) heal: Option<CloudBackupHealNotice>,
    pub(crate) last_outcome: Option<CloudBackupLastOutcome>,
}

impl Notices {
    /// The notices as the reducer holds them until their owner projects them
    pub(crate) fn from_reducer(reducer: &CloudBackupStateReducer) -> Self {
        Self { heal: reducer.heal_notice(), last_outcome: reducer.last_outcome().cloned() }
    }
}
