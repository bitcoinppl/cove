use std::{cmp::Ordering, sync::Arc};

use bitcoin_hashes::{sha256d::Hash, Hash as _};
use redb::TableDefinition;

use crate::{
    redb::Json,
    transaction::{unsigned_transaction::UnsignedTransaction, TxId},
    wallet::metadata::WalletId,
};

use super::Error;

pub const MAIN_TABLE: TableDefinition<TxId, Json<UnsignedTransaction>> =
    TableDefinition::new("unsigned_transactions");

pub const BY_WALLET_TABLE: TableDefinition<WalletId, Vec<TxId>> =
    TableDefinition::new("unsigned_transactions_by_wallet");

#[derive(Debug, Clone, uniffi::Object)]
pub struct UnsignedTransactionsTable {
    db: Arc<redb::Database>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum UnsignedTransactionsTableError {
    #[error("failed to save unconfirmed transaction: {0}")]
    Save(String),

    #[error("failed to get unconfirmed transaction: {0}")]
    Read(String),

    #[error("no record found")]
    NoRecordFound,
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

    pub fn get_tx(&self, tx_id: &TxId) -> Result<Option<UnsignedTransaction>, Error> {
        self.get(tx_id)
    }

    pub fn save_tx(&self, tx_id: TxId, record: UnsignedTransaction) -> Result<(), Error> {
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

    pub fn delete_tx(&self, tx_id: &TxId) -> Result<(), Error> {
        let record = self
            .get(tx_id)?
            .ok_or(UnsignedTransactionsTableError::NoRecordFound)?;

        // remove the tx id from the wallet
        {
            let wallet_id = &record.wallet_id;
            let mut wallet_tx_ids = self.get_tx_ids_for_wallet_id(wallet_id)?;
            wallet_tx_ids.retain(|id| &id != &tx_id);
            self.set_by_wallet_id(wallet_id.clone(), wallet_tx_ids)?;
        }

        // delete the actual tx id
        self.delete_tx_id(tx_id)?;

        Ok(())
    }

    pub fn get_by_wallet_id(&self, key: &WalletId) -> Result<Vec<UnsignedTransaction>, Error> {
        let ids = self.get_tx_ids_for_wallet_id(key)?;

        let records = ids
            .into_iter()
            .map(|id| self.get(&id))
            .filter_map(|record| match record {
                Ok(Some(record)) => Some(Ok(record)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<UnsignedTransaction>, _>>()?;

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

    fn get(&self, key: &TxId) -> Result<Option<UnsignedTransaction>, Error> {
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

    pub fn set(&self, key: TxId, value: UnsignedTransaction) -> Result<(), Error> {
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

// MARK: redb serd/de impls

impl redb::Key for TxId {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for TxId {
    type SelfType<'a> = TxId
           where Self: 'a;

    type AsBytes<'a> = Vec<u8>
        where Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let hash = Hash::from_slice(data).unwrap();
        let txid = bitcoin::Txid::from_raw_hash(hash);
        txid.into()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        let hash: &Hash = value.0.as_raw_hash();
        let bytes: &[u8] = hash.as_ref();

        bytes.to_vec()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new(&format!("{}", std::any::type_name::<TxId>()))
    }
}
