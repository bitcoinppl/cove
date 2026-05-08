use std::collections::HashSet;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{blocking_cloud_error, try_restore_from_local_master_key};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::wallets::{
    DownloadedWalletBackup, NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher,
    WalletBackupLookup, WalletBackupReader, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::workers::RestoredPasskeyMaterial;
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupRestoreProgress,
    CloudBackupRestoreReport, CloudBackupRestoreStage, CloudBackupStatus, CloudStorageIssue,
    RestoreOperation, RustCloudBackupManager, current_namespace_wallet_record_ids,
    is_connectivity_related_issue,
};

struct RestorableNamespace {
    namespace_id: String,
    master_key: cove_cspp::master_key::MasterKey,
    passkey: Option<RestorableNamespacePasskey>,
}

#[derive(Clone)]
struct RestorableNamespacePasskey {
    credential_id: Vec<u8>,
    prf_salt: [u8; 32],
}

struct RestoreDownloadProgress {
    completed: u32,
    total: u32,
}

impl RustCloudBackupManager {
    pub(crate) async fn do_restore_from_cloud_backup(
        &self,
        operation: &RestoreOperation,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Restore)?;
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_restore_report(None);
        self.set_status_for_restore_operation(operation, CloudBackupStatus::Restoring).await?;
        self.send_restore_progress(operation, CloudBackupRestoreStage::Finding, 0, None).await?;

        let cloud = CloudStorage::global_explicit_client();
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());

        // passkey matching first, local master key as fallback
        let passkey = PasskeyAccess::global();
        let restorable_namespaces = match self.restore_via_passkey_matching(&cloud, passkey).await {
            Ok(matches) => {
                if matches.is_empty() {
                    return Err(CloudBackupError::PasskeyMismatch);
                }

                matches
                    .into_iter()
                    .map(|matched| RestorableNamespace {
                        namespace_id: matched.namespace_id,
                        master_key: matched.master_key,
                        passkey: Some(RestorableNamespacePasskey {
                            credential_id: matched.credential_id,
                            prf_salt: matched.prf_salt,
                        }),
                    })
                    .collect::<Vec<_>>()
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                info!("Restore: passkey discovery cancelled");
                return Err(CloudBackupError::PasskeyDiscoveryCancelled);
            }
            Err(CloudBackupError::PasskeyMismatch) => {
                info!("Restore: passkey didn't match, trying local master key fallback");
                let (master_key, namespace_id) = try_restore_from_local_master_key(&cloud, &cspp)
                    .await
                    .map_err(|error| blocking_cloud_error(BlockingCloudStep::Restore, error))?
                    .ok_or(CloudBackupError::PasskeyMismatch)?;
                vec![RestorableNamespace { namespace_id, master_key, passkey: None }]
            }
            Err(e) => return Err(e),
        };

        self.ensure_current_restore_operation(operation).await?;
        let mut namespace_wallets = Vec::with_capacity(restorable_namespaces.len());
        let mut wallet_count = 0;

        for namespace in restorable_namespaces {
            let wallet_record_ids = cloud
                .list_wallet_backups(namespace.namespace_id.clone())
                .await
                .map_err(|error| {
                    blocking_cloud_error(
                        BlockingCloudStep::Restore,
                        CloudBackupError::cloud_storage_context("list wallet backups", error),
                    )
                })?;
            wallet_count += wallet_record_ids.len() as u32;
            namespace_wallets.push((namespace, wallet_record_ids));
        }

        let mut report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        let existing_fingerprints = crate::backup::import::collect_existing_fingerprints()
            .map_err_prefix("collect fingerprints", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);
        let mut downloaded_wallets = Vec::new();
        let mut download_progress = RestoreDownloadProgress { completed: 0, total: wallet_count };

        self.send_restore_progress(
            operation,
            CloudBackupRestoreStage::Downloading,
            0,
            Some(wallet_count),
        )
        .await?;

        for (namespace_index, (namespace, wallet_record_ids)) in
            namespace_wallets.iter().enumerate()
        {
            let reader = WalletBackupReader::new(
                cloud.clone(),
                namespace.namespace_id.clone(),
                Zeroizing::new(namespace.master_key.critical_data_key()),
            );
            let namespace_downloaded = self
                .download_wallets_for_restore(
                    operation,
                    &reader,
                    &namespace.namespace_id,
                    wallet_record_ids,
                    &mut report,
                    &mut download_progress,
                )
                .await?;

            downloaded_wallets.extend(
                namespace_downloaded.into_iter().map(|downloaded| (namespace_index, downloaded)),
            );
        }

        let restore_total = downloaded_wallets.len() as u32;

        self.send_restore_progress(
            operation,
            CloudBackupRestoreStage::Restoring,
            0,
            Some(restore_total),
        )
        .await?;

        let mut first_success_namespace_index = None;
        for (index, (namespace_index, (record_id, wallet))) in downloaded_wallets.iter().enumerate()
        {
            operation.ensure_current().await?;
            match restore_session.restore_downloaded(wallet) {
                Ok(outcome) => {
                    first_success_namespace_index.get_or_insert(*namespace_index);
                    report.wallets_restored += 1;
                    if let Some(warning) = outcome.labels_warning {
                        report.labels_failed_wallet_names.push(warning.wallet_name);
                        report.labels_failed_errors.push(warning.error);
                    }
                }
                Err(CloudBackupError::Cancelled) => return Err(CloudBackupError::Cancelled),
                Err(error) => {
                    warn!("Failed to restore wallet {record_id}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error.to_string());
                }
            }

            self.send_restore_progress(
                operation,
                CloudBackupRestoreStage::Restoring,
                (index + 1) as u32,
                Some(restore_total),
            )
            .await?;
        }

        if report.wallets_restored == 0 && report.wallets_failed > 0 {
            self.set_restore_progress_for_restore_operation(operation, None).await?;
            self.set_restore_report_for_restore_operation(operation, Some(report)).await?;
            return Err(CloudBackupError::Internal("all wallets failed to restore".into()));
        }

        let now = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let state = PersistedCloudBackupState::default()
            .mark_enabled_preserving_verification(now, wallet_count);
        self.persist_cloud_backup_state_for_restore_operation(
            operation,
            &state,
            "persist restored cloud backup state",
        )
        .await?;
        if let Some(active_namespace_index) = first_success_namespace_index
            && let Some((active, _)) = namespace_wallets.get(active_namespace_index)
        {
            let master_key =
                cove_cspp::master_key::MasterKey::from_bytes(*active.master_key.as_bytes());
            let passkey = active.passkey.as_ref().map(|passkey| RestoredPasskeyMaterial {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
            });
            operation.save_keychain_state(master_key, passkey, active.namespace_id.clone()).await?;
        }

        self.set_restore_progress_for_restore_operation(operation, None).await?;
        self.set_restore_report_for_restore_operation(operation, Some(report)).await?;
        self.set_status_for_restore_operation(operation, CloudBackupStatus::Enabled).await?;

        info!("Cloud backup restore complete");
        Ok(())
    }

    async fn send_restore_progress(
        &self,
        operation: &RestoreOperation,
        stage: CloudBackupRestoreStage,
        completed: u32,
        total: Option<u32>,
    ) -> Result<(), CloudBackupError> {
        self.set_restore_progress_for_restore_operation(
            operation,
            Some(CloudBackupRestoreProgress { stage, completed, total }),
        )
        .await
    }

    async fn download_wallets_for_restore(
        &self,
        operation: &RestoreOperation,
        reader: &WalletBackupReader,
        namespace_id: &str,
        wallet_record_ids: &[String],
        report: &mut CloudBackupRestoreReport,
        progress: &mut RestoreDownloadProgress,
    ) -> Result<Vec<(String, DownloadedWalletBackup)>, CloudBackupError> {
        let mut downloaded_wallets = Vec::with_capacity(wallet_record_ids.len());
        let mut lookups = stream::iter(
            wallet_record_ids
                .iter()
                .cloned()
                .map(|record_id| Self::lookup_wallet_backup(reader.clone(), record_id)),
        )
        .buffered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, lookup)) = lookups.next().await {
            self.ensure_current_restore_operation(operation).await?;
            let record_name = format!("{namespace_id}/{record_id}");

            match lookup {
                Ok(WalletBackupLookup::Found(wallet)) => {
                    downloaded_wallets.push((record_name.clone(), wallet));
                }
                Ok(WalletBackupLookup::NotFound) => {
                    let error =
                        format!("wallet {record_name} was listed but missing from cloud backup");
                    warn!("Failed to download wallet {record_name}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    let error = format!(
                        "wallet {record_name} uses unsupported wallet backup version {version}"
                    );
                    warn!("Failed to download wallet {record_name}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Err(error) => {
                    if is_connectivity_related_issue(CloudStorageIssue::from(&error)) {
                        return Err(blocking_cloud_error(BlockingCloudStep::Restore, error));
                    }
                    warn!("Failed to download wallet {record_name}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error.to_string());
                }
            }

            progress.completed += 1;

            self.send_restore_progress(
                operation,
                CloudBackupRestoreStage::Downloading,
                progress.completed,
                Some(progress.total),
            )
            .await?;
        }

        Ok(downloaded_wallets)
    }

    /// Restore via passkey-based namespace matching (fresh device path)
    ///
    /// Tries the selected passkey across all downloaded namespaces. If it
    /// doesn't match any of them, returns `PasskeyMismatch` so the caller can
    /// try local master key fallback or prompt the user to try a different
    /// passkey
    async fn restore_via_passkey_matching(
        &self,
        cloud: &CloudStorageClient,
        passkey: &PasskeyAccess,
    ) -> Result<Vec<NamespaceMatch>, CloudBackupError> {
        let mut namespaces = cloud.list_namespaces().await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::Restore,
                CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
            )
        })?;
        namespaces.sort();
        if namespaces.is_empty() {
            return Err(CloudBackupError::Internal("no cloud backup namespaces found".into()));
        }

        info!("Restore: authenticating with passkey across {} namespace(s)", namespaces.len());

        let matcher = NamespacePasskeyMatcher::new(cloud, passkey);
        let match_outcome = matcher.match_namespaces(&namespaces).await?;
        match match_outcome {
            NamespaceMatchOutcome::Matched(matches) => {
                info!("Restore: matched {} namespace(s)", matches.len());
                Ok(matches)
            }
            NamespaceMatchOutcome::UserDeclined => Err(CloudBackupError::PasskeyDiscoveryCancelled),
            NamespaceMatchOutcome::NoMatch => Err(CloudBackupError::PasskeyMismatch),
            NamespaceMatchOutcome::Inconclusive => {
                Err(self.offline_error_for_step(BlockingCloudStep::Restore))
            }
            NamespaceMatchOutcome::UnsupportedVersions => Err(CloudBackupError::Internal(
                "some cloud backups use a newer format, please update the app".into(),
            )),
        }
    }

    pub(crate) async fn restore_wallets_from_namespaces(
        &self,
        cloud: &CloudStorageClient,
        namespaces: Vec<NamespaceMatch>,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let existing_fingerprints = crate::backup::import::collect_existing_fingerprints()
            .map_err_prefix("collect fingerprints", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);
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

                match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => {
                        match restore_session.restore_downloaded(&wallet) {
                            Ok(outcome) => {
                                report.wallets_restored += 1;
                                if let Some(warning) = outcome.labels_warning {
                                    report.labels_failed_wallet_names.push(warning.wallet_name);
                                    report.labels_failed_errors.push(warning.error);
                                }

                                restored_wallets.push(wallet.metadata);
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
                            }
                        }
                    }
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
                    }
                }
            }

            if !restored_wallets.is_empty() {
                self.do_backup_wallets(&restored_wallets).await.map_err(|error| {
                    blocking_cloud_error(BlockingCloudStep::RecoverOtherBackups, error)
                })?;
                current_wallet_record_ids = current_namespace_wallet_record_ids(
                    cloud,
                    &current_namespace,
                    BlockingCloudStep::RecoverOtherBackups,
                )
                .await?
                .into_iter()
                .collect();
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
