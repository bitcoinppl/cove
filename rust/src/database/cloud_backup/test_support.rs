//! Raw writes and write faults for tests only; production writes go through the lifecycle owner

use std::sync::atomic::{AtomicUsize, Ordering};

use cove_util::result_ext::ResultExt as _;
use parking_lot::Mutex;

use super::{CLOUD_BACKUP_STATE_TABLE, CURRENT_KEY, CloudBackupStateTable};
use crate::database::Error;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::rebase_owners_after_test_write;

type WriteFault = Box<dyn Fn(&PersistedCloudBackupState) -> bool + Send>;

/// The failure the owner's next matching write meets before it commits
static WRITE_FAULT: Mutex<Option<WriteFault>> = Mutex::new(None);

/// How many of the next record reads fail; `usize::MAX` fails every read until reset
static READ_FAULTS: AtomicUsize = AtomicUsize::new(0);

/// How many record reads go through before the armed read faults start failing reads
static READ_FAULT_SKIPS: AtomicUsize = AtomicUsize::new(0);

/// Every read of the record fails while this is alive
#[must_use = "reads fail only while the fault is alive"]
pub(crate) struct ReadFault(());

impl Drop for ReadFault {
    fn drop(&mut self) {
        READ_FAULT_SKIPS.store(0, Ordering::SeqCst);
        READ_FAULTS.store(0, Ordering::SeqCst);
    }
}

impl CloudBackupStateTable {
    /// Fails the next `count` reads of the record, the way a transient redb read error does;
    /// writes still read and commit inside their own transaction
    pub(crate) fn fail_next_reads(&self, count: usize) {
        READ_FAULT_SKIPS.store(0, Ordering::SeqCst);
        READ_FAULTS.store(count, Ordering::SeqCst);
    }

    /// Lets `skip` reads of the record through, then fails the one after them, for a read only
    /// its place in a deterministic sequence names
    pub(crate) fn fail_read_after(&self, skip: usize) {
        READ_FAULT_SKIPS.store(skip, Ordering::SeqCst);
        READ_FAULTS.store(1, Ordering::SeqCst);
    }

    /// Fails every read of the record until the returned fault is dropped
    pub(crate) fn fail_reads(&self) -> ReadFault {
        READ_FAULT_SKIPS.store(0, Ordering::SeqCst);
        READ_FAULTS.store(usize::MAX, Ordering::SeqCst);
        ReadFault(())
    }

    /// Takes one armed read fault, if any
    pub(crate) fn take_read_fault() -> Result<(), Error> {
        if READ_FAULTS.load(Ordering::SeqCst) > 0
            && READ_FAULT_SKIPS
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |skips| skips.checked_sub(1))
                .is_ok()
        {
            return Ok(());
        }

        let taken = READ_FAULTS
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| count.checked_sub(1));
        if taken.is_err() {
            return Ok(());
        }

        Err(Error::DatabaseAccess("injected cloud backup state read failure".into()))
    }

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
