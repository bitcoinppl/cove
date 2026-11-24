use std::collections::HashSet;

use cove_cspp::backup_data::{WalletEntry, wallet_record_id};
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::keychain::Keychain;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::blocking_cloud_error;
use crate::backup::import::LabelRestoreWarning;
use crate::database::cloud_backup::{PersistedPendingSourceDelete, PersistedSourceReplacement};
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{
    DownloadedWalletBackup, NamespaceMatch, WalletBackupLookup, WalletBackupReader,
    WalletRestoreOutcome, WalletRestoreSession, backup_entry_holds,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_LABELS_WARNING_MESSAGE,
    CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
    current_namespace_wallet_record_ids, is_provider_wide_interruption,
};
use crate::wallet::metadata::WalletMetadata;
use crate::wallet_identity::NamedWalletCoverage;

/// What Recover Other Backups did with one source record
enum SourceRecordStep {
    /// The current backup's record under the same id already holds everything this one does
    Held(PersistedSourceReplacement),
    /// A local wallet was written from the record, and uploads with the namespace's batch
    Wrote {
        wallet: WalletMetadata,
        /// Whether that upload holds everything the source record does, so the record counts
        /// as recovered once the upload is confirmed; a wallet whose labels failed to import, or
        /// that kept its own labels over the record's when it gained a seed, leaves the source
        /// as the only copy of some of them
        replaces_source: bool,
    },
    /// Nothing was written, and the source keeps the record
    Kept,
}

/// The current backup, as Recover reads it to compare and merge source records
struct CurrentNamespace {
    id: String,
    record_ids: HashSet<String>,

    /// `None` when its master key is not on this device, so no source record can be proven held
    /// by the current backup, and no upload can write to it either
    reader: Option<WalletBackupReader>,
}

impl CurrentNamespace {
    /// The current backup's record `record_id`, when it lists one this device can read
    ///
    /// A record the current backup doesn't list is not read, since iCloud reports a read of a
    /// missing file as a pending sync, which would fail the run. A record from a newer version
    /// fails the read, since it may hold data this version can't see and an upload would replace
    async fn entry(&self, record_id: &str) -> Result<Option<WalletEntry>, CloudBackupError> {
        let Some(reader) = &self.reader else { return Ok(None) };
        if !self.record_ids.contains(record_id) {
            return Ok(None);
        }

        match reader.lookup_entry(record_id).await? {
            WalletBackupLookup::Found(entry) => Ok(Some(entry)),
            WalletBackupLookup::NotFound => Ok(None),
            WalletBackupLookup::UnsupportedVersion(version) => {
                Err(CloudBackupError::Compatibility(format!(
                    "read current wallet backup: unsupported wallet backup version {version}"
                )))
            }
        }
    }
}

impl RustCloudBackupManager {
    pub(crate) async fn restore_wallets_from_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: Vec<NamespaceMatch>,
        writes: &CloudBackupWriteClient,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let mut restore_session = self.restore_session()?;
        let record_ids = current_namespace_wallet_record_ids(
            cloud,
            &current_namespace,
            BlockingCloudStep::RecoverOtherBackups,
        )
        .await?;
        let mut current = CurrentNamespace {
            reader: current_namespace_reader(cloud, &current_namespace)?,
            record_ids: record_ids.into_iter().collect(),
            id: current_namespace,
        };
        let mut moved_namespace_count = 0;
        let mut report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        for namespace in namespaces {
            let source_record_ids = cloud
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

            // the current backup's records that hold source records, and the source records
            // whose replacement is the upload of a wallet written in this namespace's batch
            let mut replacements = Vec::new();
            let mut awaiting_upload = Vec::new();
            let mut written_wallets: Vec<WalletMetadata> = Vec::new();

            for record_id in &source_record_ids {
                let step = self
                    .recover_source_record(
                        &mut restore_session,
                        &reader,
                        &current,
                        record_id,
                        &mut report,
                    )
                    .await?;

                match step {
                    SourceRecordStep::Held(replacement) => replacements.push(replacement),
                    SourceRecordStep::Wrote { wallet, replaces_source } => {
                        if replaces_source {
                            awaiting_upload
                                .push((record_id.clone(), wallet_record_id(wallet.id.as_ref())));
                        }

                        // a wallet that gained a seed replaces any copy written earlier in this
                        // batch, which still has the watch-only metadata
                        written_wallets.retain(|written| written.id != wallet.id);
                        written_wallets.push(wallet);
                    }
                    SourceRecordStep::Kept => {}
                }
            }

            let uploaded =
                self.do_backup_wallets(writes, &written_wallets).await.map_err(|error| {
                    blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error)
                })?;
            current.record_ids.extend(uploaded.iter().map(|uploaded| uploaded.record_id.clone()));

            // a source record counts as replaced only by a record this batch actually wrote, at
            // the revision it wrote; a wallet whose cloud copy kept key material it lacks wrote
            // nothing
            replacements.extend(awaiting_upload.into_iter().filter_map(
                |(source_record_id, replacement_record_id)| {
                    let uploaded = uploaded
                        .iter()
                        .find(|uploaded| uploaded.record_id == replacement_record_id)?;

                    Some(PersistedSourceReplacement {
                        source_record_id,
                        replacement_record_id,
                        revision_hash: uploaded.revision_hash.clone(),
                    })
                },
            ));

            // the uploads returned, which on iOS can mean the files are only staged on this
            // device, so the source is recorded for deletion once the provider reports each
            // replacement uploaded rather than deleted here; a wallet restored under a new wallet
            // id counts its source record, but only for this namespace: another namespace's copy
            // of the same record may hold different data and is still read
            let replaced = |record_id: &String| {
                replacements.iter().any(|replacement| replacement.source_record_id == *record_id)
            };
            if source_record_ids.is_empty() || !source_record_ids.iter().all(replaced) {
                continue;
            }

            let pending = PersistedPendingSourceDelete {
                source_namespace_id: namespace.namespace_id.clone(),
                active_namespace_id: current.id.clone(),
                replacements,
            };
            self.record_pending_source_delete(writes, pending).await.map_err(|error| {
                blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error)
            })?;
            info!(
                "Recovered cloud backup namespace {} is deleted once its uploads confirm",
                namespace.namespace_id
            );
            moved_namespace_count += 1;
        }

        if report.wallets_restored == 0 && report.wallets_failed == 0 && moved_namespace_count == 0
        {
            return Err(CloudBackupError::Internal(
                "no wallets were found in the matching cloud backups".into(),
            ));
        }

        Ok(report)
    }

    /// Reads one source record and restores what the local wallets lack, counting what failed
    /// in `report`
    ///
    /// Only a provider-wide interruption is returned as an error, since it stops every later
    /// read as well
    async fn recover_source_record(
        &self,
        session: &mut WalletRestoreSession<'_>,
        reader: &WalletBackupReader,
        current: &CurrentNamespace,
        record_id: &str,
        report: &mut CloudBackupRestoreReport,
    ) -> Result<SourceRecordStep, CloudBackupError> {
        let wallet = match reader.lookup(record_id).await {
            Ok(WalletBackupLookup::Found(wallet)) => wallet,
            Ok(WalletBackupLookup::NotFound) => {
                warn!(
                    "Failed to recover wallet from other backup: listed wallet backup is missing"
                );
                report.wallets_failed += 1;
                report.failed_wallet_errors.push(CloudBackupError::NoBackupFound.reader_message());
                return Ok(SourceRecordStep::Kept);
            }
            Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                warn!(
                    "Failed to recover wallet from other backup: unsupported wallet backup version {version}"
                );
                report.wallets_failed += 1;
                report.failed_wallet_errors.push(CLOUD_BACKUP_COMPATIBILITY_MESSAGE.into());
                return Ok(SourceRecordStep::Kept);
            }
            Err(error) => return record_failure(report, error),
        };

        // the current backup's copy of a record is the one this device's wallet uploads over, so
        // a source copy under the same record id is restored only to add a seed a local wallet
        // lacks, since restoring it otherwise could upload an older copy over a newer one
        let secret_missing =
            matches!(session.coverage_of(&wallet), NamedWalletCoverage::SecretMissing);
        if current.record_ids.contains(record_id) && !secret_missing {
            return held_by_current(current, record_id, &wallet).await;
        }

        let plan = match session.plan(&wallet) {
            Ok(plan) => plan,
            Err(error) => return record_failure(report, error),
        };

        // a wallet gaining the seed uploads over the current backup's record under its id, so
        // that record's labels are read before the seed is added, and added where the wallet
        // has none; while they can't be read the seed stays only in the source, which keeps it
        let current_labels = match plan.secret_target() {
            Some(target) => match current.entry(&wallet_record_id(target.id.as_ref())).await {
                Ok(entry) => entry,
                Err(error) => return record_failure(report, error),
            },
            None => None,
        };

        let outcome = match session.commit(plan, current_labels.as_ref()) {
            Ok(outcome) => outcome,
            Err(error) => return record_failure(report, error),
        };

        let step = match outcome {
            WalletRestoreOutcome::Restored { wallet, labels_warning } => {
                report.wallets_restored += 1;
                let replaces_source = labels_warning.is_none();
                record_labels_warning(report, labels_warning);

                SourceRecordStep::Wrote { wallet: wallet.into_metadata(), replaces_source }
            }

            // labels the wallet already had keep the wallet's version over the source's, so the
            // source stays
            WalletRestoreOutcome::SecretAdded { wallet, labels_warning, .. } => {
                report.wallets_restored += 1;
                record_labels_warning(report, labels_warning);

                SourceRecordStep::Wrote { wallet: *wallet, replaces_source: false }
            }
            WalletRestoreOutcome::AlreadyOnDevice { .. } => SourceRecordStep::Kept,
        };

        Ok(step)
    }
}

/// Whether the current backup's record under `record_id` holds everything the source record
/// does, which counts the source record as recovered at that record's revision
async fn held_by_current(
    current: &CurrentNamespace,
    record_id: &str,
    source: &DownloadedWalletBackup,
) -> Result<SourceRecordStep, CloudBackupError> {
    let current_entry = match current.entry(record_id).await {
        Ok(entry) => entry,
        Err(error) if is_provider_wide_interruption(&error) => {
            return Err(blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error));
        }
        Err(error) => {
            warn!("Keeping other backup record, current copy unreadable: {error}");
            return Ok(SourceRecordStep::Kept);
        }
    };

    let Some(current_entry) = current_entry else { return Ok(SourceRecordStep::Kept) };
    if !backup_entry_holds(&current_entry, &source.entry) {
        return Ok(SourceRecordStep::Kept);
    }

    Ok(SourceRecordStep::Held(PersistedSourceReplacement {
        source_record_id: record_id.to_owned(),
        replacement_record_id: record_id.to_owned(),
        revision_hash: current_entry.content_revision_hash.clone(),
    }))
}

/// Counts a source record that failed to recover, unless the failure stops the whole run
fn record_failure(
    report: &mut CloudBackupRestoreReport,
    error: CloudBackupError,
) -> Result<SourceRecordStep, CloudBackupError> {
    if is_provider_wide_interruption(&error) {
        return Err(blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error));
    }

    warn!("Failed to recover wallet from other backup: {error}");
    report.wallets_failed += 1;
    report.failed_wallet_errors.push(error.reader_message());
    Ok(SourceRecordStep::Kept)
}

fn record_labels_warning(
    report: &mut CloudBackupRestoreReport,
    warning: Option<LabelRestoreWarning>,
) {
    let Some(warning) = warning else { return };

    report.labels_failed_wallet_names.push(warning.wallet_name);
    report.labels_failed_errors.push(CLOUD_BACKUP_LABELS_WARNING_MESSAGE.into());
}

/// A reader for the current namespace, or `None` when its master key is not on this device
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
