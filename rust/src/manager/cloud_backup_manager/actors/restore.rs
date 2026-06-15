use act_zero::{Addr, call};
use cove_cspp::master_key::MasterKey;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::ops::try_restore_from_local_master_key;
use crate::manager::cloud_backup_manager::wallets::{
    DownloadedWalletBackup, NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher,
    WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};

use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupEnableOutcome, CloudBackupError,
    CloudBackupRestoreFlow, CloudBackupRestoreOutcome, CloudBackupRestoreReport, CloudBackupStatus,
    CloudBackupStore, CloudStorageIssue, RustCloudBackupManager, blocking_cloud_error,
    is_connectivity_related_issue, offline_error_for_step,
};

use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

use super::{
    CloudBackupSupervisor, CloudBackupUploadedWallet, CloudBackupUploadedWalletsStateMode,
    CloudBackupWriteClient,
};

#[derive(Clone, Debug)]
pub(crate) enum CloudBackupRestoreEvent {
    Progress(CloudBackupRestoreFlow),
    Complete(CloudBackupRestoreReport),
    NoBackupFound,
    Failed(String),
}

struct RestorableNamespace {
    namespace_id: String,
    master_key: MasterKey,
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

#[derive(Debug, Clone, Copy)]
enum RestoreProgressPhase {
    Downloading,
    Restoring,
}

fn restore_progress_flow(
    phase: RestoreProgressPhase,
    completed: u32,
    total: u32,
) -> CloudBackupRestoreFlow {
    if total == 0 {
        return CloudBackupRestoreFlow::Finding;
    }

    match phase {
        RestoreProgressPhase::Downloading => {
            CloudBackupRestoreFlow::Downloading { completed, total }
        }
        RestoreProgressPhase::Restoring => CloudBackupRestoreFlow::Restoring { completed, total },
    }
}

async fn lookup_wallet_backup(
    reader: WalletBackupReader,
    record_id: String,
) -> (String, Result<WalletBackupLookup<DownloadedWalletBackup>, CloudBackupError>) {
    let lookup = reader.lookup(&record_id).await;
    (record_id, lookup)
}

#[derive(Clone, Debug)]
pub(crate) struct RestoreOperation {
    operation_claim: CloudBackupExclusiveOperationClaim,
    supervisor: Addr<CloudBackupSupervisor>,
    event_sender: Option<flume::Sender<CloudBackupRestoreEvent>>,
}

impl RestoreOperation {
    pub(crate) fn new(
        operation_claim: CloudBackupExclusiveOperationClaim,
        supervisor: Addr<CloudBackupSupervisor>,
    ) -> Self {
        Self { operation_claim, supervisor, event_sender: None }
    }

    pub(crate) fn new_with_events(
        operation_claim: CloudBackupExclusiveOperationClaim,
        supervisor: Addr<CloudBackupSupervisor>,
        event_sender: flume::Sender<CloudBackupRestoreEvent>,
    ) -> Self {
        Self { operation_claim, supervisor, event_sender: Some(event_sender) }
    }

    pub(crate) async fn ensure_current(&self) -> Result<(), CloudBackupError> {
        call!(self.supervisor.ensure_restore_current(self.operation_claim))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn apply_status(
        &self,
        status: CloudBackupStatus,
    ) -> Result<(), CloudBackupError> {
        call!(self.supervisor.apply_restore_status(self.operation_claim, status))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn apply_outcome(
        &self,
        outcome: CloudBackupRestoreOutcome,
    ) -> Result<(), CloudBackupError> {
        let progress = match &outcome {
            CloudBackupRestoreOutcome::ProgressReported(progress) => Some(progress.clone()),
            CloudBackupRestoreOutcome::ProgressCleared => None,
        };
        call!(self.supervisor.apply_restore_outcome(self.operation_claim, outcome))
            .await
            .map_err(|_| CloudBackupError::Cancelled)??;

        if let Some(progress) = progress {
            self.send_event_if_current(CloudBackupRestoreEvent::Progress(progress)).await;
        }

        Ok(())
    }

    pub(crate) async fn apply_enable_outcome(
        &self,
        outcome: CloudBackupEnableOutcome,
    ) -> Result<(), CloudBackupError> {
        call!(self.supervisor.apply_restore_enable_outcome(self.operation_claim, outcome))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn send_event_if_current(&self, event: CloudBackupRestoreEvent) {
        if self.ensure_current().await.is_err() {
            return;
        }

        let Some(sender) = &self.event_sender else {
            return;
        };

        if let Err(error) = sender.send_async(event).await {
            warn!("restore_from_cloud_backup: failed to send restore event: {error}");
        }
    }

    pub(crate) async fn persist_cloud_backup_state(
        &self,
        state: PersistedCloudBackupState,
        context: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.supervisor.persist_restore_cloud_backup_state(
            self.operation_claim,
            state,
            context
        ))
        .await
        .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn save_keychain_state(
        &self,
        master_key: MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.supervisor.save_restore_keychain_state(
            self.operation_claim,
            master_key,
            passkey,
            namespace_id
        ))
        .await
        .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn restore_from_cloud_backup(
        &self,
        manager: &RustCloudBackupManager,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        manager.ensure_cloud_connectivity(BlockingCloudStep::Restore)?;
        self.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared).await?;
        self.apply_outcome(CloudBackupRestoreOutcome::ProgressCleared).await?;
        self.apply_status(CloudBackupStatus::Restoring).await?;
        self.send_restore_progress(CloudBackupRestoreFlow::Finding).await?;

        let cloud = CloudStorage::global_explicit_client();
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());

        // passkey matching first, local master key as fallback
        let passkey = PasskeyAccess::global();
        let restorable_namespaces = match self.restore_via_passkey_matching(&cloud, passkey).await {
            Ok(matches) => matches
                .into_iter()
                .map(|matched| RestorableNamespace {
                    namespace_id: matched.namespace_id,
                    master_key: matched.master_key,
                    passkey: Some(RestorableNamespacePasskey {
                        credential_id: matched.credential_id,
                        prf_salt: matched.prf_salt,
                    }),
                })
                .collect::<Vec<_>>(),
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
            Err(error) => return Err(error),
        };

        self.ensure_current().await?;
        let mut namespace_wallets = Vec::with_capacity(restorable_namespaces.len());
        let mut listed_wallet_count = 0;

        for namespace in restorable_namespaces {
            let wallet_record_ids =
                match cloud.list_wallet_backups(namespace.namespace_id.clone()).await {
                    Ok(wallet_record_ids) => wallet_record_ids,
                    Err(error) => {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::Restore,
                            CloudBackupError::cloud_storage_context("list wallet backups", error),
                        ));
                    }
                };
            listed_wallet_count += wallet_record_ids.len() as u32;
            namespace_wallets.push((namespace, wallet_record_ids));
        }

        let mut report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        };

        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()
            .map_err_prefix("collect wallet identities", CloudBackupError::Internal)?;

        let mut restore_session = WalletRestoreSession::new(existing_identities);
        let mut downloaded_wallets = Vec::new();
        let mut download_progress =
            RestoreDownloadProgress { completed: 0, total: listed_wallet_count };

        self.send_restore_progress(restore_progress_flow(
            RestoreProgressPhase::Downloading,
            0,
            listed_wallet_count,
        ))
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
        self.send_restore_progress(restore_progress_flow(
            RestoreProgressPhase::Restoring,
            0,
            restore_total,
        ))
        .await?;

        let mut first_success_namespace_index = None;
        let mut skipped_duplicate_count = 0;
        for (index, (namespace_index, (_record_id, wallet))) in
            downloaded_wallets.iter().enumerate()
        {
            self.ensure_current().await?;
            match restore_session.restore_downloaded(wallet) {
                Ok(WalletRestoreOutcome::Restored { labels_warning }) => {
                    first_success_namespace_index.get_or_insert(*namespace_index);
                    report.wallets_restored += 1;
                    if let Some(warning) = labels_warning {
                        report.labels_failed_wallet_names.push(warning.wallet_name);
                        report.labels_failed_errors.push(warning.error);
                    }
                }
                Ok(WalletRestoreOutcome::SkippedDuplicate) => {
                    skipped_duplicate_count += 1;
                }
                Err(CloudBackupError::Cancelled) => return Err(CloudBackupError::Cancelled),
                Err(error) => {
                    warn!("Failed to restore wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error.to_string());
                }
            }

            self.send_restore_progress(restore_progress_flow(
                RestoreProgressPhase::Restoring,
                (index + 1) as u32,
                restore_total,
            ))
            .await?;
        }

        if report.wallets_restored == 0 && report.wallets_failed > 0 && skipped_duplicate_count == 0
        {
            self.apply_outcome(CloudBackupRestoreOutcome::ProgressCleared).await?;
            return Err(CloudBackupError::Internal("all wallets failed to restore".into()));
        }

        let active_namespace = first_success_namespace_index
            .and_then(|index| namespace_wallets.get(index))
            .map(|(namespace, _)| namespace);

        let restored_status = match active_namespace {
            Some(active) => {
                self.activate_restored_namespace(manager, &cloud, active).await?;

                CloudBackupStatus::Enabled
            }

            None if skipped_duplicate_count > 0 => {
                let state = RustCloudBackupManager::load_persisted_state();

                RustCloudBackupManager::runtime_status_for(&state)
            }

            None => {
                self.persist_cloud_backup_state(
                    PersistedCloudBackupState::default(),
                    "persist empty restored cloud backup state".into(),
                )
                .await?;

                CloudBackupStatus::Disabled
            }
        };

        self.apply_outcome(CloudBackupRestoreOutcome::ProgressCleared).await?;
        self.apply_status(restored_status).await?;

        info!("Cloud backup restore complete");
        Ok(report)
    }

    async fn send_restore_progress(
        &self,
        flow: CloudBackupRestoreFlow,
    ) -> Result<(), CloudBackupError> {
        self.apply_outcome(CloudBackupRestoreOutcome::ProgressReported(flow)).await
    }

    async fn activate_restored_namespace(
        &self,
        manager: &RustCloudBackupManager,
        cloud: &CloudStorageClient,
        active: &RestorableNamespace,
    ) -> Result<(), CloudBackupError> {
        let critical_key = Zeroizing::new(active.master_key.critical_data_key());
        let writes = CloudBackupWriteClient::for_operation(
            manager.cloud_writes.clone(),
            self.operation_claim,
        );

        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&writes, cloud, &active.namespace_id, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Restore, error))?;

        let master_key = MasterKey::from_bytes(*active.master_key.as_bytes());
        let passkey = active.passkey.as_ref().map(RestoredPasskeyMaterial::from);

        self.save_keychain_state(master_key, passkey, active.namespace_id.clone()).await?;

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

        writes
            .finalize_uploaded_wallets(
                cloud.clone(),
                active.namespace_id.clone(),
                uploaded_wallets,
                CloudBackupUploadedWalletsStateMode::PreserveVerificationWithUploadedCount,
            )
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Restore, error))?;

        Ok(())
    }

    async fn download_wallets_for_restore(
        &self,
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
                .map(|record_id| lookup_wallet_backup(reader.clone(), record_id)),
        )
        .buffered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, lookup)) = lookups.next().await {
            self.ensure_current().await?;
            let record_name = format!("{namespace_id}/{record_id}");

            match lookup {
                Ok(WalletBackupLookup::Found(wallet)) => {
                    downloaded_wallets.push((record_name.clone(), wallet));
                }
                Ok(WalletBackupLookup::NotFound) => {
                    let error = "wallet was listed but missing from cloud backup".to_string();
                    warn!("Failed to download wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    let error = format!("wallet uses unsupported wallet backup version {version}");
                    warn!("Failed to download wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Err(error) => {
                    if is_connectivity_related_issue(CloudStorageIssue::from(&error)) {
                        return Err(blocking_cloud_error(BlockingCloudStep::Restore, error));
                    }
                    let error = "wallet backup could not be read".to_string();
                    warn!("Failed to download wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
            }

            progress.completed += 1;

            self.send_restore_progress(restore_progress_flow(
                RestoreProgressPhase::Downloading,
                progress.completed,
                progress.total,
            ))
            .await?;
        }

        Ok(downloaded_wallets)
    }

    /// Restore via passkey-based namespace matching (fresh device path)
    ///
    /// Tries the selected passkey across all downloaded namespaces. If it
    /// doesn't match any of them, returns `PasskeyMismatch` so the caller can
    /// try local master key fallback or prompt the user to try a different
    /// passkey. Successful matches are non-empty
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
            return Err(CloudBackupError::NoBackupFound);
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
                Err(offline_error_for_step(BlockingCloudStep::Restore))
            }
            NamespaceMatchOutcome::UnsupportedVersions => Err(CloudBackupError::Internal(
                "some cloud backups use a newer format, please update the app".into(),
            )),
        }
    }
}

pub(crate) struct RestoredPasskeyMaterial {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
}

impl From<&RestorableNamespacePasskey> for RestoredPasskeyMaterial {
    fn from(passkey: &RestorableNamespacePasskey) -> Self {
        Self { credential_id: passkey.credential_id.clone(), prf_salt: passkey.prf_salt }
    }
}

impl std::fmt::Debug for RestoredPasskeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoredPasskeyMaterial")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .finish()
    }
}

pub(crate) fn save_restore_keychain_entries(
    master_key: MasterKey,
    passkey: Option<RestoredPasskeyMaterial>,
    namespace_id: String,
) -> Result<(), CloudBackupError> {
    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
    let cspp = cove_cspp::Cspp::new(keychain.clone());

    let metadata_save_result = match passkey {
        Some(passkey) => cloud_keychain
            .save_passkey_and_namespace(&passkey.credential_id, passkey.prf_salt, &namespace_id)
            .map_err(|error| ("save cspp credentials", error)),
        None => cloud_keychain
            .save_namespace_id(&namespace_id)
            .map_err(|error| ("save namespace_id", error)),
    };

    if let Err((context, error)) = metadata_save_result {
        if let Err(rollback) = cloud_keychain.clear_local_state() {
            return Err(CloudBackupError::Internal(format!(
                "{context}: {error}; rollback failed: {rollback}"
            )));
        }

        return Err(CloudBackupError::Internal(format!("{context}: {error}")));
    }

    if let Err(error) = cspp.save_master_key(&master_key) {
        if let Err(rollback) = cloud_keychain.clear_local_state() {
            return Err(CloudBackupError::Internal(format!(
                "save master key: {error}; rollback failed: {rollback}"
            )));
        }

        return Err(CloudBackupError::Internal(format!("save master key: {error}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::{test_globals, test_lock};

    #[test]
    fn restore_keychain_save_rolls_back_metadata_when_master_key_save_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.fail_save_at(4);

        let result = save_restore_keychain_entries(
            cove_cspp::master_key::MasterKey::generate(),
            Some(RestoredPasskeyMaterial { credential_id: vec![1, 2, 3], prf_salt: [4; 32] }),
            "namespace-id".into(),
        );

        assert!(result.is_err());
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_none());
    }

    #[test]
    fn restore_keychain_save_rolls_back_when_passkey_metadata_save_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[9, 8, 7], [6; 32], "old-namespace")
            .unwrap();
        globals.keychain.fail_save_at(1);

        let result = save_restore_keychain_entries(
            cove_cspp::master_key::MasterKey::generate(),
            Some(RestoredPasskeyMaterial { credential_id: vec![1, 2, 3], prf_salt: [4; 32] }),
            "namespace-id".into(),
        );

        assert!(result.is_err());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_none());
    }

    #[test]
    fn restore_keychain_save_rolls_back_when_namespace_metadata_save_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[9, 8, 7], [6; 32], "old-namespace")
            .unwrap();
        globals.keychain.fail_save_at(1);

        let result = save_restore_keychain_entries(
            cove_cspp::master_key::MasterKey::generate(),
            None,
            "namespace-id".into(),
        );

        assert!(result.is_err());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_none());
    }

    #[test]
    fn zero_total_restore_progress_maps_to_finding() {
        assert_eq!(
            restore_progress_flow(RestoreProgressPhase::Downloading, 0, 0),
            CloudBackupRestoreFlow::Finding
        );
        assert_eq!(
            restore_progress_flow(RestoreProgressPhase::Restoring, 0, 0),
            CloudBackupRestoreFlow::Finding
        );
    }
}
