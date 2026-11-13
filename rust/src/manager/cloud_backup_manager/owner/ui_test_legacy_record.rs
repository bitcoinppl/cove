//! Debug-only seed of the legacy stuck Turn Off record, for the heal UI tests
//!
//! Builds before the fix left a Turn Off record that never deleted anything, and load
//! normalization heals it back to the configured backup with a notice. A UI test can't make an
//! older build write that record, so while `COVE_UI_TEST_LEGACY_STUCK_RECORD` names a directory
//! holding a `seed-legacy-stuck-record` request file, the next load writes the legacy record over
//! the configured one just before normalization and deletes the request, so the heal runs once
//! on the real load path. It rewrites the stored record, so it is compiled only with debug
//! assertions, which no release profile enables, and applies only while the variable is set.
//! Tests build it too, since the harness seeds the same record

use std::path::{Path, PathBuf};

use tracing::{error, info};

use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedConfiguredCloudBackup, PersistedDisableDisposition,
    PersistedDisablingCloudBackup,
};
use crate::manager::cloud_backup_manager::CloudBackupKeychain;
use crate::manager::cloud_backup_manager::owner::lifecycle::LifecycleCapability;

/// Set by the heal UI tests to the directory that holds the request file
pub(crate) const LEGACY_STUCK_RECORD_ENV: &str = "COVE_UI_TEST_LEGACY_STUCK_RECORD";

/// The request file; the seed deletes it once the record is written, so it seeds once
pub(crate) const REQUEST_FILE: &str = "seed-legacy-stuck-record";

/// The record a pre-fix build left behind: a Turn Off of `namespace_id` that never started its
/// delete, over the configuration it would have removed
///
/// Only builds before the fix wrote this shape (`begin_disabling` with no commit time); the exact
/// `last_error` on the reporter's device is not known
pub(crate) fn legacy_stuck_record(
    previous_configured: PersistedConfiguredCloudBackup,
    namespace_id: String,
    started_at: u64,
) -> PersistedCloudBackupState {
    PersistedCloudBackupState::disabling_transition(PersistedDisablingCloudBackup {
        previous_configured,
        namespace_id,
        disable_generation: 1,
        started_at,
        delete_started_at: None,
        last_error: None,
        retry_after: None,
        disposition: PersistedDisableDisposition::DeleteRemote,
        try_again_failed: false,
    })
}

/// Writes the legacy record when the UI test asked for one; a no-op otherwise
pub(crate) fn seed_requested_legacy_stuck_record(owner: &LifecycleCapability) {
    let Some(directory) = std::env::var_os(LEGACY_STUCK_RECORD_ENV) else { return };
    seed_from_request(owner, &PathBuf::from(directory).join(REQUEST_FILE));
}

fn seed_from_request(owner: &LifecycleCapability, request: &Path) {
    // the request file is the switch, so a test that already healed can relaunch with the
    // variable still set
    if !request.exists() {
        return;
    }

    if let Err(error) = seed_over_configured(owner) {
        // the request stays, so the test sees no heal and the next launch tries again
        error!("UI test legacy record seed failed: {error}");
        return;
    }

    if let Err(error) = std::fs::remove_file(request) {
        error!("UI test legacy record seed could not delete its request: {error}");
        return;
    }

    info!("UI test seeded the legacy stuck Turn Off record");
}

fn seed_over_configured(owner: &LifecycleCapability) -> Result<(), String> {
    let namespace_id = CloudBackupKeychain::global()
        .namespace_id()
        .map_err(|error| format!("read the namespace: {error}"))?
        .ok_or("the keychain holds no namespace")?;

    let started_at = cove_util::time::unix_timestamp_secs().unwrap_or_default();
    let seeded = Database::global()
        .cloud_backup_state
        .mutate(owner, |state| {
            let PersistedCloudBackupState::Configured(previous_configured) = state else {
                return false;
            };

            *state = legacy_stuck_record(previous_configured.clone(), namespace_id, started_at);
            true
        })
        .map_err(|error| format!("write the legacy record: {error}"))?;

    if !seeded.outcome {
        return Err("the stored record is not a configured backup".into());
    }

    Ok(())
}
