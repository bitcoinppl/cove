use std::sync::{Mutex, OnceLock};
use tracing::{error, info};

/// Async bootstrap: initializes the tokio runtime, runs critical storage bootstrap
/// (encryption key derivation + redb migrations) on a blocking thread, then
/// attempts BDK migration. BDK migration failures are non-blocking — the app
/// continues with unencrypted BDK databases and retries on next launch.
///
/// Returns `Ok(None)` when everything succeeds, `Ok(Some(warning))` when BDK
/// migration failed but the app can continue, or `Err` for critical failures
#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap() -> Result<Option<String>, AppInitError> {
    cove_tokio::init();
    cove_tokio::unblock::run_blocking(|| {
        // critical bootstrap (cached via OnceLock)
        ensure_storage_bootstrapped()?;

        // BDK migration (not cached — retries every launch)
        let warning = match attempt_bdk_migration() {
            Ok(()) => None,
            Err(e) => {
                let msg = format!("{e:#}");
                error!("BDK migration failed (will retry next launch): {msg}");
                Some(msg)
            }
        };

        Ok(warning)
    })
    .await
}

fn attempt_bdk_migration() -> eyre::Result<()> {
    crate::database::migration::recover_interrupted_bdk_migrations()?;
    crate::database::migration::migrate_bdk_databases_if_needed()?;
    Ok(())
}

static STORAGE_BOOTSTRAPPED: OnceLock<()> = OnceLock::new();
static BOOTSTRAP_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi(flat_error)]
pub enum AppInitError {
    #[error("Failed to derive encryption key: {0}")]
    KeyDerivation(String),

    #[error("Main database migration failed: {0}")]
    MainDatabaseMigration(String),

    #[error("Wallet database migration failed: {0}")]
    WalletDatabaseMigration(String),
}

/// Idempotent storage bootstrap: derives encryption key and runs all pending
/// redb/wallet migrations. BDK migration is handled separately by `bootstrap()`.
///
/// Precondition: Keychain and Device must already be initialized via their FFI constructors.
/// Safe to call multiple times — only the first call performs work, subsequent calls
/// return `Ok(())` immediately. Failures are not cached, allowing retry on next call
pub fn ensure_storage_bootstrapped() -> Result<(), AppInitError> {
    if STORAGE_BOOTSTRAPPED.get().is_some() {
        return Ok(());
    }

    let _guard = BOOTSTRAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // double-check after acquiring lock
    if STORAGE_BOOTSTRAPPED.get().is_some() {
        return Ok(());
    }

    do_bootstrap()?;
    let _ = STORAGE_BOOTSTRAPPED.set(());
    Ok(())
}

fn do_bootstrap() -> Result<(), AppInitError> {
    crate::logging::init();
    info!("Starting storage bootstrap");

    // derive encryption key from master key before any database access
    let cspp = cove_cspp::Cspp::new(cove_device::keychain::Keychain::global().clone());
    let master_key =
        cspp.get_or_create_master_key().map_err(|e| AppInitError::KeyDerivation(e.to_string()))?;

    let key = master_key.sensitive_data_key();
    crate::database::encrypted_backend::set_encryption_key(key);

    info!("Encryption key derived and set");

    // recover interrupted redb migrations then migrate plaintext → encrypted
    crate::database::migration::recover_interrupted_migrations()
        .map_err(|e| AppInitError::MainDatabaseMigration(format!("{e:#}")))?;

    crate::database::migration::migrate_main_database_if_needed()
        .map_err(|e| AppInitError::MainDatabaseMigration(format!("{e:#}")))?;

    crate::database::migration::migrate_wallet_databases_if_needed()
        .map_err(|e| AppInitError::WalletDatabaseMigration(format!("{e:#}")))?;

    info!("Storage bootstrap complete");
    Ok(())
}

/// Pre-seed the bootstrap OnceLock with a test encryption key, skipping
/// keychain access and migrations
#[cfg(test)]
pub fn set_test_bootstrapped() {
    crate::database::encrypted_backend::set_test_encryption_key();
    let _ = STORAGE_BOOTSTRAPPED.set(());
}
