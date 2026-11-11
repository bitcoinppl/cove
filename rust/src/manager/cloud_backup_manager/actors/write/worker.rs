//! Leaf actor for remote Cloud Backup write commands
//!
//! This actor performs cloud-storage calls, and the keychain saves producers proved off the actor,
//! and returns raw results to the write supervisor. It does not mutate manager state directly

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, send};
use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use tracing::warn;

use super::CloudBackupWriteError;
use super::keychain::{KeychainSave, OwnedKeychainWrite, save_owned_keychain_writes};
use super::supervisor::{CloudBackupWriteCommandContext, CloudBackupWriteSupervisor};
use crate::manager::cloud_backup_manager::owner::delete_authorization::StaleTarget;

/// What one authorized row delete did
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowDeleteReport {
    /// The record is gone: deleted now, or already absent at the provider
    Removed { already_absent: bool, relist: Relist },
    /// The authorization no longer matched the inventory at the queue head; nothing was deleted
    StaleTarget(StaleTarget),
}

/// The listing a row delete runs after the record is gone
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Relist {
    Count(u32),
    /// The record is still gone; only the new count is unknown
    Failed,
}

/// Remote storage operation submitted through the serialized write lane
#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteCommand {
    UploadWallet {
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    },
    UploadMasterKey {
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
    },
    /// One authorized row delete: the record, then a listing for the new count
    DeleteWalletRecord {
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    },
    ListWalletCount {
        cloud: CloudStorageClient,
        namespace_id: String,
        fallback_count: u32,
    },
    ListWalletCountOptional {
        cloud: CloudStorageClient,
        namespace_id: String,
    },
    DeleteNamespace {
        cloud: CloudStorageClient,
        namespace: String,
    },
    /// Keys a producer proved for `namespace_id`; a local effect that runs in the lane so Turn
    /// Off's drain and a reset's idle wait cover it
    SaveKeychain {
        namespace_id: String,
        writes: Vec<OwnedKeychainWrite>,
    },
    None,
}

/// Remote result shape expected by the paired local completion
#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteResult {
    None,
    /// The record is gone; the listing after it is separate, so its failure doesn't undo the
    /// delete
    WalletRecordRemoved {
        already_absent: bool,
        relist: Result<Vec<String>, CloudBackupWriteError>,
    },
    WalletCount(u32),
    ListedWalletCount(Option<u32>),
    Keychain(KeychainSave),
}

impl CloudBackupRemoteWriteCommand {
    async fn execute(self) -> Result<CloudBackupRemoteWriteResult, CloudBackupWriteError> {
        match self {
            Self::UploadWallet { cloud, namespace, record_id, data } => {
                cloud.upload_wallet_backup(namespace, record_id, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::UploadMasterKey { cloud, namespace, data } => {
                cloud.upload_master_key_backup(namespace, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::DeleteWalletRecord { cloud, namespace, record_id } => {
                // a record already gone is what the delete wanted, so it completes
                let already_absent =
                    match cloud.delete_wallet_backup(namespace.clone(), record_id).await {
                        Ok(()) => false,
                        Err(CloudStorageError::NotFound(_)) => true,
                        Err(error) => return Err(error.into()),
                    };

                let relist = cloud.list_wallet_backups(namespace).await.map_err(|error| {
                    warn!(
                        "Row delete: the record is gone, but listing the backups failed: {error}"
                    );
                    CloudBackupWriteError::cloud_storage_context("list wallet backups", error)
                });

                Ok(CloudBackupRemoteWriteResult::WalletRecordRemoved { already_absent, relist })
            }
            Self::ListWalletCount { cloud, namespace_id, fallback_count } => {
                let wallet_count = match cloud.list_wallet_backups(namespace_id.clone()).await {
                    Ok(ids) => ids.len() as u32,
                    Err(error) => {
                        warn!(
                            "Finalize wallet uploads: failed to list wallet backups, falling back to uploaded wallet count: {error}"
                        );
                        fallback_count
                    }
                };
                Ok(CloudBackupRemoteWriteResult::WalletCount(wallet_count))
            }
            Self::ListWalletCountOptional { cloud, namespace_id } => {
                let listed_wallet_count = cloud
                    .list_wallet_backups(namespace_id)
                    .await
                    .inspect_err(|error| {
                        warn!("Cloud backup wallet count unavailable, listing failed: {error}");
                    })
                    .ok()
                    .map(|record_ids| record_ids.len() as u32);
                Ok(CloudBackupRemoteWriteResult::ListedWalletCount(listed_wallet_count))
            }
            Self::DeleteNamespace { cloud, namespace } => {
                cloud.delete_namespace(namespace).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::SaveKeychain { namespace_id, writes } => {
                // a platform keychain call blocks its thread, so it runs off the runtime
                let saved = cove_tokio::unblock::run_blocking(move || {
                    save_owned_keychain_writes(&namespace_id, writes)
                })
                .await?;

                Ok(CloudBackupRemoteWriteResult::Keychain(saved))
            }

            Self::None => Ok(CloudBackupRemoteWriteResult::None),
        }
    }
}

/// Actor that executes one remote write command and reports back to its parent
#[derive(Debug, Default)]
pub(crate) struct CloudBackupWriteWorker {
    parent: WeakAddr<CloudBackupWriteSupervisor>,
}

#[async_trait::async_trait]
impl Actor for CloudBackupWriteWorker {
    async fn started(&mut self, _addr: Addr<Self>) -> ActorResult<()> {
        Produces::ok(())
    }
}

impl CloudBackupWriteWorker {
    pub(crate) async fn execute(
        &mut self,
        parent: WeakAddr<CloudBackupWriteSupervisor>,
        context: CloudBackupWriteCommandContext,
        remote: CloudBackupRemoteWriteCommand,
    ) -> ActorResult<()> {
        self.parent = parent;
        let result = remote.execute().await;
        send!(self.parent.complete_remote_write(context, result));
        Produces::ok(())
    }
}

#[cfg(test)]
mod tests {
    use cove_device::cloud_storage::CloudStorage;

    use super::*;
    use crate::manager::cloud_backup_manager::ops::test_support::{async_test_lock, test_globals};

    #[tokio::test(flavor = "current_thread")]
    async fn a_row_delete_whose_relist_fails_still_reports_the_record_gone() {
        let _guard = async_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        let globals = test_globals();
        globals.cloud.reset();

        let namespace = "0123456789abcdef0123456789abcdef".to_owned();
        let record_id = "wallet".to_owned();
        globals.cloud.set_wallet_backup(namespace.clone(), record_id.clone(), vec![1, 2, 3]);
        globals.cloud.fail_list_wallet_files_for_namespace(
            namespace.clone(),
            CloudStorageError::NotAvailable("listing down".into()),
        );

        let result = CloudBackupRemoteWriteCommand::DeleteWalletRecord {
            cloud: CloudStorage::global_explicit_client(),
            namespace: namespace.clone(),
            record_id: record_id.clone(),
        }
        .execute()
        .await
        .expect("the delete itself succeeded");

        let CloudBackupRemoteWriteResult::WalletRecordRemoved { already_absent, relist } = result
        else {
            panic!("a row delete reports the record removed, got {result:?}");
        };
        assert!(!already_absent);
        assert!(relist.unwrap_err().to_string().contains("listing down"));
        assert!(!globals.cloud.has_wallet_backup(&namespace, &record_id));
    }
}
