use std::collections::HashSet;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::load_master_key_for_cloud_action;
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{WalletBackupLookup, WalletBackupReader};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupStateReducerEvent,
    CloudBackupStore, CloudBackupUndecryptableWalletDeletionState, RustCloudBackupManager,
    VerificationState, blocking_cloud_error, is_provider_wide_interruption,
};

const DELETE_UNDECRYPTABLE_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before inaccessible wallet backups can be deleted";

/// Rechecked cloud-only wallet records that may be removed by the confirmed operation
pub(crate) struct CloudBackupPreparedUndecryptableWalletDeletion {
    cloud: CloudStorageClient,
    namespace: String,
    reader: WalletBackupReader,
    record_ids: Vec<String>,
}

impl CloudBackupPreparedUndecryptableWalletDeletion {
    #[cfg(test)]
    pub(crate) fn record_ids(&self) -> &[String] {
        &self.record_ids
    }
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

    pub(crate) async fn prepare_delete_undecryptable_wallet_backups(
        &self,
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
                &namespace,
                DELETE_UNDECRYPTABLE_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| {
            blocking_cloud_error(BlockingCloudStep::DeleteUndecryptableWalletBackups, error)
        })?;

        let record_ids = cloud.list_wallet_backups(namespace.clone()).await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::DeleteUndecryptableWalletBackups,
                CloudBackupError::cloud_storage_context("list wallet backups", error),
            )
        })?;
        let local_record_ids = self.local_wallet_record_ids_for_undecryptable_deletion()?;
        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut candidates = {
            let mut candidates = Vec::new();
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
                    Err(CloudBackupError::Crypto(_)) => candidates.push(record_id),
                    Err(error) if is_provider_wide_interruption(&error) => {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::DeleteUndecryptableWalletBackups,
                            error,
                        ));
                    }
                    Err(error) => {
                        warn!(
                            "Skipping inaccessible wallet backup that did not fail decryption: {error}"
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

        candidates.sort();
        Ok(CloudBackupPreparedUndecryptableWalletDeletion {
            cloud,
            namespace,
            reader,
            record_ids: candidates,
        })
    }

    pub(crate) async fn delete_prepared_undecryptable_wallet_backups(
        &self,
        prepared: CloudBackupPreparedUndecryptableWalletDeletion,
        writes: CloudBackupWriteClient,
    ) -> Result<u32, CloudBackupError> {
        let CloudBackupPreparedUndecryptableWalletDeletion { cloud, namespace, reader, record_ids } =
            prepared;
        let mut deleted = 0u32;

        for record_id in record_ids {
            self.ensure_cloud_connectivity(BlockingCloudStep::DeleteUndecryptableWalletBackups)?;

            let local_record_ids = self.local_wallet_record_ids_for_undecryptable_deletion()?;
            if local_record_ids.contains(&record_id) {
                info!(
                    "Skipping inaccessible wallet backup deletion because the wallet is now on this device"
                );
                continue;
            }

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
                        "Skipping inaccessible wallet backup that no longer fails decryption: {error}"
                    );
                    continue;
                }
                Ok(_) => continue,
            }

            writes
                .delete_active_wallet_backup(cloud.clone(), namespace.clone(), record_id)
                .await
                .map_err(|error| {
                blocking_cloud_error(BlockingCloudStep::DeleteUndecryptableWalletBackups, error)
            })?;
            deleted = deleted.saturating_add(1);
        }

        info!("Deleted {deleted} inaccessible wallet backup(s)");
        Ok(deleted)
    }

    fn local_wallet_record_ids_for_undecryptable_deletion(
        &self,
    ) -> Result<HashSet<String>, CloudBackupError> {
        Ok(CloudBackupStore::new(&Database::global())
            .all_wallets()?
            .into_iter()
            .map(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()))
            .collect())
    }
}
