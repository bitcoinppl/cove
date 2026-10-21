use cove_util::result_ext::ResultExt as _;

use super::{
    CLOUD_BACKUP_STATE_TABLE, CLOUD_BLOB_SYNC_STATE_TABLE, CURRENT_KEY, CloudBackupStateTable,
    PersistedCloudBackupState, PersistedCloudBlobSyncState,
};
use crate::database::Error;

impl CloudBackupStateTable {
    /// Writes restored configured state and dirty-wallet rows in one transaction
    pub(crate) fn persist_restored_namespace_activation(
        &self,
        configured: &PersistedCloudBackupState,
        dirty_states: &[PersistedCloudBlobSyncState],
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut table =
                write_txn.open_table(CLOUD_BACKUP_STATE_TABLE).map_err_str(Error::TableAccess)?;

            #[cfg(test)]
            test_support::fail_configured_state_write()?;

            table.insert(CURRENT_KEY, configured).map_err_str(Error::TableAccess)?;
        }

        {
            let mut table = write_txn
                .open_table(CLOUD_BLOB_SYNC_STATE_TABLE)
                .map_err_str(Error::TableAccess)?;

            for state in dirty_states {
                #[cfg(test)]
                test_support::fail_dirty_wallet_write()?;

                table.insert(state.record_id(), state).map_err_str(Error::TableAccess)?;
            }
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    static FAIL_CONFIGURED_STATE_WRITE: AtomicBool = AtomicBool::new(false);
    /// Dirty-wallet writes left to succeed before the injected failure, `usize::MAX` when disarmed
    static DIRTY_WALLET_WRITES_BEFORE_FAILURE: AtomicUsize = AtomicUsize::new(usize::MAX);

    pub(crate) fn fail_next_configured_state_write() {
        FAIL_CONFIGURED_STATE_WRITE.store(true, Ordering::SeqCst);
    }

    /// Fails the dirty-wallet write at `index` of the next restored namespace activation
    pub(crate) fn fail_dirty_wallet_write_at(index: usize) {
        DIRTY_WALLET_WRITES_BEFORE_FAILURE.store(index, Ordering::SeqCst);
    }

    pub(crate) fn reset() {
        FAIL_CONFIGURED_STATE_WRITE.store(false, Ordering::SeqCst);
        DIRTY_WALLET_WRITES_BEFORE_FAILURE.store(usize::MAX, Ordering::SeqCst);
    }

    pub(crate) fn fail_configured_state_write() -> Result<(), Error> {
        if FAIL_CONFIGURED_STATE_WRITE.swap(false, Ordering::SeqCst) {
            return Err(Error::DatabaseAccess(
                "injected restored namespace configured-state failure".into(),
            ));
        }

        Ok(())
    }

    pub(crate) fn fail_dirty_wallet_write() -> Result<(), Error> {
        let remaining = DIRTY_WALLET_WRITES_BEFORE_FAILURE.load(Ordering::SeqCst);
        if remaining == usize::MAX {
            return Ok(());
        }

        if remaining > 0 {
            DIRTY_WALLET_WRITES_BEFORE_FAILURE.store(remaining - 1, Ordering::SeqCst);
            return Ok(());
        }

        DIRTY_WALLET_WRITES_BEFORE_FAILURE.store(usize::MAX, Ordering::SeqCst);
        Err(Error::TableAccess("injected restored namespace dirty-wallet failure".into()))
    }
}
