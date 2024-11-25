use std::{cmp::Ordering, sync::Arc};

use bitcoin_hashes::{sha256d::Hash, Hash as _};
use redb::TableDefinition;

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

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnsignedTransactionRecord {
    pub wallet_id: WalletId,
    pub tx_id: TxId,
    pub confirm_details: ConfirmDetails,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct UnsignedTransactionsTable {
    db: Arc<redb::Database>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum UnsignedTransactionsTableError {
    #[error("failed to save unconfirmed transaction: {0}")]
    Save(String),

    #[error("failed to get unconfirmed transaction: {0}")]
    Read(String),
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

    pub fn get(&self, key: TxId) -> Result<Option<UnsignedTransactionRecord>, Error> {
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

    pub fn set(&self, key: TxId, value: UnsignedTransactionRecord) -> Result<(), Error> {
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
}

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
        todo!()
        // TypeName::new(&format!("Bincode<{}>", type_name::<T>()))
    }
}
