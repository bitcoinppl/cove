use redb::{ReadableTable as _, TableDefinition};
use std::sync::Arc;
use tracing::debug;

use cove_util::result_ext::ResultExt as _;

use super::Error;

use crate::transaction::TxId;
use cove_types::WalletId;
use cove_types::confirm::ConfirmDetails;
use cove_types::redb::Json;

pub const MAIN_TABLE: TableDefinition<TxId, Json<UnsignedTransactionRecord>> =
    TableDefinition::new("unsigned_transactions");

pub const BY_WALLET_TABLE: TableDefinition<WalletId, Vec<TxId>> =
    TableDefinition::new("unsigned_transactions_by_wallet");

#[derive(Debug, Clone, uniffi::Object)]
pub struct UnsignedTransactionsTable {
    db: Arc<redb::Database>,
}

type Result<T, E = UnsignedTransactionsTableError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum UnsignedTransactionsTableError {
    #[error("failed to save unconfirmed transaction: {0}")]
    Save(String),

    #[error("failed to get unconfirmed transaction: {0}")]
    Read(String),

    #[error("no record found")]
    NoRecordFound,
}

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Object,
)]
pub struct UnsignedTransactionRecord {
    pub wallet_id: WalletId,
    pub tx_id: TxId,
    pub confirm_details: ConfirmDetails,
    pub created_at: u64,
}

impl UnsignedTransactionsTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(MAIN_TABLE).expect("failed to create table");

        write_txn.open_table(BY_WALLET_TABLE).expect("failed to create table");

        Self { db }
    }

    pub fn get_tx(&self, tx_id: &TxId) -> Result<Option<UnsignedTransactionRecord>, Error> {
        self.get(tx_id)
    }

    /// Save a record and add it to its wallet's index in one write transaction
    ///
    /// Wallet actor pruning deletes records concurrently with send flow saves, so the index
    /// read and write must not span separate transactions
    pub fn save_tx(&self, tx_id: TxId, record: UnsignedTransactionRecord) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        {
            let mut by_wallet =
                write_txn.open_table(BY_WALLET_TABLE).map_err_str(Error::TableAccess)?;

            let mut wallet_tx_ids = by_wallet
                .get(&record.wallet_id)
                .map_err_str(UnsignedTransactionsTableError::Read)?
                .map(|value| value.value())
                .unwrap_or_default();

            if !wallet_tx_ids.contains(&tx_id) {
                wallet_tx_ids.push(tx_id);
            }

            by_wallet
                .insert(&record.wallet_id, wallet_tx_ids)
                .map_err_str(UnsignedTransactionsTableError::Save)?;
        }

        {
            let mut main = write_txn.open_table(MAIN_TABLE).map_err_str(Error::TableAccess)?;
            main.insert(tx_id, record).map_err_str(UnsignedTransactionsTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    /// Delete a record and remove it from its wallet's index in one write transaction
    pub fn delete_tx(&self, tx_id: &TxId) -> Result<UnsignedTransactionRecord, Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        let removed = {
            let mut main = write_txn.open_table(MAIN_TABLE).map_err_str(Error::TableAccess)?;

            main.remove(tx_id)
                .map_err_str(UnsignedTransactionsTableError::Save)?
                .map(|value| value.value())
        };

        let Some(record) = removed else {
            write_txn.abort().map_err_str(Error::DatabaseAccess)?;
            return Err(UnsignedTransactionsTableError::NoRecordFound.into());
        };

        {
            let mut by_wallet =
                write_txn.open_table(BY_WALLET_TABLE).map_err_str(Error::TableAccess)?;

            let mut wallet_tx_ids = by_wallet
                .get(&record.wallet_id)
                .map_err_str(UnsignedTransactionsTableError::Read)?
                .map(|value| value.value())
                .unwrap_or_default();

            wallet_tx_ids.retain(|id| id != tx_id);
            by_wallet
                .insert(&record.wallet_id, wallet_tx_ids)
                .map_err_str(UnsignedTransactionsTableError::Save)?;
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(record)
    }

    pub fn get_by_wallet_id(
        &self,
        key: &WalletId,
    ) -> Result<Vec<UnsignedTransactionRecord>, Error> {
        let ids = self.get_tx_ids_for_wallet_id(key)?;

        let records = ids
            .into_iter()
            .map(|id| self.get(&id))
            .filter_map(|record| match record {
                Ok(Some(record)) => Some(Ok(record)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<UnsignedTransactionRecord>, _>>()?;

        Ok(records)
    }

    /// Remove every unsigned transaction that belongs to a wallet
    ///
    /// Wallet deletion owns this call so PSBT data does not outlive its wallet
    pub(crate) fn delete_by_wallet_id(&self, wallet_id: &WalletId) -> Result<(), Error> {
        let write_txn = self.db.begin_write().map_err_str(Error::DatabaseAccess)?;

        let removed_tx_ids = {
            let mut by_wallet =
                write_txn.open_table(BY_WALLET_TABLE).map_err_str(Error::TableAccess)?;

            by_wallet
                .remove(wallet_id)
                .map_err_str(UnsignedTransactionsTableError::Save)?
                .map(|value| value.value())
        };

        let Some(tx_ids) = removed_tx_ids else {
            write_txn.abort().map_err_str(Error::DatabaseAccess)?;
            return Ok(());
        };

        {
            let mut main = write_txn.open_table(MAIN_TABLE).map_err_str(Error::TableAccess)?;
            for tx_id in &tx_ids {
                main.remove(tx_id).map_err_str(UnsignedTransactionsTableError::Save)?;
            }
        }

        write_txn.commit().map_err_str(Error::DatabaseAccess)?;

        Ok(())
    }

    fn get(&self, key: &TxId) -> Result<Option<UnsignedTransactionRecord>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(MAIN_TABLE).map_err_str(Error::TableAccess)?;

        let value = table
            .get(key)
            .map_err_str(UnsignedTransactionsTableError::Read)?
            .map(|value| value.value());

        Ok(value)
    }

    fn get_tx_ids_for_wallet_id(&self, key: &WalletId) -> Result<Vec<TxId>, Error> {
        let read_txn = self.db.begin_read().map_err_str(Error::DatabaseAccess)?;

        let table = read_txn.open_table(BY_WALLET_TABLE).map_err_str(Error::TableAccess)?;

        let ids = table
            .get(key)
            .map_err_str(UnsignedTransactionsTableError::Read)?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(ids)
    }
}

// MARK: uniffi impls
#[uniffi::export]
impl UnsignedTransactionsTable {
    #[uniffi::method(name = "getTx")]
    pub fn _get_tx(&self, tx_id: Arc<TxId>) -> Option<Arc<UnsignedTransactionRecord>> {
        debug!("getTx: {tx_id:?}");
        self.get(&tx_id).ok().flatten().map(Arc::new)
    }

    #[uniffi::method(name = "getTxThrow")]
    pub fn _get_tx_throw(&self, tx_id: Arc<TxId>) -> Result<Arc<UnsignedTransactionRecord>> {
        debug!("getTxThrow: {tx_id:?}");
        self.get(&tx_id)
            .map_err_str(UnsignedTransactionsTableError::Read)?
            .ok_or(UnsignedTransactionsTableError::NoRecordFound)
            .map(Arc::new)
    }
}

#[uniffi::export]
impl UnsignedTransactionRecord {
    #[uniffi::method]
    pub fn wallet_id(&self) -> WalletId {
        self.wallet_id.clone()
    }

    #[uniffi::method]
    pub const fn tx_id(&self) -> TxId {
        self.tx_id
    }

    #[uniffi::method]
    pub fn confirm_details(&self) -> ConfirmDetails {
        self.confirm_details.clone()
    }

    #[uniffi::method]
    pub const fn created_at(&self) -> u64 {
        self.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::{UnsignedTransactionRecord, UnsignedTransactionsTable};
    use crate::transaction::TxId;
    use cove_types::WalletId;

    fn test_table() -> (tempfile::TempDir, UnsignedTransactionsTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = UnsignedTransactionsTable::new(db, &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }

    fn record(wallet_id: &WalletId) -> UnsignedTransactionRecord {
        UnsignedTransactionRecord {
            wallet_id: wallet_id.clone(),
            tx_id: TxId::preview_new(),
            confirm_details: cove_types::confirm::confirm_details_preview_new(),
            created_at: 0,
        }
    }

    #[test]
    fn delete_tx_removes_the_record_and_its_wallet_index_entry() {
        let (_tmp, table) = test_table();
        let wallet_id = WalletId::preview_new_random();
        let deleted = record(&wallet_id);
        let kept = record(&wallet_id);

        table.save_tx(deleted.tx_id, deleted.clone()).unwrap();
        table.save_tx(kept.tx_id, kept.clone()).unwrap();

        assert_eq!(table.delete_tx(&deleted.tx_id).unwrap(), deleted);
        assert_eq!(table.get_by_wallet_id(&wallet_id).unwrap(), vec![kept]);
        assert!(table.delete_tx(&deleted.tx_id).is_err());
    }

    #[test]
    fn delete_by_wallet_id_removes_only_that_wallets_records() {
        let (_tmp, table) = test_table();
        let deleted = WalletId::preview_new_random();
        let kept = WalletId::preview_new_random();

        let deleted_records = [record(&deleted), record(&deleted)];
        for record in &deleted_records {
            table.save_tx(record.tx_id, record.clone()).unwrap();
        }
        let kept_record = record(&kept);
        table.save_tx(kept_record.tx_id, kept_record.clone()).unwrap();

        table.delete_by_wallet_id(&deleted).unwrap();

        assert!(table.get_by_wallet_id(&deleted).unwrap().is_empty());
        for record in &deleted_records {
            assert_eq!(table.get_tx(&record.tx_id).unwrap(), None);
        }
        assert_eq!(table.get_by_wallet_id(&kept).unwrap(), vec![kept_record.clone()]);
        assert_eq!(table.get_tx(&kept_record.tx_id).unwrap(), Some(kept_record));
    }

    #[test]
    fn delete_by_wallet_id_is_a_no_op_for_unknown_wallets() {
        let (_tmp, table) = test_table();
        let kept = WalletId::preview_new_random();
        let kept_record = record(&kept);
        table.save_tx(kept_record.tx_id, kept_record.clone()).unwrap();

        table.delete_by_wallet_id(&WalletId::preview_new_random()).unwrap();

        assert_eq!(table.get_by_wallet_id(&kept).unwrap(), vec![kept_record.clone()]);
        assert_eq!(table.get_tx(&kept_record.tx_id).unwrap(), Some(kept_record));
    }
}
