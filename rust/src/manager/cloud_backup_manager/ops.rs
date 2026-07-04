use cove_device::cloud_storage::CloudStorageClient;
use tracing::info;

use super::wallets::{DownloadedWalletBackup, WalletBackupLookup, WalletBackupReader};

use super::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, RustCloudBackupManager,
    blocking_cloud_error,
};
mod cloud_only;
mod disable;
mod enable;
mod other_backup_operations;
mod restore;
mod sync;

pub(crate) use cloud_only::CloudBackupPreparedCloudWalletDelete;
pub(crate) use disable::{CloudBackupDisablePreparation, CloudBackupKeepEnabledPreparation};
pub(crate) use enable::{
    CloudBackupEnablePasskeyPreparation, CloudBackupEnablePasskeyRegistration,
    CloudBackupEnablePreparation, CloudBackupEnableRecoveryCompletion,
    CloudBackupEnableRecoveryPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupReadyEnableUpload, CloudBackupRegisteredEnablePasskey,
    CloudBackupSavedPasskeyConfirmation, CloudBackupUploadedEnableBackup,
    EnablePasskeyRegistrationFlow,
};
pub(crate) use sync::CloudBackupReuploadedWallets;

const CLOUD_ONLY_FETCH_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before wallets not on this device can be loaded";
const CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before this wallet can be restored";
const RECREATE_MANIFEST_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before the backup index can be recreated";
const UNSUPPORTED_CLOUD_ONLY_WALLET_NAME: &str = "Unsupported wallet backup";
impl RustCloudBackupManager {
    async fn lookup_wallet_backup(
        reader: WalletBackupReader,
        record_id: String,
    ) -> (String, Result<WalletBackupLookup<DownloadedWalletBackup>, CloudBackupError>) {
        let lookup = reader.lookup(&record_id).await;
        (record_id, lookup)
    }
}

pub(crate) async fn try_restore_from_local_master_key<S>(
    cloud: &CloudStorageClient,
    cspp: &cove_cspp::Cspp<S>,
) -> Result<Option<(cove_cspp::master_key::MasterKey, String)>, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    let Some(master_key) = cspp.load_master_key_from_store().map_err(|source| {
        CloudBackupError::internal_context("loading master key from store", source)
    })?
    else {
        return Ok(None);
    };
    let namespace_id = master_key.namespace_id();

    let has_wallets =
        cloud.list_wallet_backups(namespace_id.clone()).await.map(|ids| !ids.is_empty()).map_err(
            |error| CloudBackupError::cloud_storage_context("list wallet backups", error),
        )?;

    if has_wallets {
        info!("Restore: found local master key with wallets, namespace_id={namespace_id}");
        Ok(Some((master_key, namespace_id)))
    } else {
        info!(
            "Restore: local master key found but no wallets in cloud, falling through to passkey matching"
        );
        Ok(None)
    }
}

pub(crate) async fn load_master_key_for_cloud_action<S, F, Fut>(
    cspp: &cove_cspp::Cspp<S>,
    namespace: &str,
    recover_missing_or_stale: F,
) -> Result<cove_cspp::master_key::MasterKey, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<cove_cspp::master_key::MasterKey, CloudBackupError>>,
{
    let local_master_key = cspp
        .load_master_key_from_store()
        .map_err(|source| CloudBackupError::internal_context("load local master key", source))?;

    match local_master_key {
        Some(master_key) if master_key.namespace_id() == namespace => return Ok(master_key),
        Some(master_key) => {
            let local_namespace = master_key.namespace_id();
            info!(
                "Local master key namespace_id={local_namespace} does not match active namespace_id={namespace}, recovering from cloud"
            );
        }
        None => {}
    }

    let recovered = recover_missing_or_stale().await?;
    let recovered_namespace = recovered.namespace_id();
    if recovered_namespace != namespace {
        return Err(CloudBackupError::Internal(format!(
            "recovered master key namespace mismatch: expected {namespace}, got {recovered_namespace}",
        )
        .into()));
    }

    Ok(recovered)
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests;
