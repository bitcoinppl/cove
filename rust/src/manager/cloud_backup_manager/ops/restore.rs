use std::collections::HashSet;

use cove_cspp::backup_data::WalletSecret as CloudWalletSecret;
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::keychain::Keychain;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::blocking_cloud_error;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{
    DownloadedWalletBackup, NamespaceMatch, WalletBackupLookup, WalletBackupReader,
    WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_LABELS_WARNING_MESSAGE,
    CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
    current_namespace_wallet_record_ids, is_provider_wide_interruption,
};
use crate::wallet_identity::LocalWalletCoverage;

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
        let current_reader = current_namespace_reader(cloud, &current_namespace)?;
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

                // the current backup's copy of a record is the one this device's wallet uploads
                // over, so a source copy under the same record id is restored only to add a seed a
                // local wallet lacks, since restoring it otherwise could upload an older copy over
                // a newer one, and counts as recovered only once the current copy holds everything
                // it does
                if current_wallet_record_ids.contains(record_id)
                    && restore_session.coverage_of(&wallet) != LocalWalletCoverage::SecretMissing
                {
                    match current_record_holds(current_reader.as_ref(), record_id, &wallet).await {
                        Ok(true) => {
                            recovered_record_ids.insert(record_id.clone());
                        }
                        Ok(false) => {}
                        Err(error) if is_provider_wide_interruption(&error) => {
                            return Err(blocking_cloud_error(
                                BlockingCloudStep::RecoverOtherBackups,
                                error,
                            ));
                        }
                        Err(error) => {
                            warn!("Keeping other backup record, current copy unreadable: {error}");
                        }
                    }

                    continue;
                }

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

/// A reader for the current namespace, or `None` when its master key is not on this device, in
/// which case no source record can be proven held by the current backup
fn current_namespace_reader(
    cloud: &CloudStorageClient,
    namespace: &str,
) -> Result<Option<WalletBackupReader>, CloudBackupError> {
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .map_err(|source| CloudBackupError::internal_context("load local master key", source))?
        .filter(|master_key| master_key.namespace_id() == namespace);

    Ok(master_key.map(|master_key| {
        WalletBackupReader::new(
            cloud.clone(),
            namespace.to_owned(),
            Zeroizing::new(master_key.critical_data_key()),
        )
    }))
}

/// Whether the current backup's record under `record_id` holds everything `source` holds: its
/// key material and, when it has labels, the same labels
///
/// A matching record id alone proves nothing, since the current copy can be watch-only while
/// the source holds the seed, or hold none of the source's labels
async fn current_record_holds(
    current_reader: Option<&WalletBackupReader>,
    record_id: &str,
    source: &DownloadedWalletBackup,
) -> Result<bool, CloudBackupError> {
    let Some(current_reader) = current_reader else { return Ok(false) };
    let WalletBackupLookup::Found(current) = current_reader.lookup_entry(record_id).await? else {
        return Ok(false);
    };

    let source = &source.entry;
    let holds_key_material = match (&source.secret, &current.secret) {
        (CloudWalletSecret::Mnemonic(source), CloudWalletSecret::Mnemonic(current))
        | (CloudWalletSecret::Xprv(source), CloudWalletSecret::Xprv(current)) => source == current,
        (
            CloudWalletSecret::TapSignerBackup(source),
            CloudWalletSecret::TapSignerBackup(current),
        ) => source == current,
        (CloudWalletSecret::Descriptor(_) | CloudWalletSecret::WatchOnly, _) => true,
        _ => false,
    };
    let holds_labels = source.labels_count == 0
        || (source.labels_hash.is_some() && source.labels_hash == current.labels_hash);

    Ok(holds_key_material && holds_labels)
}
