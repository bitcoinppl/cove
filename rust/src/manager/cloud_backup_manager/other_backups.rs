use std::collections::HashMap;

use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use tracing::warn;

use super::{
    BlockingCloudStep, CloudBackupError, CloudBackupOtherBackupsOutcome,
    CloudBackupOtherBackupsState, CloudBackupOtherBackupsSummary, CloudBackupPasskeyHint,
    OtherBackupsOperation, RustCloudBackupManager, blocking_cloud_error,
};

impl RustCloudBackupManager {
    pub(crate) async fn other_backup_state(
        &self,
        cloud: &CloudStorageClient,
    ) -> CloudBackupOtherBackupsState {
        match self.other_backup_summary(cloud).await {
            Ok(summary) => CloudBackupOtherBackupsState::Loaded { summary },
            Err(error) => {
                warn!("Failed to summarize other cloud backups: {error}");
                CloudBackupOtherBackupsState::LoadFailed { error: error.to_string() }
            }
        }
    }

    pub(crate) async fn other_backup_summary(
        &self,
        cloud: &CloudStorageClient,
    ) -> Result<CloudBackupOtherBackupsSummary, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let local_wallet_record_ids = self.expected_wallet_record_ids().await?;
        let namespaces = self
            .other_backup_namespaces(cloud, &current_namespace, BlockingCloudStep::DetailRefresh)
            .await?;
        let passkey_hints = self.passkey_hints_for_namespaces(cloud, &namespaces).await;

        let mut namespace_count = 0;
        let mut wallet_count = 0;

        for namespace in &namespaces {
            let record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
                Ok(record_ids) => record_ids,
                Err(error) => {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::DetailRefresh,
                        CloudBackupError::cloud_storage_context(
                            format!("count wallets in other backup namespace {namespace}"),
                            error,
                        ),
                    ));
                }
            };

            namespace_count += 1;
            let unrecovered_wallet_count = record_ids
                .iter()
                .filter(|record_id| !local_wallet_record_ids.contains(*record_id))
                .count() as u32;

            wallet_count += unrecovered_wallet_count;
        }

        Ok(CloudBackupOtherBackupsSummary { namespace_count, wallet_count, passkey_hints })
    }

    pub(crate) async fn best_passkey_hint_for_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[String],
    ) -> Option<CloudBackupPasskeyHint> {
        self.passkey_hints_for_namespaces(cloud, namespaces)
            .await
            .into_iter()
            .max_by_key(|hint| hint.registered_at)
    }

    async fn passkey_hints_for_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[String],
    ) -> Vec<CloudBackupPasskeyHint> {
        let mut hints_by_suffix = HashMap::<String, CloudBackupPasskeyHint>::new();

        for namespace in namespaces {
            let Ok(master_json) =
                cloud.download_master_key_backup(namespace.clone()).await.inspect_err(|error| {
                    warn!("Failed to load passkey hint for namespace {namespace}: {error}")
                })
            else {
                continue;
            };

            let Ok(encrypted) = serde_json::from_slice::<
                cove_cspp::backup_data::EncryptedMasterKeyBackup,
            >(&master_json)
            .inspect_err(|error| {
                warn!("Failed to parse passkey hint for namespace {namespace}: {error}")
            }) else {
                continue;
            };
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                warn!("Failed to normalize passkey hint for namespace {namespace}");
                continue;
            }

            let Some(provider_hint) = encrypted.passkey_provider_hint.as_ref() else {
                continue;
            };
            let hint = CloudBackupPasskeyHint::from_provider_hint(provider_hint);

            hints_by_suffix
                .entry(hint.name_suffix.clone())
                .and_modify(|current| {
                    if hint.registered_at > current.registered_at {
                        *current = hint.clone();
                    }
                })
                .or_insert(hint);
        }

        let mut hints = hints_by_suffix.into_values().collect::<Vec<_>>();
        hints.sort_by_key(|hint| std::cmp::Reverse(hint.registered_at));
        hints
    }

    pub(crate) async fn other_backup_namespaces(
        &self,
        cloud: &CloudStorageClient,
        current_namespace: &str,
        step: BlockingCloudStep,
    ) -> Result<Vec<String>, CloudBackupError> {
        let mut namespaces = cloud.list_namespaces().await.map_err(|error| {
            blocking_cloud_error(
                step,
                CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
            )
        })?;

        namespaces.retain(|namespace| namespace != current_namespace);
        namespaces.sort();

        let mut backup_namespaces = Vec::new();
        for namespace in namespaces {
            match cloud.download_master_key_backup(namespace.clone()).await {
                Ok(_) => backup_namespaces.push(namespace),
                Err(CloudStorageError::NotFound(_)) => {}
                Err(error) => {
                    return Err(blocking_cloud_error(
                        step,
                        CloudBackupError::cloud_storage_context(
                            "inspect cloud backup namespace",
                            error,
                        ),
                    ));
                }
            }
        }

        Ok(backup_namespaces)
    }

    pub(crate) fn apply_other_backups_outcome(&self, outcome: CloudBackupOtherBackupsOutcome) {
        let other_backups_operation = match outcome {
            CloudBackupOtherBackupsOutcome::Idle => OtherBackupsOperation::Idle,
            CloudBackupOtherBackupsOutcome::Recovering => OtherBackupsOperation::Recovering,
            CloudBackupOtherBackupsOutcome::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            } => OtherBackupsOperation::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            },
            CloudBackupOtherBackupsOutcome::Deleting => OtherBackupsOperation::Deleting,
            CloudBackupOtherBackupsOutcome::Deleted => OtherBackupsOperation::Deleted,
            CloudBackupOtherBackupsOutcome::Failed(error) => {
                OtherBackupsOperation::Failed { error }
            }
        };

        self.apply_model_event(super::CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
            other_backups_operation,
        ));
    }
}
