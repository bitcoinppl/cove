use super::{
    global_config::GlobalConfigTableError, global_flag::GlobalFlagTableError,
    wallet::WalletTableError,
};

type Error = DatabaseError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum DatabaseError {
    #[error("failed to open database: {0}")]
    DatabaseAccessError(String),

    #[error("failed to open table: {0}")]
    TableAccessError(String),

    #[error(transparent)]
    WalletsError(#[from] WalletTableError),

    #[error(transparent)]
    GlobalFlagError(#[from] GlobalFlagTableError),

    #[error(transparent)]
    GlobalConfigError(#[from] GlobalConfigTableError),
}

impl From<redb::TransactionError> for Error {
    fn from(error: redb::TransactionError) -> Self {
        Self::DatabaseAccessError(error.to_string())
    }
}

impl From<redb::TableError> for Error {
    fn from(error: redb::TableError) -> Self {
        Self::TableAccessError(error.to_string())
    }
}

impl From<redb::StorageError> for Error {
    fn from(error: redb::StorageError) -> Self {
        Self::TableAccessError(error.to_string())
    }
}
