use std::collections::BTreeSet;
use std::sync::Arc;

use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::load_master_key_for_cloud_action;
use crate::manager::cloud_backup_manager::actors::{CloudBackupWriteClient, RowDeleteReport};
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteAuthorization, StaleTarget,
};
use crate::manager::cloud_backup_manager::owner::inventory::UnreadableRead;
use crate::manager::cloud_backup_manager::wallets::{
    PasskeyRequestAuthority, WalletBackupLookup, WalletBackupReader,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupStateReducerEvent,
    CloudBackupStore, CloudBackupUndecryptableWalletDeletionState, RustCloudBackupManager,
    VerificationState, blocking_cloud_error, is_provider_wide_interruption,
};

const DELETE_UNDECRYPTABLE_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before inaccessible wallet backups can be deleted";

/// The records of the confirmed batch the configured key failed to decrypt, read for the
/// operation
///
/// The read is evidence for the inventory's owner, stamped with the listing and configuration it
/// was made under, which records it before the confirmed records it still holds are authorized;
/// this carries no permission to delete anything
pub(crate) struct CloudBackupPreparedUndecryptableWalletDeletion {
    reader: WalletBackupReader,
    read: UnreadableRead,
}

impl CloudBackupPreparedUndecryptableWalletDeletion {
    /// The records the configured key failed to decrypt, with what they were read under
    pub(crate) fn read(&self) -> &UnreadableRead {
        &self.read
    }

    /// The reader the batch rechecks each record with before deleting it
    pub(crate) fn into_reader(self) -> WalletBackupReader {
        self.reader
    }
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

    /// Rechecks the records of the confirmed batch, `record_ids`, keeping in `read` those that
    /// are still not this device's own and still fail to decrypt
    ///
    /// Only the confirmed records are read, so a record listed after the confirmation is never
    /// added to the batch. `read` names the listing generation and configuration the caller took
    /// before this reads the local wallets. The verification report this requires is what
    /// proves the key that failed is the configuration's verified one
    pub(crate) async fn prepare_delete_undecryptable_wallet_backups(
        &self,
        writes: &CloudBackupWriteClient,
        record_ids: Vec<String>,
        authority: PasskeyRequestAuthority,
        mut read: UnreadableRead,
    ) -> Result<CloudBackupPreparedUndecryptableWalletDeletion, CloudBackupError> {
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
        read.record_ids = {
            let mut candidates = BTreeSet::new();
            let mut lookups = stream::iter(
                record_ids
                    .into_iter()
                    .filter(|record_id| !local_record_ids.contains(record_id))
                    .map(|record_id| {
                        let reader = reader.clone();

                        async move {
                            let result = reader.lookup_entry(&record_id).await;
                            (record_id, result)
                        }
                    }),
            )
            .buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY);

            while let Some((record_id, result)) = lookups.next().await {
                match result {
                    Err(CloudBackupError::Crypto(_)) => {
                        candidates.insert(record_id);
                    }
                    Err(error) if is_provider_wide_interruption(&error) => {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::DeleteUndecryptableWalletBackups,
                            error,
                        ));
                    }
                    Err(error) => {
                        warn!(
                            "skipping inaccessible wallet backup that did not fail decryption: {error}"
                        );
                    }
                    Ok(
                        WalletBackupLookup::Found(_)
                        | WalletBackupLookup::NotFound
                        | WalletBackupLookup::UnsupportedVersion(_),
                    ) => {}
                }
            }

            candidates
        };

        Ok(CloudBackupPreparedUndecryptableWalletDeletion { reader, read })
    }

    /// Deletes the batch `authorization` froze, one record at a time through the write lane
    ///
    /// Only the authorized records are tried, never one found later. A record that now decrypts
    /// is skipped; a record whose authorization went stale stops the batch, because the
    /// inventory changed under all of them
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

    /// Deletes one record of the batch, or `None` when a fresh read shows it no longer fails
    /// decryption
    async fn delete_one_undecryptable(
        &self,
        reader: &WalletBackupReader,
        authorization: &Arc<DeleteAuthorization>,
        writes: &CloudBackupWriteClient,
        record_id: String,
    ) -> Result<Option<RowDeleteReport>, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteUndecryptableWalletBackups)?;

        match reader.lookup_entry(&record_id).await {
            Err(CloudBackupError::Crypto(_)) => {}
            Err(error) if is_provider_wide_interruption(&error) => {
                return Err(blocking_cloud_error(
                    BlockingCloudStep::DeleteUndecryptableWalletBackups,
                    error,
                ));
            }
            Err(error) => {
                warn!(
                    "skipping inaccessible wallet backup that no longer fails decryption: {error}"
                );
                return Ok(None);
            }
            Ok(_) => return Ok(None),
        }

        let report = writes
            .delete_authorized_record(
                CloudStorage::global_explicit_client(),
                authorization.clone(),
                record_id,
            )
            .await
            .map_err(|error| {
                blocking_cloud_error(BlockingCloudStep::DeleteUndecryptableWalletBackups, error)
            })?;

        Ok(Some(report))
    }
}
