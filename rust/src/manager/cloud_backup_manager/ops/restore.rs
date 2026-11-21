use std::collections::HashSet;

use cove_device::cloud_storage::CloudStorageClient;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::blocking_cloud_error;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome,
    WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_LABELS_WARNING_MESSAGE,
    CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
    current_namespace_wallet_record_ids, is_provider_wide_interruption,
};

impl RustCloudBackupManager {
    pub(crate) async fn restore_wallets_from_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: Vec<NamespaceMatch>,
        writes: &CloudBackupWriteClient,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()?;
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

            // the source records this device and the current backup now hold, so the namespace
            // may be deleted once they are all of its records
            let mut recovered_record_ids = HashSet::new();

            // a record under an id the current backup also holds is still read: the current copy
            // may be watch-only while this one holds the seed
            for record_id in &wallet_record_ids {
                let wallet = match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => wallet,
                    Ok(WalletBackupLookup::NotFound) => {
                        warn!(
                            "Failed to recover wallet from other backup: listed wallet backup is missing"
                        );
                        report.wallets_failed += 1;
                        report
                            .failed_wallet_errors
                            .push(CloudBackupError::NoBackupFound.reader_message());
                        continue;
                    }

                    Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                        warn!(
                            "Failed to recover wallet from other backup: unsupported wallet backup version {version}"
                        );
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(CLOUD_BACKUP_COMPATIBILITY_MESSAGE.into());
                        continue;
                    }

                    Err(error) => {
                        if is_provider_wide_interruption(&error) {
                            return Err(blocking_cloud_error(
                                BlockingCloudStep::RecoverOtherBackups,
                                error,
                            ));
                        }
                        warn!("Failed to recover wallet from other backup: {error}");
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(error.reader_message());
                        continue;
                    }
                };

                match self.restore_in_session(&mut restore_session, &wallet) {
                    Ok(WalletRestoreOutcome::Restored { wallet: restored, labels_warning }) => {
                        report.wallets_restored += 1;
                        if let Some(warning) = labels_warning {
                            report.labels_failed_wallet_names.push(warning.wallet_name);
                            report
                                .labels_failed_errors
                                .push(CLOUD_BACKUP_LABELS_WARNING_MESSAGE.into());
                        }

                        restored_wallets.push(restored.into_metadata());
                        recovered_record_ids.insert(record_id.clone());
                    }

                    // the upgraded wallet uploads with this batch, replacing any copy restored
                    // earlier in it, which still has the watch-only metadata; the source stays,
                    // since adding the secret imported none of its labels
                    Ok(WalletRestoreOutcome::SecretAdded { wallet: upgraded }) => {
                        report.wallets_restored += 1;
                        restored_wallets.retain(|restored| restored.id != upgraded.id);
                        restored_wallets.push(*upgraded);
                    }

                    // the local wallet holds everything this record can recover and the current
                    // backup holds a record under its id; the id alone proves nothing, since the
                    // current copy can be watch-only while this one holds the seed
                    Ok(WalletRestoreOutcome::SkippedDuplicate)
                        if current_wallet_record_ids.contains(record_id) =>
                    {
                        recovered_record_ids.insert(record_id.clone());
                    }
                    Ok(WalletRestoreOutcome::SkippedDuplicate) => {}

                    Err(error) => {
                        if is_provider_wide_interruption(&error) {
                            return Err(blocking_cloud_error(
                                BlockingCloudStep::RecoverOtherBackups,
                                error,
                            ));
                        }
                        warn!("Failed to recover wallet from other backup: {error}");
                        report.wallets_failed += 1;
                        report.failed_wallet_errors.push(error.reader_message());
                    }
                };
            }

            if !restored_wallets.is_empty() {
                self.do_backup_wallets(writes, &restored_wallets).await.map_err(|error| {
                    blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error)
                })?;
                current_wallet_record_ids.extend(restored_wallets.iter().map(|metadata| {
                    cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref())
                }));
            }

            // reached only once the restored wallets uploaded, so every restored record is backed
            // up; a wallet restored under a new wallet id is backed up under a new record id, so
            // its source record counts here, but only for this namespace: another namespace's
            // copy of the same record may hold different data and is still read
            if !wallet_record_ids.is_empty()
                && wallet_record_ids
                    .iter()
                    .all(|record_id| recovered_record_ids.contains(record_id))
            {
                writes
                    .delete_other_namespace(cloud.clone(), namespace.namespace_id.clone())
                    .await
                    .map_err(|error| {
                        blocking_cloud_error(
                            BlockingCloudStep::RecoverOtherBackups,
                            error.with_cloud_storage_context(
                                "delete recovered cloud backup namespace",
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
