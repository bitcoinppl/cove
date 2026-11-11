//! What an open dialog is bound to, and the current decision it reads from the latest screen
//!
//! A row sheet or delete confirmation holds a [`CloudBackupRowTarget`] and nothing Rust decided
//! about it: each frame reads the row's current action here, so a refresh that changes the row's
//! availability or its delete confirmation, or removes it, reaches the open dialog

use super::types::{
    CloudBackupListedRow, CloudBackupRowAction, CloudBackupRowActionKind, CloudBackupRowControl,
    CloudBackupRowTarget, CloudBackupScreen, CloudBackupScreenBody, CloudBackupSection,
};
use crate::manager::cloud_backup_manager::RecordId;

#[uniffi::export]
impl CloudBackupScreen {
    /// The listed row for `record_id`, or `None` once this screen no longer lists it
    pub fn listed_row(&self, record_id: RecordId) -> Option<CloudBackupListedRow> {
        self.find_listed_row(&record_id).cloned()
    }

    /// The current action for `target`, or `None` once Rust no longer offers that control on the
    /// row, which closes a dialog bound to it
    pub fn row_action(&self, target: CloudBackupRowTarget) -> Option<CloudBackupRowAction> {
        self.find_row_action(&target).cloned()
    }
}

#[uniffi::export]
impl CloudBackupRowActionKind {
    /// Which control this action is drawn as
    pub fn control(&self) -> CloudBackupRowControl {
        match self {
            Self::Restore => CloudBackupRowControl::Restore,
            Self::Delete { .. } => CloudBackupRowControl::Delete,
            Self::BackUpFirst => CloudBackupRowControl::BackUpFirst,
            Self::CheckAgain => CloudBackupRowControl::CheckAgain,
            Self::Verify => CloudBackupRowControl::Verify,
        }
    }
}

impl CloudBackupScreen {
    pub(crate) fn find_listed_row(&self, record_id: &RecordId) -> Option<&CloudBackupListedRow> {
        let CloudBackupScreenBody::Configured(configured) = &self.body else { return None };

        configured
            .sections
            .iter()
            .flat_map(|section| match section {
                CloudBackupSection::NotOnDevice(section) => section.rows.as_slice(),
                CloudBackupSection::ExtraCopies(section) => section.rows.as_slice(),
                _ => &[],
            })
            .find(|row| row.record_id == *record_id)
    }

    pub(crate) fn find_row_action(
        &self,
        target: &CloudBackupRowTarget,
    ) -> Option<&CloudBackupRowAction> {
        self.find_listed_row(&target.record_id)?
            .actions
            .iter()
            .find(|action| action.action.control() == target.control)
    }
}
