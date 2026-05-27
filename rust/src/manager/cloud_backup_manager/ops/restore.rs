use std::collections::HashSet;

use cove_device::cloud_storage::CloudStorageClient;
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::blocking_cloud_error;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome,
    WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
    current_namespace_wallet_record_ids, is_connectivity_related_issue,
};

impl RustCloudBackupManager {
    pub(crate) async fn restore_wallets_from_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: Vec<NamespaceMatch>,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()
            .map_err_prefix("collect wallet identities", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_identities);
        let mut current_wallet_record_ids: HashSet<_> = current_namespace_wallet_record_ids(
            cloud,
            &current_namespace,
            BlockingCloudStep::RecoverOtherBackups,
        )
        .await?
        .into_iter()
        .collect();
        let mut moved_namespace_count = 0;
        let mut report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        for namespace in namespaces {
            let wallet_record_ids = cloud
                .list_wallet_backups(namespace.namespace_id.clone())
                .await
                .map_err(|error| {
                    blocking_cloud_error(
                        BlockingCloudStep::RecoverOtherBackups,
                        CloudBackupError::cloud_storage_context("list wallet backups", error),
                    )
                })?;

            let reader = WalletBackupReader::new(
                cloud.clone(),
                namespace.namespace_id.clone(),
                Zeroizing::new(namespace.master_key.critical_data_key()),
            );

            let mut restored_wallets = Vec::new();

            for record_id in &wallet_record_ids {
                if current_wallet_record_ids.contains(record_id) {
                    continue;
                }

                let wallet = match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => wallet,
                    Ok(WalletBackupLookup::NotFound) => {
                        warn!(
                            "Failed to recover wallet {}/{} from other backup: listed wallet backup is missing",
                            namespace.namespace_id, record_id
                        );
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(format!(
                            "{} was listed but missing from cloud backup",
                            record_id
                        ));
                        continue;
                    }

                    Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                        warn!(
                            "Failed to recover wallet {}/{} from other backup: unsupported wallet backup version {version}",
                            namespace.namespace_id, record_id
                        );
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(format!(
                            "{record_id} uses unsupported wallet backup version {version}"
                        ));
                        continue;
                    }

                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(
                                BlockingCloudStep::RecoverOtherBackups,
                                error,
                            ));
                        }
                        warn!(
                            "Failed to recover wallet {}/{} from other backup: {error}",
                            namespace.namespace_id, record_id
                        );
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(error.to_string());
                        continue;
                    }
                };

                match restore_session.restore_downloaded(&wallet) {
                    Ok(WalletRestoreOutcome::Restored { labels_warning }) => {
                        report.wallets_restored += 1;
                        if let Some(warning) = labels_warning {
                            report.labels_failed_wallet_names.push(warning.wallet_name);
                            report.labels_failed_errors.push(warning.error);
                        }

                        restored_wallets.push(wallet.metadata);
                    }
                    Ok(WalletRestoreOutcome::SkippedDuplicate) => {}

                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(
                                BlockingCloudStep::RecoverOtherBackups,
                                error,
                            ));
                        }
                        warn!(
                            "Failed to recover wallet {}/{} from other backup: {error}",
                            namespace.namespace_id, record_id
                        );
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(error.to_string());
                    }
                };
            }

            if !restored_wallets.is_empty() {
                self.do_backup_wallets(&restored_wallets).await.map_err(|error| {
                    blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error)
                })?;
                current_wallet_record_ids.extend(restored_wallets.iter().map(|metadata| {
                    cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref())
                }));
            }

            if !wallet_record_ids.is_empty()
                && wallet_record_ids
                    .iter()
                    .all(|record_id| current_wallet_record_ids.contains(record_id))
            {
                cloud.delete_namespace(namespace.namespace_id.clone()).await.map_err(|error| {
                    blocking_cloud_error(
                        BlockingCloudStep::RecoverOtherBackups,
                        CloudBackupError::cloud_storage_context(
                            "delete recovered cloud backup namespace",
                            error,
                        ),
                    )
                })?;
                info!("Deleted recovered cloud backup namespace {}", namespace.namespace_id);
                moved_namespace_count += 1;
            }
        }

        if report.wallets_restored == 0 && report.wallets_failed == 0 && moved_namespace_count == 0
        {
            return Err(CloudBackupError::Internal(
                "no wallets were found in the matching cloud backups".into(),
            ));
        }

        Ok(report)
    }
}
