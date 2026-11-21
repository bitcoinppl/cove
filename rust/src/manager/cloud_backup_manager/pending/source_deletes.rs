//! Deletes other-backup namespaces once the cloud confirms the records that replaced them
//!
//! Recover Other Backups and enable merge cleanup prove that the active namespace holds a source
//! namespace's wallets, then record a pending source delete instead of deleting. A returned
//! upload can mean the file is only staged on this device, as on iOS, so a source deleted then
//! could leave the cloud with no copy of a wallet once its own deletion syncs. This owner deletes
//! a source only once every replacement record's upload, and the active master key wrapper's, is
//! confirmed in the active namespace and the provider reports the copy this device wrote
//! uploaded, and only while the source still lists no record the proof didn't cover

use act_zero::call;
use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use cove_device::cloud_storage::{
    CloudBackupUploadStatus, CloudStorage, CloudStorageClient, CloudStorageError,
};
use tracing::{info, warn};

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobFailedState, PersistedCloudBlobState, PersistedPendingSourceDelete,
};
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::{CloudBackupError, RustCloudBackupManager};

impl RustCloudBackupManager {
    /// Records that the source in `pending` is deleted once the provider reports its
    /// replacements uploaded, under the authority `writes` writes for, then starts a verifier
    /// pass, which deletes it once they are
    pub(crate) async fn record_pending_source_delete(
        &self,
        writes: &CloudBackupWriteClient,
        pending: PersistedPendingSourceDelete,
    ) -> Result<(), CloudBackupError> {
        // a worker may wait on the lifecycle owner
        call!(self.supervisor.record_pending_source_delete(writes.owner(), pending))
            .await
            .map_err(|_| CloudBackupError::Cancelled)??;

        self.wake_pending_upload_verifier();
        self.start_pending_upload_verification_loop();
        Ok(())
    }
}

/// What one pass found for one pending source delete
enum SourceDeleteStep {
    /// The source is deleted, or can no longer be: the entry is cleared
    Settled,
    /// A replacement record isn't confirmed yet, or the delete didn't run; a later pass retries
    Pending,
    /// The provider needs authorization, so no later entry is checked in this pass either
    AuthorizationRequired,
}

/// Settles the pending source deletes the configured record holds
///
/// Runs at the end of each pending upload verifier pass, under that pass's lock, so one settle
/// runs at a time. The pass already runs on launch, on provider metadata changes and after every
/// upload, so an entry still pending waits for the next one instead of keeping the verifier loop
/// alive
pub(crate) struct PendingSourceDeletes<'a>(pub(crate) &'a RustCloudBackupManager);

impl PendingSourceDeletes<'_> {
    pub(crate) async fn settle(&self) {
        let pending =
            RustCloudBackupManager::load_persisted_state().pending_source_deletes().to_vec();
        if pending.is_empty() {
            return;
        }

        // the configuration is read before any evidence, so a delete proven here carries it and
        // the write lane drops the delete once the configuration changes
        let generation = match call!(self.0.supervisor.evidence_basis_for_confirmation()).await {
            Ok(basis) => basis.config(),
            Err(error) => {
                warn!("Pending source delete: lifecycle owner stopped: {error}");
                return;
            }
        };

        // an unreadable keychain proves nothing about which namespace is active, so every entry
        // waits for a later pass
        let current_namespace = match self.0.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(error) => {
                warn!("Pending source delete: active namespace unknown: {error}");
                return;
            }
        };

        let cloud = CloudStorage::global_silent_client();
        let writes = CloudBackupWriteClient::background(self.0.cloud_writes.clone(), generation);

        for entry in pending {
            let step = if entry.active_namespace_id == current_namespace {
                self.settle_one(&cloud, &writes, &entry).await
            } else {
                info!(
                    "Pending source delete: keeping source={} reason=active_namespace_changed",
                    entry.source_namespace_id
                );
                SourceDeleteStep::Settled
            };

            match step {
                SourceDeleteStep::Settled => self.clear(entry).await,
                SourceDeleteStep::Pending => {}
                SourceDeleteStep::AuthorizationRequired => {
                    warn!("Pending source delete: paused until cloud authorization is restored");
                    return;
                }
            }
        }
    }

    async fn settle_one(
        &self,
        cloud: &CloudStorageClient,
        writes: &CloudBackupWriteClient,
        entry: &PersistedPendingSourceDelete,
    ) -> SourceDeleteStep {
        let source = &entry.source_namespace_id;

        // the replacements are readable only with the active master key wrapper, so a new device
        // restoring from the cloud needs it uploaded as much as the wallet records
        let required_record_ids = std::iter::once(MASTER_KEY_RECORD_ID.to_string())
            .chain(entry.record_ids.iter().cloned())
            .collect::<Vec<_>>();

        // every local row is checked before the provider is asked, so a terminal failure clears
        // the entry even while another record is still uploading
        let mut all_confirmed = true;
        for record_id in &required_record_ids {
            match Self::local_upload(entry, record_id) {
                LocalUpload::Confirmed => {}
                LocalUpload::Pending => all_confirmed = false,
                LocalUpload::TerminalFailure => {
                    warn!(
                        "Pending source delete: keeping source={source} reason=upload_failed record_id={record_id}"
                    );
                    return SourceDeleteStep::Settled;
                }
            }
        }

        if !all_confirmed {
            return SourceDeleteStep::Pending;
        }

        for record_id in &required_record_ids {
            match Self::provider_upload(cloud, &entry.active_namespace_id, record_id).await {
                Ok(CloudBackupUploadStatus::Uploaded) => {}
                Ok(CloudBackupUploadStatus::Pending | CloudBackupUploadStatus::NotFound) => {
                    info!(
                        "Pending source delete: waiting source={source} reason=not_uploaded record_id={record_id}"
                    );
                    return SourceDeleteStep::Pending;
                }
                Err(CloudStorageError::AuthorizationRequired(_)) => {
                    return SourceDeleteStep::AuthorizationRequired;
                }
                Err(error) => {
                    warn!(
                        "Pending source delete: waiting source={source} record_id={record_id} upload state unavailable: {error}"
                    );
                    return SourceDeleteStep::Pending;
                }
            }
        }

        // the proof read the source when it was recorded, and another device still using the
        // source may have added a wallet since, which no replacement holds
        match cloud.list_wallet_backups(source.clone()).await {
            Ok(listed) => {
                if let Some(unproven) =
                    listed.iter().find(|record_id| !entry.source_record_ids.contains(record_id))
                {
                    warn!(
                        "Pending source delete: keeping source={source} reason=source_gained_record record_id={unproven}"
                    );
                    return SourceDeleteStep::Settled;
                }
            }

            Err(CloudStorageError::NotFound(_)) => {
                info!("Cloud backup source namespace {source} was already deleted");
                return SourceDeleteStep::Settled;
            }

            Err(CloudStorageError::AuthorizationRequired(_)) => {
                return SourceDeleteStep::AuthorizationRequired;
            }

            Err(error) => {
                warn!(
                    "Pending source delete: waiting source={source} listing unavailable: {error}"
                );
                return SourceDeleteStep::Pending;
            }
        }

        match writes.delete_source_namespace(cloud.clone(), source.clone()).await {
            Ok(()) => {
                info!("Deleted cloud backup source namespace {source}");
                SourceDeleteStep::Settled
            }

            // a source already gone is what the delete wanted, as for Turn Off's delete
            Err(CloudBackupError::CloudStorage(CloudStorageError::NotFound(_))) => {
                info!("Cloud backup source namespace {source} was already deleted");
                SourceDeleteStep::Settled
            }

            // the delete waits for the lane to run background writes again, or for the next
            // configuration's pass, which drops the entry if another namespace became active
            Err(CloudBackupError::WritesPaused(reason)) => {
                info!("Pending source delete: waiting source={source} reason={reason:?}");
                SourceDeleteStep::Pending
            }

            Err(error) => {
                warn!("Pending source delete: failed to delete source={source}: {error}");
                SourceDeleteStep::Pending
            }
        }
    }

    /// The provider state of `record_id` in `namespace`, asked first of the copy this app writes
    ///
    /// An uploaded legacy copy would vouch for a new upload still staged at the written location,
    /// so the written copy decides whenever it exists. A record with none there, such as a master
    /// key wrapper an older build or another device wrote, is judged by its legacy copy, or the
    /// source would wait on it forever
    async fn provider_upload(
        cloud: &CloudStorageClient,
        namespace: &str,
        record_id: &str,
    ) -> Result<CloudBackupUploadStatus, CloudStorageError> {
        let written =
            cloud.is_written_backup_uploaded(namespace.to_owned(), record_id.to_owned()).await?;
        if written != CloudBackupUploadStatus::NotFound {
            return Ok(written);
        }

        cloud.is_backup_uploaded(namespace.to_owned(), record_id.to_owned()).await
    }

    /// Whether this device's upload of `record_id` to the active namespace is confirmed
    ///
    /// A record with no row was written by another device, and the provider check that follows
    /// is what proves it. A confirmed row alone is not proof, since a listing or summary read can
    /// confirm an upload the provider still holds only on this device
    fn local_upload(entry: &PersistedPendingSourceDelete, record_id: &str) -> LocalUpload {
        let row = match Database::global().cloud_blob_sync_states.get(record_id) {
            Ok(row) => row,
            Err(error) => {
                warn!("Pending source delete: failed to read upload state: {error}");
                return LocalUpload::Pending;
            }
        };

        let Some(row) = row else { return LocalUpload::Confirmed };
        if row.namespace_id != entry.active_namespace_id {
            return LocalUpload::Pending;
        }

        match row.state {
            PersistedCloudBlobState::Confirmed(_) => LocalUpload::Confirmed,
            PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. }) => {
                LocalUpload::TerminalFailure
            }
            PersistedCloudBlobState::Dirty(_)
            | PersistedCloudBlobState::Uploading(_)
            | PersistedCloudBlobState::UploadedPendingConfirmation(_)
            | PersistedCloudBlobState::Failed(_) => LocalUpload::Pending,
        }
    }

    async fn clear(&self, entry: PersistedPendingSourceDelete) {
        // waiting means the next pass's read already sees the entry gone
        if let Err(error) = call!(self.0.supervisor.clear_pending_source_delete(entry)).await {
            warn!("Pending source delete: lifecycle owner stopped before clearing: {error}");
        }
    }
}

/// This device's upload state for one replacement record
enum LocalUpload {
    Confirmed,
    Pending,
    TerminalFailure,
}
