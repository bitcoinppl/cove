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

    #[error("encryption key not set before database access")]
    EncryptionKeyNotSet,

    #[error("storage bootstrap failed: {0}")]
    BootstrapFailed(String),

    #[error("failed to open encrypted backend at {path}: {error}")]
    BackendOpen { path: String, error: String },

    #[error("truncated or corrupt encrypted block at {path}: {error}")]
    CorruptBlock { path: String, error: String },

    #[error("database is already open by another process")]
    DatabaseAlreadyOpen,

    #[error("header integrity check failed at {path}: {error}")]
    HeaderIntegrity { path: String, error: String },

    #[error("database at {path} is not encrypted; migration may not have completed")]
    PlaintextNotAllowed { path: String },
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

impl From<redb::CommitError> for Error {
    fn from(error: redb::CommitError) -> Self {
        Self::DatabaseAccess(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::DatabaseAccess(error.to_string())
    }
}
