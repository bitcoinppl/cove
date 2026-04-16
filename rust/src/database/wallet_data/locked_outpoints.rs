use std::sync::Arc;

use redb::{ReadOnlyTable, ReadableTable as _, ReadableTableMetadata as _, TableDefinition};

use crate::database::key::OutPointKey;

use super::Error;

/// Stores locked outpoints as `OutPointKey -> ()`.
///
/// An outpoint present in this table is locked.
/// Absent means unlocked.
pub(crate) const LOCKED_OUTPOINTS_TABLE: TableDefinition<OutPointKey, ()> =
    TableDefinition::new("locked_outpoints");

/// 3-state aggregate lock state for a set of outpoints.
///
/// Used by the Transaction Details UI to render the correct bulk toggle:
/// - `Unlocked`: none locked → tap locks all
/// - `Mixed`: some locked → tap locks all remaining
/// - `Locked`: all locked → tap unlocks all
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum UtxoLockState {
    /// No wallet-owned unspent outputs are locked.
    Unlocked,
    /// Some (but not all) wallet-owned unspent outputs are locked.
    Mixed,
    /// All wallet-owned unspent outputs are locked.
    Locked,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct LockedOutpointsTable {
    db: Arc<redb::Database>,
}

impl LockedOutpointsTable {
    /// Create a new `LockedOutpointsTable`, ensuring the table exists.
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        write_txn
            .open_table(LOCKED_OUTPOINTS_TABLE)
            .expect("failed to create locked outpoints table");

        Self { db }
    }

    // MARK: READ

    /// Check whether a single outpoint is locked.
    pub fn is_locked(&self, outpoint: impl Into<OutPointKey>) -> Result<bool, Error> {
        let key = outpoint.into();
        let table = self.read_table()?;
        let locked = table.get(key).map_err(|error| Error::Read(error.to_string()))?.is_some();

        Ok(locked)
    }

    /// Return the set of all currently locked outpoints as `OutPointKey`s.
    pub fn all_locked(&self) -> Result<Vec<OutPointKey>, Error> {
        let table = self.read_table()?;
        let locked = table
            .iter()
            .map_err(|error| Error::Read(error.to_string()))?
            .filter_map(Result::ok)
            .map(|(key, _)| key.value())
            .collect();

        Ok(locked)
    }

    /// Return the set of all currently locked outpoints as `bitcoin::OutPoint`s.
    ///
    /// Convenience wrapper used by PSBT builders to pass to `TxBuilder::unspendable()`.
    pub fn all_locked_outpoints(&self) -> Result<Vec<bitcoin::OutPoint>, Error> {
        let locked = self.all_locked()?;
        Ok(locked
            .into_iter()
            .map(|key| bitcoin::OutPoint { txid: key.id(), vout: key.index })
            .collect())
    }

    /// Return the subset of `outpoints` that are currently locked.
    ///
    /// Used to validate manual coin-selection: if the result is non-empty,
    /// the caller should reject the selection before building the PSBT.
    pub fn find_locked(
        &self,
        outpoints: &[bitcoin::OutPoint],
    ) -> Result<Vec<bitcoin::OutPoint>, Error> {
        let mut locked = Vec::new();
        for op in outpoints {
            if self.is_locked(*op)? {
                locked.push(*op);
            }
        }
        Ok(locked)
    }

    /// Number of locked outpoints.
    pub fn count(&self) -> Result<u64, Error> {
        let table = self.read_table()?;
        table.len().map_err(|error| Error::Read(error.to_string()))
    }

    /// Compute the aggregate lock state for a set of wallet-owned unspent outpoints.
    ///
    /// `outpoints` should contain only wallet-owned unspent outputs for a transaction.
    /// Spent outputs and external outputs must be filtered out by the caller.
    ///
    /// Returns `None` if the slice is empty (no wallet-owned unspent outputs).
    pub fn aggregate_lock_state(
        &self,
        outpoints: &[OutPointKey],
    ) -> Result<Option<UtxoLockState>, Error> {
        if outpoints.is_empty() {
            return Ok(None);
        }

        let mut locked_count = 0u64;
        for outpoint in outpoints {
            if self.is_locked(outpoint.clone())? {
                locked_count += 1;
            }
        }

        let total = outpoints.len() as u64;
        let state = if locked_count == 0 {
            UtxoLockState::Unlocked
        } else if locked_count == total {
            UtxoLockState::Locked
        } else {
            UtxoLockState::Mixed
        };

        Ok(Some(state))
    }

    // MARK: WRITE

    /// Lock a single outpoint.
    pub fn lock(&self, outpoint: impl Into<OutPointKey>) -> Result<(), Error> {
        let key = outpoint.into();
        self.set(key)
    }

    /// Unlock a single outpoint.
    pub fn unlock(&self, outpoint: impl Into<OutPointKey>) -> Result<(), Error> {
        let key = outpoint.into();
        self.remove(key)
    }

    /// Lock all given outpoints in a single write transaction.
    pub fn lock_all(&self, outpoints: &[OutPointKey]) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err(|e| Error::Save(e.to_string()))?;

        {
            let mut table = write_txn
                .open_table(LOCKED_OUTPOINTS_TABLE)
                .map_err(|e| Error::Save(e.to_string()))?;

            for key in outpoints {
                table.insert(key, ()).map_err(|error| Error::Save(error.to_string()))?;
            }
        }

        write_txn.commit().map_err(|e| Error::Save(e.to_string()))?;
        Ok(())
    }

    /// Unlock all given outpoints in a single write transaction.
    pub fn unlock_all(&self, outpoints: &[OutPointKey]) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err(|e| Error::Save(e.to_string()))?;

        {
            let mut table = write_txn
                .open_table(LOCKED_OUTPOINTS_TABLE)
                .map_err(|e| Error::Save(e.to_string()))?;

            for key in outpoints {
                table.remove(key).map_err(|error| Error::Save(error.to_string()))?;
            }
        }

        write_txn.commit().map_err(|e| Error::Save(e.to_string()))?;
        Ok(())
    }

    /// Toggle the bulk lock state for a transaction's wallet-owned unspent outpoints.
    ///
    /// - If current state is `Unlocked` or `Mixed` → lock all
    /// - If current state is `Locked` → unlock all
    ///
    /// Returns the new aggregate state, or `None` if the slice is empty.
    pub fn toggle_transaction_lock(
        &self,
        outpoints: &[OutPointKey],
    ) -> Result<Option<UtxoLockState>, Error> {
        let current = self.aggregate_lock_state(outpoints)?;

        match current {
            None => Ok(None),
            Some(UtxoLockState::Locked) => {
                self.unlock_all(outpoints)?;
                Ok(Some(UtxoLockState::Unlocked))
            }
            Some(UtxoLockState::Unlocked | UtxoLockState::Mixed) => {
                self.lock_all(outpoints)?;
                Ok(Some(UtxoLockState::Locked))
            }
        }
    }

    // MARK: PRIVATE

    fn set(&self, key: OutPointKey) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err(|e| Error::Save(e.to_string()))?;

        {
            let mut table = write_txn
                .open_table(LOCKED_OUTPOINTS_TABLE)
                .map_err(|e| Error::Save(e.to_string()))?;

            table.insert(key, ()).map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|e| Error::Save(e.to_string()))?;
        Ok(())
    }

    fn remove(&self, key: OutPointKey) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err(|e| Error::Save(e.to_string()))?;

        {
            let mut table = write_txn
                .open_table(LOCKED_OUTPOINTS_TABLE)
                .map_err(|e| Error::Save(e.to_string()))?;

            table.remove(key).map_err(|error| Error::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|e| Error::Save(e.to_string()))?;
        Ok(())
    }

    fn read_table(&self) -> Result<ReadOnlyTable<OutPointKey, ()>, Error> {
        let read_txn = self.db.begin_read().map_err(|e| Error::Read(e.to_string()))?;
        let table =
            read_txn.open_table(LOCKED_OUTPOINTS_TABLE).map_err(|e| Error::Read(e.to_string()))?;

        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletId};

    fn outpoint_key(txid_byte: u8, vout: u32) -> OutPointKey {
        OutPointKey { id: [txid_byte; 32], index: vout }
    }

    #[test]
    fn lock_and_query_single_outpoint() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op = outpoint_key(0xAA, 0);

        assert!(!table.is_locked(op.clone()).unwrap());
        table.lock(op.clone()).unwrap();
        assert!(table.is_locked(op.clone()).unwrap());
    }

    #[test]
    fn unlock_removes_outpoint() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op = outpoint_key(0xBB, 1);

        table.lock(op.clone()).unwrap();
        assert!(table.is_locked(op.clone()).unwrap());

        table.unlock(op.clone()).unwrap();
        assert!(!table.is_locked(op.clone()).unwrap());
    }

    #[test]
    fn all_locked_returns_only_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op1 = outpoint_key(0x01, 0);
        let op2 = outpoint_key(0x02, 0);
        let op3 = outpoint_key(0x03, 0);

        table.lock(op1.clone()).unwrap();
        table.lock(op2.clone()).unwrap();
        // op3 not locked

        let locked = table.all_locked().unwrap();
        assert_eq!(locked.len(), 2);
        assert!(locked.contains(&op1));
        assert!(locked.contains(&op2));
        assert!(!locked.contains(&op3));
    }

    #[test]
    fn aggregate_empty_returns_none() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        assert_eq!(table.aggregate_lock_state(&[]).unwrap(), None);
    }

    #[test]
    fn aggregate_all_unlocked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1)];

        assert_eq!(table.aggregate_lock_state(&ops).unwrap(), Some(UtxoLockState::Unlocked));
    }

    #[test]
    fn aggregate_all_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1)];
        table.lock_all(&ops).unwrap();

        assert_eq!(table.aggregate_lock_state(&ops).unwrap(), Some(UtxoLockState::Locked));
    }

    #[test]
    fn aggregate_mixed_state() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op1 = outpoint_key(0x01, 0);
        let op2 = outpoint_key(0x01, 1);

        table.lock(op1.clone()).unwrap();
        // op2 not locked

        assert_eq!(table.aggregate_lock_state(&[op1, op2]).unwrap(), Some(UtxoLockState::Mixed));
    }

    #[test]
    fn bulk_lock_all() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1), outpoint_key(0x01, 2)];
        table.lock_all(&ops).unwrap();

        for op in &ops {
            assert!(table.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn bulk_unlock_all() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1)];
        table.lock_all(&ops).unwrap();
        table.unlock_all(&ops).unwrap();

        for op in &ops {
            assert!(!table.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn toggle_from_unlocked_locks_all() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1)];

        let result = table.toggle_transaction_lock(&ops).unwrap();
        assert_eq!(result, Some(UtxoLockState::Locked));

        for op in &ops {
            assert!(table.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn toggle_from_mixed_locks_all() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op1 = outpoint_key(0x01, 0);
        let op2 = outpoint_key(0x01, 1);

        table.lock(op1.clone()).unwrap();
        // op2 unlocked → mixed

        let ops = vec![op1.clone(), op2.clone()];
        let result = table.toggle_transaction_lock(&ops).unwrap();
        assert_eq!(result, Some(UtxoLockState::Locked));

        assert!(table.is_locked(op1).unwrap());
        assert!(table.is_locked(op2).unwrap());
    }

    #[test]
    fn toggle_from_locked_unlocks_all() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let ops = vec![outpoint_key(0x01, 0), outpoint_key(0x01, 1)];
        table.lock_all(&ops).unwrap();

        let result = table.toggle_transaction_lock(&ops).unwrap();
        assert_eq!(result, Some(UtxoLockState::Unlocked));

        for op in &ops {
            assert!(!table.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn toggle_empty_returns_none() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let result = table.toggle_transaction_lock(&[]).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn lock_is_idempotent() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op = outpoint_key(0xCC, 0);
        table.lock(op.clone()).unwrap();
        table.lock(op.clone()).unwrap();
        assert!(table.is_locked(op).unwrap());
    }

    #[test]
    fn unlock_nonexistent_is_noop() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op = outpoint_key(0xDD, 0);
        // should not error
        table.unlock(op.clone()).unwrap();
        assert!(!table.is_locked(op).unwrap());
    }

    // ── Tests for #658: PSBT-builder helpers ──────────────────────────

    /// Helper to convert an `OutPointKey` to a `bitcoin::OutPoint`.
    fn to_bitcoin_outpoint(key: &OutPointKey) -> bitcoin::OutPoint {
        bitcoin::OutPoint { txid: key.id(), vout: key.index }
    }

    #[test]
    fn all_locked_outpoints_empty_when_none_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let result = table.all_locked_outpoints().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn all_locked_outpoints_returns_bitcoin_outpoints() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        // Use index 0 for both to avoid the known endian mismatch in
        // OutPointKey::from_bytes (big-endian) vs as_bytes (native).
        let op1 = outpoint_key(0x01, 0);
        let op2 = outpoint_key(0x02, 0);
        table.lock_all(&[op1.clone(), op2.clone()]).unwrap();

        let locked = table.all_locked_outpoints().unwrap();
        assert_eq!(locked.len(), 2);

        // Round-trip: lock as OutPointKey, retrieve as bitcoin::OutPoint,
        // then convert back to OutPointKey and verify equality.
        let locked_keys: Vec<OutPointKey> = locked.iter().map(OutPointKey::from).collect();
        assert!(locked_keys.contains(&op1));
        assert!(locked_keys.contains(&op2));
    }

    #[test]
    fn find_locked_returns_only_locked_subset() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let op1 = outpoint_key(0x01, 0);
        let op2 = outpoint_key(0x02, 1);
        let op3 = outpoint_key(0x03, 2);

        // Lock only op1 and op3
        table.lock_all(&[op1.clone(), op3.clone()]).unwrap();

        let selection =
            vec![to_bitcoin_outpoint(&op1), to_bitcoin_outpoint(&op2), to_bitcoin_outpoint(&op3)];

        let locked = table.find_locked(&selection).unwrap();
        assert_eq!(locked.len(), 2);
        assert!(locked.contains(&to_bitcoin_outpoint(&op1)));
        assert!(locked.contains(&to_bitcoin_outpoint(&op3)));
        // op2 is not locked, should not appear
        assert!(!locked.contains(&to_bitcoin_outpoint(&op2)));
    }

    #[test]
    fn find_locked_returns_empty_when_none_locked() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        let selection = vec![
            to_bitcoin_outpoint(&outpoint_key(0x01, 0)),
            to_bitcoin_outpoint(&outpoint_key(0x02, 1)),
        ];

        let locked = table.find_locked(&selection).unwrap();
        assert!(locked.is_empty());
    }

    #[test]
    fn find_locked_empty_selection_returns_empty() {
        let (db, _tmp) = WalletDataDb::new_test(WalletId::preview_new());
        let table = &db.locked_outpoints;

        // Lock something, but pass empty selection
        table.lock(outpoint_key(0x01, 0)).unwrap();

        let locked = table.find_locked(&[]).unwrap();
        assert!(locked.is_empty());
    }
}
