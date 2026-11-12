use std::collections::HashSet;
use std::sync::Arc;

use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::info;
use zeroize::Zeroizing;

use super::cloud_only::read_record;
use super::load_master_key_for_cloud_action;
use crate::manager::cloud_backup_manager::actors::{CloudBackupWriteClient, RowDeleteReport};
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteAuthorization, StaleTarget,
};
use crate::manager::cloud_backup_manager::owner::inventory::RecordReads;
use crate::manager::cloud_backup_manager::wallets::{PasskeyRequestAuthority, WalletBackupReader};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupStateReducerEvent,
    CloudBackupStore, CloudBackupUndecryptableWalletDeletionState, RustCloudBackupManager,
    VerificationState, blocking_cloud_error,
};
use crate::wallet_identity::LocalWalletIdentities;

const DELETE_UNDECRYPTABLE_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before inaccessible wallet backups can be deleted";

/// The records of the confirmed batch, read with the configuration's verified key for the
/// operation
///
/// The reads are evidence for the inventory's owner, stamped with the listing and configuration
/// they were made under, which applies them before the confirmed records still undecryptable are
/// authorized; this carries no permission to delete anything
pub(crate) struct CloudBackupPreparedUndecryptableWalletDeletion {
    reader: WalletBackupReader,
    reads: RecordReads,
}

impl CloudBackupPreparedUndecryptableWalletDeletion {
    /// What each confirmed record read as, with what the reads were made under
    pub(crate) fn reads(&self) -> &RecordReads {
        &self.reads
    }

    /// The reader the lane reads each record with again, just before deleting it
    pub(crate) fn into_reader(self) -> WalletBackupReader {
        self.reader
    }
}

/// A batch preparation that stopped, with every read it made before it did
///
/// The reads are facts the inventory owner applies even though nothing is authorized, so a
/// record read readable here loses the Delete its old unreadable row carried
#[derive(Debug)]
pub(crate) struct UndecryptablePreparationFailed {
    pub(crate) reads: RecordReads,
    pub(crate) error: CloudBackupError,
}

/// Why a batch delete stopped before its last record
#[derive(Debug)]
pub(crate) enum BatchStop {
    /// A record's authorization no longer held when the lane reached it; nothing more was deleted
    StaleTarget(StaleTarget),
    Failed(CloudBackupError),
}

/// What a frozen batch delete did; records it removed stay removed whatever stopped it
#[derive(Debug)]
pub(crate) struct BatchDeleteReport {
    pub(crate) removed: u32,
    pub(crate) stopped: Option<BatchStop>,
}

impl RustCloudBackupManager {
    pub(crate) fn apply_undecryptable_wallet_deletion_state(
        &self,
        state: CloudBackupUndecryptableWalletDeletionState,
    ) {
        self.apply_model_event(
            CloudBackupStateReducerEvent::UndecryptableWalletDeletionStateResolved(state),
        );
    }

    /// Reads the records of the confirmed batch, `record_ids`, that are still not this device's
    /// own, recording in `reads` what each read as
    ///
    /// Only the confirmed records are read, so a record listed after the confirmation is never
    /// added to the batch. `reads` names the listing generation and configuration the caller took
    /// before this reads the local wallets. The verification report this requires is what
    /// proves the key reading them is the configuration's verified one
    pub(crate) async fn prepare_delete_undecryptable_wallet_backups(
        &self,
        writes: &CloudBackupWriteClient,
        record_ids: Vec<String>,
        authority: PasskeyRequestAuthority,
        mut reads: RecordReads,
    ) -> Result<CloudBackupPreparedUndecryptableWalletDeletion, UndecryptablePreparationFailed>
    {
        let (reader, local_record_ids, local_wallets) =
            match self.undecryptable_read_setup(writes, authority).await {
                Ok(setup) => setup,
                Err(error) => return Err(UndecryptablePreparationFailed { reads, error }),
            };

        // a provider-wide failure stops the batch, but each read that lands is still a fact the
        // owner needs, so the reads in flight are collected rather than dropped with the error
        let mut failure = None;

        // scoped so the reads' borrow of the reader ends before it moves into the result
        {
            let mut lookups = stream::iter(
                record_ids
                    .into_iter()
                    .filter(|record_id| !local_record_ids.contains(record_id))
                    .map(|record_id| {
                        let reader = reader.clone();
                        let local_wallets = &local_wallets;

                        async move {
                            let read = read_record(&reader, &record_id, local_wallets).await;
                            (record_id, read)
                        }
                    }),
            )
            .buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY);

            while let Some((record_id, read)) = lookups.next().await {
                match read {
                    Ok(read) => {
                        reads.reads.insert(record_id, read);
                    }
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
        }

        if let Some(error) = failure {
            let error = blocking_cloud_error(
                BlockingCloudStep::DeleteUndecryptableWalletBackups,
                CloudBackupError::cloud_storage_context("download wallet backup", error),
            );
            return Err(UndecryptablePreparationFailed { reads, error });
        }

        Ok(CloudBackupPreparedUndecryptableWalletDeletion { reader, reads })
    }

    /// The verified key's reader, the record IDs of this device's wallets, and the local wallet
    /// identities a batch preparation reads with
    async fn undecryptable_read_setup(
        &self,
        writes: &CloudBackupWriteClient,
        authority: PasskeyRequestAuthority,
    ) -> Result<(WalletBackupReader, HashSet<String>, LocalWalletIdentities), CloudBackupError>
    {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteUndecryptableWalletBackups)?;

        let has_reported_failure = matches!(
            self.state.read().verification(),
            VerificationState::NeedsAttention(report)
                if report.wallet_issues.decryption_failed > 0
        );
        if !has_reported_failure {
            return Err(CloudBackupError::RecoveryRequired(
                "Run Cloud Backup verification before deleting inaccessible wallet backups".into(),
            ));
        }

        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, &namespace, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                writes,
                &namespace,
                DELETE_UNDECRYPTABLE_RECOVERY_MESSAGE,
                authority,
            )
        })
        .await
        .map_err(|error| {
            blocking_cloud_error(BlockingCloudStep::DeleteUndecryptableWalletBackups, error)
        })?;

        let local_record_ids = CloudBackupStore::global().local_wallet_record_ids()?;
        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        // read after the listing generation in `reads`, so a wallet change since moves it and
        // the owner drops these reads
        let local_wallets =
            crate::wallet_identity::collect_local_wallet_identities().map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;

        Ok((reader, local_record_ids, local_wallets))
    }

    /// Deletes the batch `authorization` froze, one record at a time through the write lane
    ///
    /// Only the authorized records are tried, never one found later. The lane reads each record
    /// just before deleting it and keeps one that no longer fails to decrypt; a record whose
    /// authorization went stale stops the batch, because the inventory changed under all of them
    pub(crate) async fn delete_authorized_undecryptable_wallet_backups(
        &self,
        reader: WalletBackupReader,
        authorization: Arc<DeleteAuthorization>,
        writes: CloudBackupWriteClient,
    ) -> BatchDeleteReport {
        let mut removed = 0u32;
        let record_ids: Vec<String> = authorization.record_ids().map(str::to_owned).collect();

        for record_id in record_ids {
            match self.delete_one_undecryptable(&reader, &authorization, &writes, record_id).await {
                Ok(Some(RowDeleteReport::Removed { .. })) => removed = removed.saturating_add(1),
                Ok(None) => {}
                Ok(Some(RowDeleteReport::StaleTarget(stale))) => {
                    return BatchDeleteReport {
                        removed,
                        stopped: Some(BatchStop::StaleTarget(stale)),
                    };
                }
                Err(error) => {
                    return BatchDeleteReport { removed, stopped: Some(BatchStop::Failed(error)) };
                }
            }
        }

        info!("Deleted {removed} inaccessible wallet backup(s)");
        BatchDeleteReport { removed, stopped: None }
    }

    /// Deletes one record of the batch, or `None` when the lane's read kept it
    async fn delete_one_undecryptable(
        &self,
        reader: &WalletBackupReader,
        authorization: &Arc<DeleteAuthorization>,
        writes: &CloudBackupWriteClient,
        record_id: String,
    ) -> Result<Option<RowDeleteReport>, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteUndecryptableWalletBackups)?;

        writes
            .delete_unreadable_record(
                CloudStorage::global_explicit_client(),
                reader.clone(),
                authorization.clone(),
                record_id,
            )
            .await
            .map_err(|error| {
                blocking_cloud_error(BlockingCloudStep::DeleteUndecryptableWalletBackups, error)
            })
    }
}
