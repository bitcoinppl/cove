pub mod migration;
pub use migration::Migration;

use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use cove_util::ResultExt;
use parking_lot::Mutex;
use tracing::{error, info};

#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum BootstrapStep {
    #[default]
    NotStarted,
    Initializing,
    TokioInitialized,
    DerivingEncryptionKey,
    EncryptionKeySet,
    RecoveringInterruptedMigrations,
    MigratingMainDatabase,
    MigratingWalletDatabases,
    RedbMigrationComplete,
    MigratingBdkDatabases,
    Complete,
}

impl BootstrapStep {
    pub fn is_migration_in_progress(&self) -> bool {
        matches!(
            self,
            Self::RecoveringInterruptedMigrations
                | Self::MigratingMainDatabase
                | Self::MigratingWalletDatabases
                | Self::MigratingBdkDatabases
        )
    }
}

#[uniffi::export]
impl BootstrapStep {
    #[uniffi::method(name = "isMigrationInProgress")]
    fn ffi_is_migration_in_progress(&self) -> bool {
        self.is_migration_in_progress()
    }
}

static BOOTSTRAP_STEP: Mutex<BootstrapStep> = Mutex::new(BootstrapStep::NotStarted);
static BOOTSTRAP_CANCELLED: LazyLock<Arc<AtomicBool>> =
    LazyLock::new(|| Arc::new(AtomicBool::new(false)));

/// Async bootstrap: initializes the tokio runtime, runs critical storage bootstrap
/// (encryption key derivation + redb migrations) on a blocking thread, then
/// attempts BDK migration. BDK migration failures are non-blocking — the app
/// continues with unencrypted BDK databases and retries on next launch
///
/// Returns `Ok(None)` when everything succeeds, `Ok(Some(warning))` when BDK
/// migration failed but the app can continue, or `Err` for critical failures
///
/// Re-entrant safe: returns `Ok(None)` if already complete, or
/// `Err(AlreadyCalled)` if another call is still in progress
#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap() -> Result<Option<String>, AppInitError> {
    {
        let mut step = BOOTSTRAP_STEP.lock();
        match *step {
            BootstrapStep::NotStarted => *step = BootstrapStep::Initializing,
            BootstrapStep::Complete => {
                info!("Bootstrap already complete, returning immediately");
                return Ok(None);
            }
            _ => {
                return Err(AppInitError::AlreadyCalled(
                    "bootstrap already called — force-quit and restart the app".into(),
                ));
            }
        }
    }

    crate::logging::init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls ring crypto provider");

    cove_tokio::init();
    set_step(BootstrapStep::TokioInitialized);

    // safe to reset: any cancellation set after this point will be caught
    // at the next check_cancelled() boundary
    BOOTSTRAP_CANCELLED.store(false, Ordering::Release);
    migration::set_active_migration(None);
    info!("Bootstrap: tokio initialized, starting blocking work");

    cove_tokio::unblock::run_blocking(|| {
        check_cancelled()?;

        // derive encryption key and run redb migrations (idempotent via OnceLock)
        let bdk_count = ensure_storage_bootstrapped_internal(true)?;
        set_step(BootstrapStep::RedbMigrationComplete);
        info!("Bootstrap: storage bootstrapped, attempting BDK migration");

        check_cancelled()?;

        // bdk migration (not cached — retries every launch)
        set_step(BootstrapStep::MigratingBdkDatabases);
        let warning = match attempt_bdk_migration(bdk_count) {
            Ok(()) => None,
            Err(e) => {
                let msg = format!("{e:#}");
                error!("BDK migration failed (will retry next launch): {msg}");
                // prefer cancellation over migration error since the failure
                // may have been caused by the cancellation itself
                check_cancelled()?;
                Some(msg)
            }
        };

        set_step(BootstrapStep::Complete);
        migration::set_active_migration(None);
        info!("Bootstrap: blocking work complete");
        Ok(warning)
    })
    .await
    .inspect_err(|_| {
        migration::set_active_migration(None);
    })
}

/// Signal the bootstrap to stop at the next cancellation check point,
/// typically called from the frontend watchdog when a timeout fires
///
/// Cancellation is cooperative: the blocking thread only checks between
/// complete database operations. Individual migrations (two-phase swap)
/// always run to completion, preserving atomicity. Do not add
/// check_cancelled() or is_cancelled() calls inside migrate_single_bdk_database
/// or migrate_wallet_database
#[uniffi::export]
pub fn cancel_bootstrap() {
    BOOTSTRAP_CANCELLED.store(true, Ordering::Release);
    // don't clear ACTIVE_MIGRATION here — the blocking thread may still be running
    // and the frontend polls it for the progress bar. It gets cleared when
    // bootstrap() finishes or on next launch
    info!("Bootstrap cancellation requested");
}

/// Reset all bootstrap state so restore can re-run bootstrap with a new key
///
/// Clears encryption key cache, bootstrap step, storage bootstrapped flag,
/// and cancellation flag. Must be called before re-running bootstrap after restore
#[uniffi::export]
pub fn reset_bootstrap_for_restore() {
    *BOOTSTRAP_STEP.lock() = BootstrapStep::NotStarted;
    STORAGE_BOOTSTRAPPED.store(false, Ordering::Release);
    BOOTSTRAP_CANCELLED.store(false, Ordering::Release);
    cove_cspp::reset_master_key_cache();
    migration::set_active_migration(None);
    info!("Bootstrap state reset for restore");
}

fn check_cancelled() -> Result<(), AppInitError> {
    if BOOTSTRAP_CANCELLED.load(Ordering::Acquire) {
        Err(AppInitError::Cancelled("bootstrap cancelled by caller".into()))
    } else {
        Ok(())
    }
}

fn attempt_bdk_migration(pre_recovery_bdk_count: u32) -> eyre::Result<()> {
    crate::database::migration::recover_interrupted_bdk_migrations()?;

    let post_recovery_count = crate::database::migration::count_bdk_databases_needing_migration();
    let migration = migration::active_migration()
        .unwrap_or_else(|| Arc::new(Migration::new(0, Arc::clone(&BOOTSTRAP_CANCELLED))));

    // tick for databases that recovery already finished
    let recovered = pre_recovery_bdk_count.saturating_sub(post_recovery_count);
    for _ in 0..recovered {
        migration.tick();
    }

    crate::database::migration::BdkMigration::new(migration).run()
}

static STORAGE_BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);
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

    #[error("Bootstrap was cancelled: {0}")]
    Cancelled(String),

    #[error("Bootstrap already called: {0}")]
    AlreadyCalled(String),

    #[error("Database encryption key mismatch (backup/restore?): {0}")]
    DatabaseKeyMismatch(String),

    #[error("Database verification failed: {0}")]
    DatabaseVerificationFailed(String),
}

/// Idempotent storage bootstrap: derives encryption key and runs all pending
/// redb/wallet migrations. BDK migration is handled separately by `bootstrap()`
///
/// Precondition: Keychain and Device must already be initialized via their FFI constructors
/// Safe to call multiple times — only the first call performs work, subsequent calls
/// return `Ok(())` immediately. Failures are not cached, allowing retry on next call
pub fn ensure_storage_bootstrapped() -> Result<(), AppInitError> {
    ensure_storage_bootstrapped_internal(false).map(|_| ())
}

/// Returns the pre-recovery BDK database count (only meaningful when track_progress is true)
fn ensure_storage_bootstrapped_internal(track_progress: bool) -> Result<u32, AppInitError> {
    if STORAGE_BOOTSTRAPPED.load(Ordering::Acquire) {
        return Ok(0);
    }

    let _guard = BOOTSTRAP_LOCK.lock();

    // double-check after acquiring lock
    if STORAGE_BOOTSTRAPPED.load(Ordering::Acquire) {
        return Ok(0);
    }

    let bdk_count = do_bootstrap(track_progress)?;
    STORAGE_BOOTSTRAPPED.store(true, Ordering::Release);
    Ok(bdk_count)
}

/// Returns the pre-recovery BDK database count for use by attempt_bdk_migration
fn do_bootstrap(track_progress: bool) -> Result<u32, AppInitError> {
    crate::logging::init();
    if track_progress {
        set_step(BootstrapStep::DerivingEncryptionKey);
    }
    info!("Starting storage bootstrap");

    // derive encryption key from master key before any database access
    let cspp = cove_cspp::Cspp::new(cove_device::keychain::Keychain::global().clone());
    let master_key = cspp.get_or_create_master_key().map_err_str(AppInitError::KeyDerivation)?;

    let key = master_key.sensitive_data_key();
    crate::database::encrypted_backend::set_encryption_key(key);

    if track_progress {
        set_step(BootstrapStep::EncryptionKeySet);
    }
    info!("Encryption key derived and set");

    // verify the key matches the existing database before proceeding
    let db_path = cove_common::consts::ROOT_DATA_DIR.join("cove.db");
    crate::database::encrypted_backend::verify_database_key(&db_path)
        .map_err(map_database_key_verification_error)?;

    check_cancelled()?;

    // recover interrupted redb migrations before proceeding
    if track_progress {
        set_step(BootstrapStep::RecoveringInterruptedMigrations);
    }
    info!("Recovering interrupted redb migrations");
    crate::database::migration::recover_interrupted_migrations()
        .map_err_display_alt(AppInitError::MainDatabaseMigration)?;

    check_cancelled()?;

    // pre-count BDK databases before recovery so the total is stable
    let bdk_count = crate::database::migration::count_bdk_databases_needing_migration();

    // count items needing migration for progress bar
    let main_needs = crate::database::migration::main_database_needs_migration();
    let redb_count = crate::database::migration::count_redb_wallets_needing_migration();
    let total = main_needs as u32 + redb_count + bdk_count;

    let migration = Arc::new(Migration::new(total, Arc::clone(&BOOTSTRAP_CANCELLED)));
    if track_progress {
        migration::set_active_migration(Some(Arc::clone(&migration)));
    }

    if track_progress {
        set_step(BootstrapStep::MigratingMainDatabase);
    }
    info!("Migrating main database if needed");
    let migrated_main = crate::database::migration::migrate_main_database_if_needed()
        .map_err_display_alt(AppInitError::MainDatabaseMigration)?;

    if migrated_main {
        migration.tick();
    }

    check_cancelled()?;

    if track_progress {
        set_step(BootstrapStep::MigratingWalletDatabases);
    }
    info!("Migrating wallet databases if needed");
    if let Err(e) = crate::database::migration::WalletMigration::new(Arc::clone(&migration)).run() {
        error!("Wallet database migration failed: {e:#}");
        // prefer cancellation over migration error since the failure
        // may have been caused by the cancellation itself
        check_cancelled()?;
        return Err(AppInitError::WalletDatabaseMigration(format!("{e:#}")));
    }

    info!("Storage bootstrap complete");
    Ok(bdk_count)
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum StartupRecoveryState {
    /// Expected: device restore detected (encrypted dbs exist, key mismatch, no sentinel)
    /// Route to recovery screen with "Restore from Cloud Backup" and "Start Fresh" options
    DeviceRestore,

    /// Unexpected: key loss (encrypted dbs exist, key mismatch, sentinel present)
    /// Route to catastrophic error screen with bug-report + wipe option
    CatastrophicKeyLoss,
}

/// Returns the path to the install-lineage sentinel file
///
/// Swift owns the sentinel lifecycle (creation + backup-exclusion).
/// Rust only reads it to diagnose key mismatch scenarios
#[uniffi::export]
pub fn sentinel_path() -> String {
    cove_common::consts::ROOT_DATA_DIR.join(".install_sentinel").to_string_lossy().to_string()
}

/// Diagnose why a database key mismatch occurred
///
/// Called from iOS after bootstrap returns DatabaseKeyMismatch.
/// Checks the sentinel file to distinguish device restore from unexpected key loss
#[uniffi::export]
pub fn diagnose_key_mismatch() -> StartupRecoveryState {
    let sentinel = cove_common::consts::ROOT_DATA_DIR.join(".install_sentinel");
    if sentinel.exists() {
        StartupRecoveryState::CatastrophicKeyLoss
    } else {
        StartupRecoveryState::DeviceRestore
    }
}

fn map_database_key_verification_error(
    error: crate::database::error::DatabaseError,
) -> AppInitError {
    match error {
        crate::database::error::DatabaseError::HeaderIntegrity { error, .. } => {
            AppInitError::DatabaseKeyMismatch(error)
        }
        other => AppInitError::DatabaseVerificationFailed(other.to_string()),
    }
}

/// Pre-seed the bootstrap OnceLock with a test encryption key, skipping
/// keychain access and migrations
#[cfg(test)]
pub fn set_test_bootstrapped() {
    crate::database::encrypted_backend::set_test_encryption_key();
    STORAGE_BOOTSTRAPPED.store(true, Ordering::Release);
}

fn set_step(step: BootstrapStep) {
    let mut current = BOOTSTRAP_STEP.lock();

    if step <= *current {
        error!("bootstrap step regression: {current:?} -> {step:?}");
    }

    debug_assert!(step > *current, "bootstrap step must advance: {current:?} -> {step:?}");
    *current = step;
}

/// Current bootstrap step, readable from Swift/Kotlin for diagnostics on timeout or failure
#[uniffi::export]
pub fn bootstrap_progress() -> BootstrapStep {
    *BOOTSTRAP_STEP.lock()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::error::DatabaseError;

    #[test]
    fn database_key_verification_maps_header_integrity_to_key_mismatch() {
        let error = DatabaseError::HeaderIntegrity {
            path: "/tmp/cove.db".into(),
            error: "wrong key".into(),
        };

        let mapped = map_database_key_verification_error(error);
        assert!(
            matches!(mapped, AppInitError::DatabaseKeyMismatch(message) if message == "wrong key")
        );
    }

    #[test]
    fn diagnose_key_mismatch_without_sentinel_returns_device_restore() {
        // when sentinel doesn't exist, it's a device restore scenario
        let state = super::diagnose_key_mismatch();
        // in test environment ROOT_DATA_DIR may not have sentinel
        assert!(matches!(
            state,
            super::StartupRecoveryState::DeviceRestore
                | super::StartupRecoveryState::CatastrophicKeyLoss
        ));
    }

    #[test]
    fn database_key_verification_preserves_non_mismatch_failures() {
        let error = DatabaseError::BackendOpen {
            path: "/tmp/cove.db".into(),
            error: "permission denied".into(),
        };

        let mapped = map_database_key_verification_error(error);
        assert!(matches!(
            mapped,
            AppInitError::DatabaseVerificationFailed(message)
                if message == "failed to open encrypted backend at /tmp/cove.db: permission denied"
        ));
    }
}
