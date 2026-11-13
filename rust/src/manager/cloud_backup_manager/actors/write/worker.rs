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
use crate::manager::cloud_backup_manager::ops::read_record;
use crate::manager::cloud_backup_manager::owner::delete_authorization::StaleTarget;
use crate::manager::cloud_backup_manager::owner::inventory::RecordRead;
use crate::manager::cloud_backup_manager::wallets::{WalletBackupReader, WalletUpload};
use crate::wallet_identity::collect_local_wallet_identities;

/// What one authorized row delete did
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowDeleteReport {
    /// The record is gone: deleted now, or already absent at the provider
    ///
    /// The provider's answer to the delete is the result; the listing that refreshes the rows and
    /// the count is the inventory owner's, so it never holds the lane or the operation
    Removed { already_absent: bool },
    /// The authorization no longer matched the inventory at the queue head; nothing was deleted
    StaleTarget(StaleTarget),
}

/// What one wallet upload did
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalletUploadOutcome {
    /// The record was written
    Uploaded,
    /// Nothing was written: the cloud record holds key material this upload lacks, and it may be
    /// the last copy of it
    KeptKeyMaterial,
}

/// Remote storage operation submitted through the serialized write lane
#[derive(Debug)]
pub(crate) enum CloudBackupRemoteWriteCommand {
    UploadWallet {
        cloud: CloudStorageClient,
        namespace: String,
        upload: WalletUpload,
    },
    UploadMasterKey {
        cloud: CloudStorageClient,
        namespace: String,
        data: Vec<u8>,
    },
    /// One authorized row delete
    DeleteWalletRecord {
        cloud: CloudStorageClient,
        namespace: String,
        record_id: String,
    },
    /// One authorized delete of a record the verified key failed to decrypt, which reads it with
    /// that key first and deletes it only if it still fails
    DeleteUnreadableRecord {
        cloud: CloudStorageClient,
        reader: WalletBackupReader,
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
    /// A wallet upload wrote nothing, since it would have replaced key material
    KeptKeyMaterial,
    /// The record is gone
    WalletRecordRemoved {
        already_absent: bool,
    },
    /// An unreadable delete read the record as something that still exists and no longer fails to
    /// decrypt, so nothing was deleted
    RecordKept(RecordRead),
    WalletCount(u32),
    ListedWalletCount(Option<u32>),
    Keychain(KeychainSave),
}

impl CloudBackupRemoteWriteCommand {
    async fn execute(self) -> Result<CloudBackupRemoteWriteResult, CloudBackupWriteError> {
        match self {
            Self::UploadWallet { cloud, namespace, upload } => {
                // the replaced record is read here, just before the write, so a copy another
                // device wrote while this upload waited in the lane is the one checked
                if upload.drops_key_material(&cloud, &namespace).await? {
                    warn!("Cloud backup upload would drop key material, keeping the cloud copy");
                    return Ok(CloudBackupRemoteWriteResult::KeptKeyMaterial);
                }

                let (record_id, data) = upload.into_parts();
                cloud.upload_wallet_backup(namespace, record_id, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::UploadMasterKey { cloud, namespace, data } => {
                cloud.upload_master_key_backup(namespace, data).await?;
                Ok(CloudBackupRemoteWriteResult::None)
            }
            Self::DeleteWalletRecord { cloud, namespace, record_id } => {
                delete_wallet_record(&cloud, namespace, record_id).await
            }
            Self::DeleteUnreadableRecord { cloud, reader, namespace, record_id } => {
                // read here, just before the delete, so a record another device rewrote while
                // this waited in the lane is the one checked
                let local_wallets = collect_local_wallet_identities().map_err(|source| {
                    CloudBackupWriteError::internal_context("collect wallet identities", source)
                })?;
                match read_record(&reader, &record_id, &local_wallets).await? {
                    RecordRead::Undecryptable => {
                        delete_wallet_record(&cloud, namespace, record_id).await
                    }

                    // gone already, which is what the delete wanted
                    RecordRead::Absent => Ok(CloudBackupRemoteWriteResult::WalletRecordRemoved {
                        already_absent: true,
                    }),
                    read => Ok(CloudBackupRemoteWriteResult::RecordKept(read)),
                }
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

/// Deletes one wallet record; a record already gone is what the delete wanted, so it completes
async fn delete_wallet_record(
    cloud: &CloudStorageClient,
    namespace: String,
    record_id: String,
) -> Result<CloudBackupRemoteWriteResult, CloudBackupWriteError> {
    let already_absent = match cloud.delete_wallet_backup(namespace, record_id).await {
        Ok(()) => false,
        Err(CloudStorageError::NotFound(_)) => true,
        Err(error) => return Err(error.into()),
    };

    Ok(CloudBackupRemoteWriteResult::WalletRecordRemoved { already_absent })
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

    /// The delete answers once the provider removed the record; a listing that would fail or
    /// hang is the inventory owner's to run, never the lane's
    #[tokio::test(flavor = "current_thread")]
    async fn a_row_delete_completes_without_listing_the_backups() {
        let _guard = async_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        let globals = test_globals();
        globals.cloud.reset();

        let namespace = "0123456789abcdef0123456789abcdef".to_owned();
        let record_id = "wallet".to_owned();
        globals.cloud.set_wallet_backup(namespace.clone(), record_id.clone(), vec![1, 2, 3]);
        let listings = globals.cloud.list_wallet_files_attempt_count();

        let result = CloudBackupRemoteWriteCommand::DeleteWalletRecord {
            cloud: CloudStorage::global_explicit_client(),
            namespace: namespace.clone(),
            record_id: record_id.clone(),
        }
        .execute()
        .await
        .expect("the delete itself succeeded");

        assert!(matches!(
            result,
            CloudBackupRemoteWriteResult::WalletRecordRemoved { already_absent: false }
        ));
        assert!(!globals.cloud.has_wallet_backup(&namespace, &record_id));
        assert_eq!(globals.cloud.list_wallet_files_attempt_count(), listings);
    }
}
