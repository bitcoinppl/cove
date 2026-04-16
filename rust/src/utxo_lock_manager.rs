use std::sync::Arc;

use cove_types::{OutPoint, lock_state::LockState};

use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletId};

#[derive(Debug, Clone, uniffi::Object)]
pub struct UtxoLockManager {
    db: WalletDataDb,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum UtxoLockManagerError {
    #[error("failed to open wallet database: {0}")]
    DatabaseOpen(String),

    #[error("lock operation failed: {0}")]
    LockFailed(String),

    #[error("unlock operation failed: {0}")]
    UnlockFailed(String),

    #[error("query failed: {0}")]
    QueryFailed(String),
}

type Error = UtxoLockManagerError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl UtxoLockManager {
    #[uniffi::constructor]
    pub fn new(id: WalletId) -> std::result::Result<Self, Error> {
        let db = WalletDataDb::new_or_existing(id)
            .map_err(|e| Error::DatabaseOpen(e.to_string()))?;

        Ok(Self { db })
    }

    /// Lock a single outpoint.
    pub fn lock_outpoint(&self, outpoint: Arc<OutPoint>) -> Result<()> {
        let bitcoin_op: bitcoin::OutPoint = outpoint.as_ref().into();
        self.db
            .locked_outpoints
            .lock(&bitcoin_op)
            .map_err(|e| Error::LockFailed(e.to_string()))
    }

    /// Unlock a single outpoint. No-op if it was already unlocked.
    pub fn unlock_outpoint(&self, outpoint: Arc<OutPoint>) -> Result<()> {
        let bitcoin_op: bitcoin::OutPoint = outpoint.as_ref().into();
        self.db
            .locked_outpoints
            .unlock(&bitcoin_op)
            .map_err(|e| Error::UnlockFailed(e.to_string()))
    }

    /// Check whether a single outpoint is locked.
    pub fn is_locked(&self, outpoint: Arc<OutPoint>) -> Result<bool> {
        let bitcoin_op: bitcoin::OutPoint = outpoint.as_ref().into();
        self.db
            .locked_outpoints
            .is_locked(&bitcoin_op)
            .map_err(|e| Error::QueryFailed(e.to_string()))
    }

    /// Compute the aggregate lock state over a set of outpoints.
    ///
    /// The caller passes in only the wallet-owned unspent outputs they care
    /// about (e.g. the unspent outputs created by a particular transaction).
    ///
    /// Returns `Unlocked` for an empty set.
    pub fn aggregate_lock_state(&self, outpoints: Vec<Arc<OutPoint>>) -> Result<LockState> {
        if outpoints.is_empty() {
            return Ok(LockState::Unlocked);
        }

        let mut locked_count: usize = 0;

        for op in &outpoints {
            let bitcoin_op: bitcoin::OutPoint = op.as_ref().into();
            let is_locked = self
                .db
                .locked_outpoints
                .is_locked(&bitcoin_op)
                .map_err(|e| Error::QueryFailed(e.to_string()))?;

            if is_locked {
                locked_count += 1;
            }
        }

        let state = if locked_count == 0 {
            LockState::Unlocked
        } else if locked_count == outpoints.len() {
            LockState::Locked
        } else {
            LockState::Mixed
        };

        Ok(state)
    }

    /// Lock all outpoints in the set within a single transaction.
    pub fn lock_all(&self, outpoints: Vec<Arc<OutPoint>>) -> Result<()> {
        let bitcoin_ops: Vec<bitcoin::OutPoint> =
            outpoints.iter().map(|op| op.as_ref().into()).collect();

        self.db
            .locked_outpoints
            .lock_all(&bitcoin_ops)
            .map_err(|e| Error::LockFailed(e.to_string()))
    }

    /// Unlock all outpoints in the set within a single transaction.
    pub fn unlock_all(&self, outpoints: Vec<Arc<OutPoint>>) -> Result<()> {
        let bitcoin_ops: Vec<bitcoin::OutPoint> =
            outpoints.iter().map(|op| op.as_ref().into()).collect();

        self.db
            .locked_outpoints
            .unlock_all(&bitcoin_ops)
            .map_err(|e| Error::UnlockFailed(e.to_string()))
    }

    /// Execute the Transaction Details 3-state bulk toggle:
    ///
    /// - `Unlocked` → lock all → returns `Locked`
    /// - `Mixed`    → lock remaining unlocked → returns `Locked`
    /// - `Locked`   → unlock all → returns `Unlocked`
    ///
    /// `outpoints` should be the wallet-owned unspent outputs for a transaction.
    pub fn toggle_lock_state(&self, outpoints: Vec<Arc<OutPoint>>) -> Result<LockState> {
        let current = self.aggregate_lock_state(outpoints.clone())?;

        match current {
            LockState::Unlocked | LockState::Mixed => {
                self.lock_all(outpoints)?;
                Ok(LockState::Locked)
            }
            LockState::Locked => {
                self.unlock_all(outpoints)?;
                Ok(LockState::Unlocked)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::wallet_data::WalletDataDb;
    use std::str::FromStr;

    fn make_outpoint(vout: u32) -> Arc<OutPoint> {
        let txid = bitcoin::Txid::from_str(
            "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290",
        )
        .unwrap();

        Arc::new(OutPoint::from(bitcoin::OutPoint { txid, vout }))
    }

    fn make_manager() -> (UtxoLockManager, tempfile::TempDir) {
        let id = WalletId::new();
        let (db, tmp) = WalletDataDb::new_test(id);
        (UtxoLockManager { db }, tmp)
    }

    #[test]
    fn test_lock_and_query() {
        let (mgr, _tmp) = make_manager();
        let op = make_outpoint(0);

        assert!(!mgr.is_locked(op.clone()).unwrap());
        mgr.lock_outpoint(op.clone()).unwrap();
        assert!(mgr.is_locked(op).unwrap());
    }

    #[test]
    fn test_unlock() {
        let (mgr, _tmp) = make_manager();
        let op = make_outpoint(1);

        mgr.lock_outpoint(op.clone()).unwrap();
        mgr.unlock_outpoint(op.clone()).unwrap();
        assert!(!mgr.is_locked(op).unwrap());
    }

    #[test]
    fn test_aggregate_empty() {
        let (mgr, _tmp) = make_manager();
        assert_eq!(mgr.aggregate_lock_state(vec![]).unwrap(), LockState::Unlocked);
    }

    #[test]
    fn test_aggregate_unlocked() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();
        assert_eq!(mgr.aggregate_lock_state(ops).unwrap(), LockState::Unlocked);
    }

    #[test]
    fn test_aggregate_locked() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();

        mgr.lock_all(ops.clone()).unwrap();
        assert_eq!(mgr.aggregate_lock_state(ops).unwrap(), LockState::Locked);
    }

    #[test]
    fn test_aggregate_mixed() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();

        // lock only the first one
        mgr.lock_outpoint(ops[0].clone()).unwrap();
        assert_eq!(mgr.aggregate_lock_state(ops).unwrap(), LockState::Mixed);
    }

    #[test]
    fn test_toggle_from_unlocked() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();

        let new_state = mgr.toggle_lock_state(ops.clone()).unwrap();
        assert_eq!(new_state, LockState::Locked);

        // verify all are locked
        for op in &ops {
            assert!(mgr.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn test_toggle_from_mixed() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();

        // lock only the first
        mgr.lock_outpoint(ops[0].clone()).unwrap();

        let new_state = mgr.toggle_lock_state(ops.clone()).unwrap();
        assert_eq!(new_state, LockState::Locked);

        for op in &ops {
            assert!(mgr.is_locked(op.clone()).unwrap());
        }
    }

    #[test]
    fn test_toggle_from_locked() {
        let (mgr, _tmp) = make_manager();
        let ops: Vec<_> = (0..3).map(make_outpoint).collect();

        mgr.lock_all(ops.clone()).unwrap();

        let new_state = mgr.toggle_lock_state(ops.clone()).unwrap();
        assert_eq!(new_state, LockState::Unlocked);

        for op in &ops {
            assert!(!mgr.is_locked(op.clone()).unwrap());
        }
    }
}
