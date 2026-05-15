use cove_device::cloud_storage::CloudStorageClient;
use cove_util::ResultExt as _;
use tracing::info;

use super::wallets::{DownloadedWalletBackup, WalletBackupLookup, WalletBackupReader};

use super::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, RustCloudBackupManager,
    blocking_cloud_error,
};

mod cloud_only;
mod disable;
mod enable;
mod other_backups;
mod restore;
mod sync;

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
    let Some(master_key) = cspp
        .load_master_key_from_store()
        .map_err_prefix("loading master key from store", CloudBackupError::Internal)?
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
    recover_missing: F,
) -> Result<cove_cspp::master_key::MasterKey, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<cove_cspp::master_key::MasterKey, CloudBackupError>>,
{
    match cspp
        .load_master_key_from_store()
        .map_err_prefix("load local master key", CloudBackupError::Internal)?
    {
        Some(master_key) => Ok(master_key),
        None => recover_missing().await,
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests;
