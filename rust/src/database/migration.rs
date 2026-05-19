mod bdk;
mod redb;

use std::path::Path;
use tracing::warn;

#[derive(Debug, Clone, derive_more::Display)]
#[display("{db_path}: {error}")]
pub(crate) struct MigrationFailure {
    pub db_path: String,
    pub error: String,
}

pub fn log_remove_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            let path = path.display();
            warn!("Failed to remove {path}: {e}");
        }
    }
}

pub use bdk::{
    BdkMigration, count_bdk_databases_needing_migration, is_plaintext_sqlite,
    recover_interrupted_bdk_migrations,
};

pub(crate) use redb::{HISTORICAL_MAIN_REDB_TABLES, HISTORICAL_WALLET_REDB_TABLES};
pub use redb::{
    WalletMigration, count_redb_wallets_needing_migration, known_wallet_ids_from_main_database,
    main_database_needs_migration, migrate_main_database_if_needed,
    recover_interrupted_main_migration, recover_interrupted_wallet_migrations,
};
