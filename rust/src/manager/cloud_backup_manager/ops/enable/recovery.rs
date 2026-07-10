use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::types::{
    CloudBackupEnableRecoveryCompletion, CloudBackupEnableRecoveryPreparation, MergeNamespace,
};
use super::{BlockingCloudStep, RustCloudBackupManager, blocking_cloud_error};
use crate::manager::cloud_backup_manager::actors::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupUploadedWallet,
    CloudBackupWriteClient,
};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome,
    WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupError, CloudBackupStore, is_connectivity_related_issue,
};
use crate::wallet::metadata::WalletMetadata;

struct MergedNamespaceWallets {
    source: CleanupSourceNamespace,
    restored_wallets: Vec<WalletMetadata>,
}

impl RustCloudBackupManager {
    pub(crate) async fn prepare_enable_recovery(
        &self,
        context: CloudBackupEnableContext,
        matches: Vec<NamespaceMatch>,
    ) -> Result<CloudBackupEnableRecoveryPreparation, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let merge_namespaces = self.load_enable_merge_namespaces(&cloud, matches).await?;
        let Some(active_index) = active_merge_namespace_index(&merge_namespaces) else {
            return Err(CloudBackupError::Internal(
                "no matching cloud backup namespaces found".into(),
            ));
        };
        let active_namespace_id = merge_namespaces[active_index].matched.namespace_id.clone();
        let active_master_key = cove_cspp::master_key::MasterKey::from_bytes(
            *merge_namespaces[active_index].matched.master_key.as_bytes(),
        );
        if active_master_key.namespace_id() != active_namespace_id {
            return Err(CloudBackupError::Internal(
                "recovered master key did not match its cloud namespace".into(),
            ));
        }
        let active_critical_key = active_master_key.critical_data_key();

        info!(
            "Enable: merging {} recovered namespace(s) into active namespace {}",
            merge_namespaces.len(),
            active_namespace_id
        );

        Ok(CloudBackupEnableRecoveryPreparation {
            context,
            merge_namespaces,
            active_index,
            active_namespace_id,
            active_master_key: Zeroizing::new(active_master_key),
            active_critical_key: Zeroizing::new(active_critical_key),
        })
    }

    pub(crate) async fn prepare_enable_recovery_completion(
        &self,
        preparation: CloudBackupEnableRecoveryPreparation,
        writes: CloudBackupWriteClient,
    ) -> Result<CloudBackupEnableRecoveryCompletion, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let active_namespace_id = preparation.active_namespace_id.clone();
        let merged_wallets =
            self.restore_enable_merge_wallets(&cloud, &preparation.merge_namespaces).await?;

        for merged in &merged_wallets {
            info!(
                "Enable: recovered {} wallet(s) from matched namespace {}",
                merged.restored_wallets.len(),
                merged.source.namespace_id
            );
        }

        self.mark_enable_recovery_remote_writes_started(&preparation)?;
        let active_critical_key = preparation.active_critical_key;
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets_with_progress(
                &writes,
                &cloud,
                &active_namespace_id,
                &active_critical_key,
                0,
                |progress| self.report_enable_progress(progress),
            )
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Enable, error))?;
        let pending_uploads = Self::pending_verification_uploads(&uploaded_wallets);

        let active_match = &preparation.merge_namespaces[preparation.active_index].matched;
        let credential_id = active_match.credential_id.clone();
        let prf_salt = active_match.prf_salt;
        let uploaded_wallets = uploaded_wallets
            .into_iter()
            .map(|wallet| {
                CloudBackupUploadedWallet::new(
                    wallet.metadata.id,
                    wallet.record_id,
                    wallet.revision_hash,
                )
            })
            .collect();
        let cleanup_sources = merged_wallets
            .into_iter()
            .filter(|merged| merged.source.namespace_id != active_namespace_id)
            .map(|merged| merged.source)
            .collect::<Vec<_>>();

        Ok(CloudBackupEnableRecoveryCompletion {
            context: preparation.context,
            namespace_id: active_namespace_id,
            credential_id,
            prf_salt,
            active_critical_key,
            uploaded_wallets,
            pending_uploads,
            cleanup_sources,
        })
    }

    async fn load_enable_merge_namespaces(
        &self,
        cloud: &CloudStorageClient,
        matches: Vec<NamespaceMatch>,
    ) -> Result<Vec<MergeNamespace>, CloudBackupError> {
        let mut merge_namespaces = Vec::with_capacity(matches.len());

        for matched in matches {
            let wallet_record_ids =
                match cloud.list_wallet_backups(matched.namespace_id.clone()).await {
                    Ok(wallet_record_ids) => wallet_record_ids,
                    Err(error) => {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::Enable,
                            CloudBackupError::cloud_storage_context("list wallet backups", error),
                        ));
                    }
                };

            merge_namespaces.push(MergeNamespace { matched, wallet_record_ids });
        }

        Ok(merge_namespaces)
    }

    async fn restore_enable_merge_wallets(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[MergeNamespace],
    ) -> Result<Vec<MergedNamespaceWallets>, CloudBackupError> {
        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()
            .map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;
        let mut restore_session = WalletRestoreSession::new(existing_identities);
        let mut merged_namespaces = Vec::with_capacity(namespaces.len());

        for namespace in namespaces {
            let reader = WalletBackupReader::new(
                cloud.clone(),
                namespace.matched.namespace_id.clone(),
                Zeroizing::new(namespace.matched.master_key.critical_data_key()),
            );
            let mut expected_wallets = Vec::with_capacity(namespace.wallet_record_ids.len());
            let mut restored_wallets = Vec::new();

            for record_id in &namespace.wallet_record_ids {
                let wallet = match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: Some(wallet.entry.content_revision_hash.clone()),
                        });

                        wallet
                    }
                    Ok(WalletBackupLookup::NotFound) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!("Enable: matched namespace listed a missing wallet backup");
                        continue;
                    }
                    Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!(
                            "Enable: matched namespace uses unsupported wallet backup version {version}"
                        );
                        continue;
                    }
                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(BlockingCloudStep::Enable, error));
                        }
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!("Enable: failed to inspect wallet during namespace merge: {error}");
                        continue;
                    }
                };

                match restore_session.restore_downloaded(&wallet) {
                    Ok(WalletRestoreOutcome::Restored { .. }) => {
                        restored_wallets.push(wallet.metadata)
                    }
                    Ok(WalletRestoreOutcome::SkippedDuplicate) => {}
                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(BlockingCloudStep::Enable, error));
                        }
                        warn!("Enable: failed to restore wallet during namespace merge: {error}");
                    }
                }
            }

            merged_namespaces.push(MergedNamespaceWallets {
                source: CleanupSourceNamespace {
                    namespace_id: namespace.matched.namespace_id.clone(),
                    expected_wallets,
                },
                restored_wallets,
            });
        }

        Ok(merged_namespaces)
    }
}

fn active_merge_namespace_index(namespaces: &[MergeNamespace]) -> Option<usize> {
    namespaces
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left.wallet_record_ids
                .len()
                .cmp(&right.wallet_record_ids.len())
                .then_with(|| right.matched.namespace_id.cmp(&left.matched.namespace_id))
        })
        .map(|(index, _)| index)
}
