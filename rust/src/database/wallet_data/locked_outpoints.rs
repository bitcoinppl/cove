use std::sync::Arc;

use cove_types::lock_state::LockState;
use cove_util::result_ext::ResultExt as _;
use redb::{ReadableTable as _, TableDefinition};

use crate::database::{error::DatabaseError, key::OutPointKey};

/// Presence-based lock table: an outpoint key exists ⇒ locked, absent ⇒ unlocked.
///
/// The value is a unit `()` — we only care about key existence.
/// `()` implements `redb::Value` natively so no `Cbor` wrapper is needed.
pub(crate) const LOCKED_OUTPOINTS_TABLE: TableDefinition<OutPointKey, ()> =
    TableDefinition::new("locked_outpoints");

pub type Error = LockedOutpointsError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum LockedOutpointsError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

#[derive(Debug, Clone)]
pub struct LockedOutpointsTable {
    db: Arc<redb::Database>,
}

impl LockedOutpointsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        write_txn
            .open_table(LOCKED_OUTPOINTS_TABLE)
            .expect("failed to create locked_outpoints table");

        Self { db }
    }

    // MARK: Single-outpoint operations

    /// Mark an outpoint as locked.
    pub fn lock(&self, outpoint: &bitcoin::OutPoint) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(LOCKED_OUTPOINTS_TABLE)?;
            let key = OutPointKey::from(outpoint);
            table.insert(key, ())?;
        }

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;
        Ok(())
    }

    /// Remove the lock from an outpoint. No-op if already unlocked.
    pub fn unlock(&self, outpoint: &bitcoin::OutPoint) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(LOCKED_OUTPOINTS_TABLE)?;
            let key = OutPointKey::from(outpoint);
            table.remove(key)?;
        }

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;
        Ok(())
    }

    /// Returns `true` when the outpoint is locked.
    pub fn is_locked(&self, outpoint: &bitcoin::OutPoint) -> Result<bool, Error> {
        let read_txn = self.db.begin_read().map_err_str(DatabaseError::DatabaseAccess)?;
        let table =
            read_txn.open_table(LOCKED_OUTPOINTS_TABLE).map_err_str(DatabaseError::TableAccess)?;
        let key = OutPointKey::from(outpoint);

        Ok(table.get(key)?.is_some())
    }

    /// Check whether multiple outpoints are locked in a single read transaction.
    pub fn are_locked(&self, outpoints: &[bitcoin::OutPoint]) -> Result<Vec<bool>, Error> {
        if outpoints.is_empty() {
            return Ok(vec![]);
        }

        let read_txn = self.db.begin_read().map_err_str(DatabaseError::DatabaseAccess)?;
        let table =
            read_txn.open_table(LOCKED_OUTPOINTS_TABLE).map_err_str(DatabaseError::TableAccess)?;

        let mut results = Vec::with_capacity(outpoints.len());
        for outpoint in outpoints {
            let key = OutPointKey::from(outpoint);
            results.push(table.get(key)?.is_some());
        }

        Ok(results)
    }

    // MARK: Bulk operations

    /// Lock every outpoint in the slice within a single write transaction.
    pub fn lock_all(&self, outpoints: &[bitcoin::OutPoint]) -> Result<(), Error> {
        if outpoints.is_empty() {
            return Ok(());
        }

        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(LOCKED_OUTPOINTS_TABLE)?;
            for outpoint in outpoints {
                let key = OutPointKey::from(outpoint);
                table.insert(key, ())?;
            }
        }

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;
        Ok(())
    }

    /// Unlock every outpoint in the slice within a single write transaction.
    pub fn unlock_all(&self, outpoints: &[bitcoin::OutPoint]) -> Result<(), Error> {
        if outpoints.is_empty() {
            return Ok(());
        }

        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        {
            let mut table = write_txn.open_table(LOCKED_OUTPOINTS_TABLE)?;
            for outpoint in outpoints {
                let key = OutPointKey::from(outpoint);
                table.remove(key)?;
            }
        }

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;
        Ok(())
    }

    /// Return all currently-locked outpoints.
    pub fn all_locked(&self) -> Result<Vec<bitcoin::OutPoint>, Error> {
        let read_txn = self.db.begin_read().map_err_str(DatabaseError::DatabaseAccess)?;
        let table =
            read_txn.open_table(LOCKED_OUTPOINTS_TABLE).map_err_str(DatabaseError::TableAccess)?;

        let outpoints: std::result::Result<Vec<_>, redb::StorageError> = table
            .iter()?
            .map(|entry| {
                let (key, _) = entry?;
                let k = key.value();
                Ok(bitcoin::OutPoint { txid: k.id(), vout: k.index })
            })
            .collect();

        Ok(outpoints.map_err(|e| DatabaseError::DatabaseAccess(e.to_string()))?)
    }

    /// Atomically compute aggregate lock state and toggle all outpoints.
    ///
    /// This performs the read (aggregate) and write (lock/unlock all) within a
    /// **single write transaction** so there is no TOCTOU race if the user
    /// double-taps the toggle button.
    ///
    /// - `Unlocked` / `Mixed` → lock all → returns `Locked`
    /// - `Locked` → unlock all → returns `Unlocked`
    pub fn toggle_all(&self, outpoints: &[bitcoin::OutPoint]) -> Result<LockState, Error> {
        if outpoints.is_empty() {
            return Ok(LockState::Unlocked);
        }

        let write_txn = self.db.begin_write().map_err_str(DatabaseError::DatabaseAccess)?;

        let new_state = {
            let mut table = write_txn.open_table(LOCKED_OUTPOINTS_TABLE)?;

            // Step 1: Read aggregate state within this transaction
            let mut locked_count: usize = 0;
            for outpoint in outpoints {
                let key = OutPointKey::from(outpoint);
                if table.get(key)?.is_some() {
                    locked_count += 1;
                }
            }

            let all_locked = locked_count == outpoints.len();

            // Step 2: Toggle based on current state
            if all_locked {
                // All locked → unlock all
                for outpoint in outpoints {
                    let key = OutPointKey::from(outpoint);
                    table.remove(key)?;
                }
                LockState::Unlocked
            } else {
                // Unlocked or Mixed → lock all
                for outpoint in outpoints {
                    let key = OutPointKey::from(outpoint);
                    table.insert(key, ())?;
                }
                LockState::Locked
            }
        };

        write_txn.commit().map_err_str(DatabaseError::DatabaseAccess)?;
        Ok(new_state)
    }
}

impl From<redb::TransactionError> for Error {
    fn from(error: redb::TransactionError) -> Self {
        Self::Database(error.into())
    }
}

impl From<redb::TableError> for Error {
    fn from(error: redb::TableError) -> Self {
        Self::Database(error.into())
    }
}

impl From<redb::StorageError> for Error {
    fn from(error: redb::StorageError) -> Self {
        Self::Database(error.into())
    }
}

#[cfg(test)]
mod tests {

    use crate::database::wallet_data::WalletDataDb;
    use crate::wallet::metadata::WalletId;
    use cove_types::lock_state::LockState;
    use std::str::FromStr;

    fn test_outpoint(vout: u32) -> bitcoin::OutPoint {
        bitcoin::OutPoint {
            txid: bitcoin::Txid::from_str(
                "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290",
            )
            .unwrap(),
            vout,
        }
    }

    #[test]
    fn test_lock_and_is_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        let op = test_outpoint(0);
        assert!(!table.is_locked(&op).unwrap());

        table.lock(&op).unwrap();
        assert!(table.is_locked(&op).unwrap());
    }

    #[test]
    fn test_unlock() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        let op = test_outpoint(1);
        table.lock(&op).unwrap();
        assert!(table.is_locked(&op).unwrap());

        table.unlock(&op).unwrap();
        assert!(!table.is_locked(&op).unwrap());
    }

    #[test]
    fn test_unlock_noop_when_already_unlocked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        let op = test_outpoint(2);
        // should not error
        table.unlock(&op).unwrap();
        assert!(!table.is_locked(&op).unwrap());
    }

    #[test]
    fn test_bulk_lock_and_unlock() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        let ops: Vec<_> = (0..5).map(test_outpoint).collect();
        table.lock_all(&ops).unwrap();

        for op in &ops {
            assert!(table.is_locked(op).unwrap());
        }

        table.unlock_all(&ops[..3]).unwrap();

        assert!(!table.is_locked(&ops[0]).unwrap());
        assert!(!table.is_locked(&ops[1]).unwrap());
        assert!(!table.is_locked(&ops[2]).unwrap());
        assert!(table.is_locked(&ops[3]).unwrap());
        assert!(table.is_locked(&ops[4]).unwrap());
    }

    #[test]
    fn test_all_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        assert!(table.all_locked().unwrap().is_empty());

        let ops: Vec<_> = (0..3).map(test_outpoint).collect();
        table.lock_all(&ops).unwrap();

        let all = table.all_locked().unwrap();
        assert_eq!(all.len(), 3);

        for op in &ops {
            assert!(all.contains(op));
        }
    }

    #[test]
    fn test_bulk_lock_empty_is_noop() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        table.lock_all(&[]).unwrap();
        table.unlock_all(&[]).unwrap();
        assert!(table.all_locked().unwrap().is_empty());
    }

    #[test]
    fn test_toggle_from_unlocked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;
        let ops: Vec<_> = (0..3).map(test_outpoint).collect();

        let state = table.toggle_all(&ops).unwrap();
        assert_eq!(state, LockState::Locked);

        for op in &ops {
            assert!(table.is_locked(op).unwrap());
        }
    }

    #[test]
    fn test_toggle_from_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;
        let ops: Vec<_> = (0..3).map(test_outpoint).collect();

        table.lock_all(&ops).unwrap();
        let state = table.toggle_all(&ops).unwrap();
        assert_eq!(state, LockState::Unlocked);

        for op in &ops {
            assert!(!table.is_locked(op).unwrap());
        }
    }

    #[test]
    fn test_toggle_from_mixed() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;
        let ops: Vec<_> = (0..3).map(test_outpoint).collect();

        // lock only the first one → mixed
        table.lock(&ops[0]).unwrap();
        let state = table.toggle_all(&ops).unwrap();
        assert_eq!(state, LockState::Locked);

        for op in &ops {
            assert!(table.is_locked(op).unwrap());
        }
    }

    #[test]
    fn test_toggle_empty_is_unlocked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::new());
        let table = &db.locked_outpoints;

        let state = table.toggle_all(&[]).unwrap();
        assert_eq!(state, LockState::Unlocked);
    }
}
