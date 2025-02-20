use redb::TableDefinition;
use std::sync::Arc;
use tracing::debug;

use crate::{
    redb::Json,
    transaction::TxId,
    wallet::{confirm::ConfirmDetails, metadata::WalletId},
};

use super::Error;

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
        write_txn
            .open_table(MAIN_TABLE)
            .expect("failed to create table");

        write_txn
            .open_table(BY_WALLET_TABLE)
            .expect("failed to create table");

        Self { db }
    }

    pub fn get_tx(&self, tx_id: &TxId) -> Result<Option<UnsignedTransactionRecord>, Error> {
        self.get(tx_id)
    }

    pub fn save_tx(&self, tx_id: TxId, record: UnsignedTransactionRecord) -> Result<(), Error> {
        let wallet_id = record.wallet_id.clone();

        // get all the tx ids for the wallet
        let mut wallet_tx_ids = self.get_tx_ids_for_wallet_id(&wallet_id)?;

        // save the tx id for the wallet
        wallet_tx_ids.push(tx_id);

        // save the wallet tx ids
        self.set_by_wallet_id(wallet_id, wallet_tx_ids)?;

        // save the tx id
        self.set(tx_id, record)?;

        Ok(())
    }

    pub fn delete_tx(&self, tx_id: &TxId) -> Result<UnsignedTransactionRecord, Error> {
        let record = self
            .get(tx_id)?
            .ok_or(UnsignedTransactionsTableError::NoRecordFound)?;

        // remove the tx id from the wallet
        {
            let wallet_id = &record.wallet_id;
            let mut wallet_tx_ids = self.get_tx_ids_for_wallet_id(wallet_id)?;
            wallet_tx_ids.retain(|id| id != tx_id);
            self.set_by_wallet_id(wallet_id.clone(), wallet_tx_ids)?;
        }

        // delete the actual tx id
        self.delete_tx_id(tx_id)?;

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

    fn delete_tx_id(&self, key: &TxId) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(MAIN_TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            table
                .remove(key)
                .map_err(|error| UnsignedTransactionsTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    fn get(&self, key: &TxId) -> Result<Option<UnsignedTransactionRecord>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(MAIN_TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let value = table
            .get(key)
            .map_err(|error| UnsignedTransactionsTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn get_tx_ids_for_wallet_id(&self, key: &WalletId) -> Result<Vec<TxId>, Error> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(BY_WALLET_TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let ids = table
            .get(key)
            .map_err(|error| UnsignedTransactionsTableError::Read(error.to_string()))?
            .map(|value| value.value())
            .unwrap_or_default();

        Ok(ids)
    }

    fn set(&self, key: TxId, value: UnsignedTransactionRecord) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(MAIN_TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            table
                .insert(key, value)
                .map_err(|error| UnsignedTransactionsTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Ok(())
    }

    fn set_by_wallet_id(&self, key: WalletId, value: Vec<TxId>) -> Result<(), Error> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(BY_WALLET_TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            table
                .insert(key, value)
                .map_err(|error| UnsignedTransactionsTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Ok(())
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
            .map_err(|e| UnsignedTransactionsTableError::Read(e.to_string()))?
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
    pub fn tx_id(&self) -> TxId {
        self.tx_id
    }

    #[uniffi::method]
    pub fn confirm_details(&self) -> ConfirmDetails {
        self.confirm_details.clone()
    }

    #[uniffi::method]
    pub fn created_at(&self) -> u64 {
        self.created_at
    }
}
