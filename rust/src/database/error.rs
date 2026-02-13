use super::{
    global_cache::GlobalCacheTableError, global_config::GlobalConfigTableError,
    global_flag::GlobalFlagTableError, historical_price::HistoricalPriceTableError,
    unsigned_transactions::UnsignedTransactionsTableError, wallet::WalletTableError,
};

type Error = DatabaseError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum SerdeError {
    #[error("failed to serialize: {0}")]
    SerializationError(String),

    #[error("failed to deserialize: {0}")]
    DeserializationError(String),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum DatabaseError {
    #[error("failed to open database: {0}")]
    DatabaseAccess(String),

    #[error("failed to open table: {0}")]
    TableAccess(String),

    #[error(transparent)]
    Wallets(#[from] WalletTableError),

    #[error(transparent)]
    GlobalFlag(#[from] GlobalFlagTableError),

    #[error(transparent)]
    GlobalConfig(#[from] GlobalConfigTableError),

    #[error(transparent)]
    GlobalCache(#[from] GlobalCacheTableError),

    #[error(transparent)]
    UnsignedTransactions(#[from] UnsignedTransactionsTableError),

    #[error(transparent)]
    HistoricalPrice(#[from] HistoricalPriceTableError),

    #[error("unable to serialize or deserialize: {0}")]
    Serialization(#[from] SerdeError),

    #[error("wallet not found")]
    WalletNotFound,
}

impl From<redb::TransactionError> for Error {
    fn from(error: redb::TransactionError) -> Self {
        Self::DatabaseAccess(error.to_string())
    }
}

impl From<redb::TableError> for Error {
    fn from(error: redb::TableError) -> Self {
        Self::TableAccess(error.to_string())
    }
}

impl From<redb::StorageError> for Error {
    fn from(error: redb::StorageError) -> Self {
        Self::TableAccess(error.to_string())
    }
}
