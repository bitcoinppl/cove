mod v1_bdk;
mod v1_redb;

use std::path::Path;
use tracing::warn;

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

pub use v1_bdk::{
    BdkMigration, count_bdk_databases_needing_migration, is_plaintext_sqlite,
    recover_interrupted_bdk_migrations,
};

pub use v1_redb::{
    WalletMigration, count_redb_wallets_needing_migration, main_database_needs_migration,
    migrate_main_database_if_needed, recover_interrupted_migrations,
};
