use std::sync::LazyLock;

use crate::database::Database;
use cove_device::{
    cloud_storage::{CloudAccessPolicy, CloudStorage, CloudStorageError},
    keychain::Keychain,
};
use cove_util::ResultExt as _;

use super::{CLOUD_BACKUP_MANAGER, CloudBackupKeychain};

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CatastrophicRecoveryError {
    #[error("{0}")]
    Failure(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreResult {
    BackupFound,
    NoBackupFound { message: String },
    Offline { message: String },
    Unreadable { message: String },
    Inconclusive { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreProvider {
    ICloudDrive,
    GoogleDrive,
}

impl CatastrophicCloudRestoreProvider {
    fn storage_name(self) -> &'static str {
        match self {
            Self::ICloudDrive => "iCloud",
            Self::GoogleDrive => "Google Drive",
        }
    }

    fn account_name(self) -> &'static str {
        match self {
            Self::ICloudDrive => "iCloud account",
            Self::GoogleDrive => "Google account",
        }
    }
}

/// Reset local state for the database-encryption-key-mismatch recovery flow
///
/// Removes wallet keychain items, deletes local databases, then reinitializes
/// the database handle so bootstrap can start from a clean state
#[uniffi::export]
pub fn reset_local_data_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    wipe_local_data_for_catastrophic_recovery()?;
    clear_in_process_cloud_backup_state_for_catastrophic_recovery();
    reinit_database_after_catastrophic_recovery()
}

#[uniffi::export]
pub async fn check_catastrophic_cloud_restore_backup(
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    catastrophic_cloud_restore_check_result(
        CloudStorage::global().has_restorable_cloud_backup(CloudAccessPolicy::ConsentAllowed).await,
        provider,
    )
}

fn catastrophic_cloud_restore_check_result(
    result: Result<bool, CloudStorageError>,
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    match result {
        Ok(true) => CatastrophicCloudRestoreResult::BackupFound,
        Ok(false) => CatastrophicCloudRestoreResult::NoBackupFound {
            message: format!(
                "No Cloud Backup was found for the selected {}.",
                provider.account_name()
            ),
        },
        Err(error) => catastrophic_cloud_restore_error(error, provider),
    }
}

fn catastrophic_cloud_restore_error(
    error: CloudStorageError,
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    match error {
        CloudStorageError::AuthorizationRequired(message) => {
            if message.trim().is_empty() {
                return CatastrophicCloudRestoreResult::Inconclusive {
                    message: format!(
                        "{} access is required before local data can be reset.",
                        provider.storage_name()
                    ),
                };
            }

            CatastrophicCloudRestoreResult::Inconclusive { message }
        }
        CloudStorageError::Offline(message) => CatastrophicCloudRestoreResult::Offline {
            message: format!("Cannot check {} while offline: {message}", provider.storage_name()),
        },
        CloudStorageError::SyncPending(_) => CatastrophicCloudRestoreResult::Inconclusive {
            message: format!(
                "{} is still loading Cove backup files. Keep Cove open, then try again.",
                provider.storage_name()
            ),
        },
        CloudStorageError::NotFound(_) => CatastrophicCloudRestoreResult::NoBackupFound {
            message: format!(
                "No Cloud Backup was found for the selected {}.",
                provider.account_name()
            ),
        },
        CloudStorageError::DownloadFailed(message) => CatastrophicCloudRestoreResult::Unreadable {
            message: format!("Cloud Backup data could not be read: {message}"),
        },
        CloudStorageError::InvalidNamespace(_) => CatastrophicCloudRestoreResult::Unreadable {
            message: "Cloud Backup data could not be read.".into(),
        },
        CloudStorageError::QuotaExceeded => CatastrophicCloudRestoreResult::Inconclusive {
            message: format!(
                "{} quota is exceeded. Cove could not check for a Cloud Backup.",
                provider.storage_name()
            ),
        },
        CloudStorageError::NotAvailable(message) => CatastrophicCloudRestoreResult::Inconclusive {
            message: format!("{} is unavailable: {message}", provider.storage_name()),
        },
        CloudStorageError::UploadFailed(message) => {
            CatastrophicCloudRestoreResult::Inconclusive { message }
        }
    }
}

fn wipe_local_data_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    let cleanup = crate::wallet::deletion::RecoveryCleanup::prepare_database_unavailable()
        .map_err_str(CatastrophicRecoveryError::Failure)?;
    cleanup.delete_all_wallet_items().map_err_str(CatastrophicRecoveryError::Failure)?;
    CloudBackupKeychain::global()
        .clear_local_state()
        .map_err_str(CatastrophicRecoveryError::Failure)?;

    cleanup
        .purge_orphan_wallet_artifacts()
        .map_err_prefix("remove wallet artifacts", CatastrophicRecoveryError::Failure)?;

    // restore markers and locks must not survive the wallets they describe, or
    // the next bootstrap replays recovery against data this reset removed
    crate::backup::recovery::remove_all_restore_recovery_state()
        .map_err_str(CatastrophicRecoveryError::Failure)?;

    let root = &*cove_common::consts::ROOT_DATA_DIR;

    crate::bdk_store::BdkStore::remove_wallet_artifact(&root.join("cove.encrypted.db"))
        .map_err_prefix("remove encrypted database", CatastrophicRecoveryError::Failure)?;
    crate::bdk_store::BdkStore::remove_wallet_artifact(&root.join("cove.db"))
        .map_err_prefix("remove database", CatastrophicRecoveryError::Failure)?;

    Ok(())
}

fn clear_in_process_cloud_backup_state_for_catastrophic_recovery() {
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

    if let Some(manager) = LazyLock::get(&CLOUD_BACKUP_MANAGER) {
        manager.clear_in_process_state_for_local_reset();
    }
}

fn reinit_database_after_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    crate::database::wallet_data::clear_database_connections();
    Database::try_reinit()
        .map_err_prefix("reinitialize database", CatastrophicRecoveryError::Failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_device::cloud_storage::CloudStorageError;

    #[test]
    fn catastrophic_cloud_restore_check_result_reports_backup_found() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Ok(true),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::BackupFound
        );
    }

    #[test]
    fn catastrophic_cloud_restore_check_result_reports_no_backup_found() {
        assert!(matches!(
            catastrophic_cloud_restore_check_result(
                Ok(false),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::NoBackupFound { .. }
        ));
    }

    #[test]
    fn catastrophic_cloud_restore_error_requires_access_for_blank_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired(" ".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "iCloud access is required before local data can be reset.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_preserves_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired("sign in before continuing".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "sign in before continuing".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_offline_state() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::Offline("offline".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Offline {
                message: "Cannot check Google Drive while offline: offline".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_treats_not_found_as_no_backup() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::NotFound("namespace".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::NoBackupFound {
                message: "No Cloud Backup was found for the selected iCloud account.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_unreadable_download_failure() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::DownloadFailed("bad json".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Unreadable {
                message: "Cloud Backup data could not be read: bad json".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_unreadable_invalid_namespace() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::InvalidNamespace("bad namespace".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Unreadable {
                message: "Cloud Backup data could not be read.".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_quota_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::QuotaExceeded),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "iCloud quota is exceeded. Cove could not check for a Cloud Backup."
                    .into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_provider_unavailable_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::NotAvailable("service unavailable".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                message: "Google Drive is unavailable: service unavailable".into()
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_upload_failure_as_inconclusive() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::UploadFailed("upload failed".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive { message: "upload failed".into() }
        );
    }
}
