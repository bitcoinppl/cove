use std::{path::Path, sync::LazyLock};

use cove_device::{
    cloud_storage::{CloudAccessPolicy, CloudStorage, CloudStorageError},
    keychain::Keychain,
};
use cove_util::ResultExt as _;
use tracing::{error, warn};

use crate::{database::Database, wallet::metadata::WalletId};

use super::{CLOUD_BACKUP_MANAGER, CloudBackupKeychain, CloudBackupStore};

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CatastrophicRecoveryError {
    #[error("{0}")]
    Failure(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreResult {
    BackupFound,
    NoBackupFound {
        provider: CatastrophicCloudRestoreProvider,
    },
    Offline {
        provider: CatastrophicCloudRestoreProvider,
    },
    Unreadable,
    Inconclusive {
        provider: CatastrophicCloudRestoreProvider,
        reason: CatastrophicCloudRestoreInconclusiveReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreProvider {
    ICloudDrive,
    GoogleDrive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CatastrophicCloudRestoreInconclusiveReason {
    AuthorizationRequired,
    QuotaExceeded,
    ProviderUnavailable,
    Unknown,
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
        Ok(false) => CatastrophicCloudRestoreResult::NoBackupFound { provider },
        Err(error) => catastrophic_cloud_restore_error(error, provider),
    }
}

fn catastrophic_cloud_restore_error(
    error: CloudStorageError,
    provider: CatastrophicCloudRestoreProvider,
) -> CatastrophicCloudRestoreResult {
    match error {
        CloudStorageError::AuthorizationRequired(message) => {
            if !message.trim().is_empty() {
                warn!("Catastrophic cloud restore check authorization required: {message}");
            }

            CatastrophicCloudRestoreResult::Inconclusive {
                provider,
                reason: CatastrophicCloudRestoreInconclusiveReason::AuthorizationRequired,
            }
        }
        CloudStorageError::Offline(message) => {
            warn!("Catastrophic cloud restore check offline: {message}");
            CatastrophicCloudRestoreResult::Offline { provider }
        }
        CloudStorageError::NotFound(_) => {
            CatastrophicCloudRestoreResult::NoBackupFound { provider }
        }
        CloudStorageError::DownloadFailed(message) => {
            warn!("Catastrophic cloud restore check unreadable backup: {message}");
            CatastrophicCloudRestoreResult::Unreadable
        }
        CloudStorageError::InvalidNamespace(message) => {
            warn!("Catastrophic cloud restore check invalid namespace: {message}");
            CatastrophicCloudRestoreResult::Unreadable
        }
        CloudStorageError::QuotaExceeded => CatastrophicCloudRestoreResult::Inconclusive {
            provider,
            reason: CatastrophicCloudRestoreInconclusiveReason::QuotaExceeded,
        },
        CloudStorageError::NotAvailable(message) => {
            warn!("Catastrophic cloud restore check provider unavailable: {message}");
            CatastrophicCloudRestoreResult::Inconclusive {
                provider,
                reason: CatastrophicCloudRestoreInconclusiveReason::ProviderUnavailable,
            }
        }
        CloudStorageError::UploadFailed(message) => {
            warn!("Catastrophic cloud restore check failed: {message}");
            CatastrophicCloudRestoreResult::Inconclusive {
                provider,
                reason: CatastrophicCloudRestoreInconclusiveReason::Unknown,
            }
        }
    }
}

fn wipe_local_data_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    use crate::database::migration::log_remove_file;

    wipe_wallet_keychain_items_for_catastrophic_recovery()?;
    CloudBackupKeychain::global()
        .clear_local_state()
        .map_err_str(CatastrophicRecoveryError::Failure)?;

    let root = &*cove_common::consts::ROOT_DATA_DIR;

    log_remove_file(&root.join("cove.encrypted.db"));
    log_remove_file(&root.join("cove.db"));

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("bdk_wallet") {
                log_remove_file(&entry.path());
            }
        }
    }

    let wallet_dir = &*cove_common::consts::WALLET_DATA_DIR;
    if wallet_dir.exists()
        && let Err(error) = std::fs::remove_dir_all(wallet_dir)
    {
        error!("Failed to remove wallet data dir: {error}");
    }

    Ok(())
}

fn clear_in_process_cloud_backup_state_for_catastrophic_recovery() {
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

    if let Some(manager) = LazyLock::get(&CLOUD_BACKUP_MANAGER) {
        manager.clear_in_process_state_for_local_reset();
    }
}

fn reinit_database_after_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    crate::database::wallet_data::DATABASE_CONNECTIONS.write().clear();
    Database::try_reinit()
        .map_err_prefix("reinitialize database", CatastrophicRecoveryError::Failure)
}

fn wipe_wallet_keychain_items_for_catastrophic_recovery() -> Result<(), CatastrophicRecoveryError> {
    let keychain = Keychain::global();
    let wallet_ids = catastrophic_wipe_wallet_ids(
        persisted_wallet_ids_for_catastrophic_wipe(),
        &cove_common::consts::WALLET_DATA_DIR,
    );
    let mut failed_wallet_ids = Vec::new();

    for wallet_id in wallet_ids {
        if !keychain.delete_wallet_items(&wallet_id) {
            failed_wallet_ids.push(wallet_id.to_string());
        }
    }

    if failed_wallet_ids.is_empty() {
        return Ok(());
    }

    let failed_wallet_ids = failed_wallet_ids.join(", ");
    error!("Failed to delete wallet keychain items for: {failed_wallet_ids}");
    Err(CatastrophicRecoveryError::Failure(format!(
        "failed to delete wallet keychain items for: {failed_wallet_ids}"
    )))
}

fn persisted_wallet_ids_for_catastrophic_wipe() -> Option<Vec<WalletId>> {
    let Some(db_swap) = crate::database::DATABASE.get() else {
        warn!("Database not initialized, deriving wipe wallet ids from wallet data dir");
        return None;
    };

    let db = db_swap.load();
    match CloudBackupStore::new(&db).all_wallets() {
        Ok(wallets) => Some(wallets.into_iter().map(|wallet| wallet.id).collect()),
        Err(error) => {
            warn!(
                "Failed to read wallet ids for catastrophic recovery, deriving from wallet data dir: {error}"
            );
            None
        }
    }
}

fn catastrophic_wipe_wallet_ids(
    persisted_wallet_ids: Option<Vec<WalletId>>,
    wallet_data_dir: &Path,
) -> Vec<WalletId> {
    if let Some(wallet_ids) = persisted_wallet_ids {
        return wallet_ids;
    }

    wallet_ids_from_wallet_data_dir(wallet_data_dir)
}

fn wallet_ids_from_wallet_data_dir(wallet_data_dir: &Path) -> Vec<WalletId> {
    let mut wallet_ids = std::collections::BTreeSet::new();
    let entries = match std::fs::read_dir(wallet_data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!("Failed to read wallet data dir during catastrophic wipe: {error}");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(wallet_id) = file_name.to_str() else {
            continue;
        };
        wallet_ids.insert(wallet_id.to_owned());
    }

    wallet_ids.into_iter().map(WalletId::from).collect()
}

#[cfg(test)]
mod tests {
    use cove_device::cloud_storage::CloudStorageError;
    use tempfile::TempDir;

    use super::*;

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
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Ok(false),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::NoBackupFound {
                provider: CatastrophicCloudRestoreProvider::ICloudDrive
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_requires_access_for_blank_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired(" ".into())),
                CatastrophicCloudRestoreProvider::ICloudDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                provider: CatastrophicCloudRestoreProvider::ICloudDrive,
                reason: CatastrophicCloudRestoreInconclusiveReason::AuthorizationRequired
            }
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_classifies_authorization_message() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::AuthorizationRequired("sign in before continuing".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Inconclusive {
                provider: CatastrophicCloudRestoreProvider::GoogleDrive,
                reason: CatastrophicCloudRestoreInconclusiveReason::AuthorizationRequired
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
                provider: CatastrophicCloudRestoreProvider::GoogleDrive
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
                provider: CatastrophicCloudRestoreProvider::ICloudDrive
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
            CatastrophicCloudRestoreResult::Unreadable
        );
    }

    #[test]
    fn catastrophic_cloud_restore_error_reports_unreadable_invalid_namespace() {
        assert_eq!(
            catastrophic_cloud_restore_check_result(
                Err(CloudStorageError::InvalidNamespace("bad namespace".into())),
                CatastrophicCloudRestoreProvider::GoogleDrive
            ),
            CatastrophicCloudRestoreResult::Unreadable
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
                provider: CatastrophicCloudRestoreProvider::ICloudDrive,
                reason: CatastrophicCloudRestoreInconclusiveReason::QuotaExceeded
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
                provider: CatastrophicCloudRestoreProvider::GoogleDrive,
                reason: CatastrophicCloudRestoreInconclusiveReason::ProviderUnavailable
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
            CatastrophicCloudRestoreResult::Inconclusive {
                provider: CatastrophicCloudRestoreProvider::ICloudDrive,
                reason: CatastrophicCloudRestoreInconclusiveReason::Unknown
            }
        );
    }

    #[test]
    fn catastrophic_wipe_wallet_ids_prefers_persisted_wallet_ids() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-from-dir")).unwrap();

        let wallet_ids = catastrophic_wipe_wallet_ids(
            Some(vec![WalletId::from("wallet-from-db".to_string())]),
            dir.path(),
        );

        assert_eq!(wallet_ids, vec![WalletId::from("wallet-from-db".to_string())]);
    }

    #[test]
    fn catastrophic_wipe_wallet_ids_falls_back_to_wallet_data_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-from-dir")).unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-two")).unwrap();

        let wallet_ids = catastrophic_wipe_wallet_ids(None, dir.path());

        assert_eq!(
            wallet_ids,
            vec![
                WalletId::from("wallet-from-dir".to_string()),
                WalletId::from("wallet-two".to_string()),
            ]
        );
    }

    #[test]
    fn wallet_ids_from_wallet_data_dir_uses_directory_names() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("AbCd123")).unwrap();
        std::fs::create_dir_all(dir.path().join("wallet-two")).unwrap();
        std::fs::write(dir.path().join("bdk_wallet_abcd123.db"), "").unwrap();

        let wallet_ids = wallet_ids_from_wallet_data_dir(dir.path());

        assert_eq!(
            wallet_ids,
            vec![WalletId::from("AbCd123".to_string()), WalletId::from("wallet-two".to_string()),],
        );
    }

    #[test]
    fn wallet_ids_from_wallet_data_dir_returns_empty_for_missing_dir() {
        let dir = TempDir::new().unwrap();
        let wallet_ids = wallet_ids_from_wallet_data_dir(&dir.path().join("missing"));

        assert!(wallet_ids.is_empty());
    }
}
