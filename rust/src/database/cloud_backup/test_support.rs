//! Raw writes and write faults for tests only; production writes go through the lifecycle owner

use cove_util::result_ext::ResultExt as _;
use parking_lot::Mutex;

use super::{CLOUD_BACKUP_STATE_TABLE, CURRENT_KEY, CloudBackupStateTable};
use crate::database::Error;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::rebase_owners_after_test_write;

type WriteFault = Box<dyn Fn(&PersistedCloudBackupState) -> bool + Send>;

/// The failure the owner's next matching write meets before it commits
static WRITE_FAULT: Mutex<Option<WriteFault>> = Mutex::new(None);

impl CloudBackupStateTable {
    /// Fails the owner's next write whose new record matches `matches`, leaving the stored record
    /// as it was, the way a failed redb commit does
    pub(crate) fn fail_next_write_where(
        &self,
        matches: impl Fn(&PersistedCloudBackupState) -> bool + Send + 'static,
    ) {
        *WRITE_FAULT.lock() = Some(Box::new(matches));
    }

    /// Takes the armed write fault when `state` meets it
    pub(crate) fn take_write_fault(state: &PersistedCloudBackupState) -> Result<(), Error> {
        let mut fault = WRITE_FAULT.lock();
        if !fault.as_ref().is_some_and(|matches| matches(state)) {
            return Ok(());
        }

        *fault = None;
        Err(Error::DatabaseAccess("injected cloud backup state write failure".into()))
    }

    /// Seeds the record directly, bypassing the lifecycle owner
    pub fn set(&self, value: &PersistedCloudBackupState) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table =
                write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;
            table.insert(CURRENT_KEY, value).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;
        rebase_owners_after_test_write();

        Ok(())
    }

    pub fn delete(&self) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table =
                write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;
            table.remove(CURRENT_KEY).map_err_str(Error::TableAccess)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;
        rebase_owners_after_test_write();

        Ok(())
    }
}
