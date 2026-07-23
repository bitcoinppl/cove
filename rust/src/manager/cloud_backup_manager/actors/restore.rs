use std::time::Duration;

use act_zero::{Addr, call};
use cove_cspp::master_key::MasterKey;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use futures::stream::{self, StreamExt as _};
use tokio::time::Instant;
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::ops::try_restore_from_local_master_key;
use crate::manager::cloud_backup_manager::wallets::{
    DownloadedWalletBackup, NamespaceMatch, NamespaceMatchOutcome, NamespaceMatchSnapshotOutcome,
    NamespacePasskeyMatcher, WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome,
    WalletRestoreSession,
};

use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CLOUD_BACKUP_COMPATIBILITY_MESSAGE, CLOUD_BACKUP_IO_CONCURRENCY,
    CLOUD_BACKUP_LABELS_WARNING_MESSAGE, CloudBackupError, CloudBackupRestoreFlow,
    CloudBackupRestoreOutcome, CloudBackupRestoreReport, CloudBackupStatus, CloudBackupStore,
    GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, RustCloudBackupManager, blocking_cloud_error,
    is_connectivity_related_issue, is_provider_wide_interruption, offline_error_for_step,
};

use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

use super::CloudBackupSupervisor;

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

const PASSKEY_NAMESPACE_REFRESH_OFFSETS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(7),
    Duration::from_secs(11),
    Duration::from_secs(15),
];

const PASSKEY_NAMESPACE_MATCH_GRACE_OFFSETS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(7),
    Duration::from_secs(10),
];

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

fn merge_namespace_matches(accumulated: &mut Vec<NamespaceMatch>, discovered: Vec<NamespaceMatch>) {
    for namespace_match in discovered {
        if let Some(existing) = accumulated
            .iter_mut()
            .find(|existing| existing.namespace_id == namespace_match.namespace_id)
        {
            *existing = namespace_match;
        } else {
            accumulated.push(namespace_match);
        }
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

    pub(crate) async fn clear_enable_progress(&self) -> Result<(), CloudBackupError> {
        call!(self.supervisor.clear_restore_enable_progress(self.operation_claim))
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
        self.clear_enable_progress().await?;
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
            Err(error @ (CloudBackupError::PasskeyMismatch | CloudBackupError::NoBackupFound)) => {
                info!(
                    "Restore: passkey matching found no restore, trying local master key fallback"
                );
                let (master_key, namespace_id) = try_restore_from_local_master_key(&cloud, &cspp)
                    .await
                    .map_err(|error| blocking_cloud_error(BlockingCloudStep::Restore, error))?
                    .ok_or(error)?;
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
            .map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;

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
        let mut first_duplicate_namespace_index = None;
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
                        report
                            .labels_failed_errors
                            .push(CLOUD_BACKUP_LABELS_WARNING_MESSAGE.into());
                    }
                }
                Ok(WalletRestoreOutcome::SkippedDuplicate) => {
                    first_duplicate_namespace_index.get_or_insert(*namespace_index);
                    skipped_duplicate_count += 1;
                }
                Err(CloudBackupError::Cancelled) => return Err(CloudBackupError::Cancelled),
                Err(error) => {
                    warn!("Failed to restore wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error.reader_message());
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

        let restored_namespace = first_success_namespace_index
            .and_then(|index| namespace_wallets.get(index))
            .map(|(namespace, _)| namespace);
        let duplicate_namespace = first_duplicate_namespace_index
            .and_then(|index| namespace_wallets.get(index))
            .map(|(namespace, _)| namespace);

        let restored_status = match restored_namespace {
            Some(active) => {
                self.activate_restored_namespace(manager, active).await?;

                CloudBackupStatus::Enabled
            }

            None if skipped_duplicate_count > 0 => {
                let state = RustCloudBackupManager::load_persisted_state();
                if matches!(state, PersistedCloudBackupState::Disabled)
                    && let Some(active) = duplicate_namespace
                {
                    self.activate_restored_namespace(manager, active).await?;

                    CloudBackupStatus::Enabled
                } else {
                    RustCloudBackupManager::runtime_status_for(&state)
                }
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
        active: &RestorableNamespace,
    ) -> Result<(), CloudBackupError> {
        let master_key = MasterKey::from_bytes(*active.master_key.as_bytes());
        let passkey = active.passkey.as_ref().map(RestoredPasskeyMaterial::from);
        let wallets = CloudBackupStore::global().all_wallets()?;
        let wallet_count = wallets.len() as u32;

        self.save_keychain_state(master_key, passkey, active.namespace_id.clone()).await?;

        let enabled_state = PersistedCloudBackupState::configured_after_restore(
            crate::manager::cloud_backup_manager::current_timestamp(),
            wallet_count,
        );
        self.persist_cloud_backup_state(
            enabled_state,
            "persist restored cloud backup state".into(),
        )
        .await?;

        manager.mark_wallet_blobs_dirty_for_background_upload(
            wallets.into_iter().map(|wallet| wallet.id),
        )?;

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
                    let error = CloudBackupError::NoBackupFound.reader_message();
                    warn!("Failed to download wallet backup: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!("Failed to download wallet backup: unsupported version {version}");
                    let error = CLOUD_BACKUP_COMPATIBILITY_MESSAGE.to_string();
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Err(error) => {
                    if is_provider_wide_interruption(&error) {
                        return Err(blocking_cloud_error(BlockingCloudStep::Restore, error));
                    }
                    let error = GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.to_string();
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
        let matcher = NamespacePasskeyMatcher::new(cloud, passkey);
        let mut session = matcher.start_session();
        let started_at = Instant::now();
        let mut first_match_at = None;
        let mut no_match_refresh_index = 0;
        let mut grace_refresh_index = 0;
        let mut refresh_index = 0;
        let mut accumulated_matches = Vec::new();

        loop {
            self.ensure_current().await?;

            let mut namespaces = match cloud.list_namespaces().await {
                Ok(namespaces) => namespaces,
                Err(error) if is_connectivity_related_issue(&error) => {
                    warn!(
                        "Restore: cloud namespace refresh failed refresh_index={refresh_index}: {error}"
                    );
                    session.note_namespace_discovery_failure();
                    Vec::new()
                }
                Err(error) => {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::Restore,
                        CloudBackupError::cloud_storage_context(
                            "list cloud backup namespaces",
                            error,
                        ),
                    ));
                }
            };
            namespaces.sort();
            namespaces.dedup();

            info!(
                "Restore: passkey candidate refresh refresh_index={refresh_index} namespace_count={}",
                namespaces.len()
            );
            match session.match_snapshot(&namespaces).await? {
                NamespaceMatchSnapshotOutcome::Matched(matches) => {
                    info!("Restore: matched {} namespace(s)", matches.len());
                    merge_namespace_matches(&mut accumulated_matches, matches);
                    first_match_at.get_or_insert_with(Instant::now);
                }
                NamespaceMatchSnapshotOutcome::UserDeclined => {
                    if accumulated_matches.is_empty() {
                        return Err(CloudBackupError::PasskeyDiscoveryCancelled);
                    }

                    return Ok(accumulated_matches);
                }
                NamespaceMatchSnapshotOutcome::Continue => {}
            }

            let next_refresh_at = if let Some(first_match_at) = first_match_at {
                let Some(refresh_offset) =
                    PASSKEY_NAMESPACE_MATCH_GRACE_OFFSETS.get(grace_refresh_index)
                else {
                    return Ok(accumulated_matches);
                };
                grace_refresh_index += 1;

                // keep scanning through the anchored 10-second grace window so another matching namespace can appear
                // then restore without waiting longer
                first_match_at + *refresh_offset
            } else {
                let Some(refresh_offset) =
                    PASSKEY_NAMESPACE_REFRESH_OFFSETS.get(no_match_refresh_index)
                else {
                    break;
                };
                no_match_refresh_index += 1;

                started_at + *refresh_offset
            };

            tokio::time::sleep_until(next_refresh_at).await;
            self.ensure_current().await?;
            refresh_index += 1;
        }

        let saw_supported_candidate = session.saw_supported_candidate();
        let match_outcome = session.finish();

        match match_outcome {
            NamespaceMatchOutcome::Matched(matches) => Ok(matches),
            NamespaceMatchOutcome::UserDeclined => Err(CloudBackupError::PasskeyDiscoveryCancelled),
            NamespaceMatchOutcome::NoMatch if saw_supported_candidate => {
                Err(CloudBackupError::PasskeyMismatch)
            }
            NamespaceMatchOutcome::NoMatch => Err(CloudBackupError::NoBackupFound),
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
            return Err(CloudBackupError::Internal(
                format!("{context}: {error}; rollback failed: {rollback}").into(),
            ));
        }

        return Err(CloudBackupError::Internal(format!("{context}: {error}").into()));
    }

    if let Err(error) = cspp.save_master_key(&master_key) {
        if let Err(rollback) = cloud_keychain.clear_local_state() {
            return Err(CloudBackupError::Internal(
                format!("save master key: {error}; rollback failed: {rollback}").into(),
            ));
        }

        return Err(CloudBackupError::Internal(format!("save master key: {error}").into()));
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

    #[test]
    fn namespace_match_merge_preserves_first_seen_order_and_replaces_revisions_in_place() {
        let namespace_match = |namespace_id: &str, prf_salt: [u8; 32]| NamespaceMatch {
            namespace_id: namespace_id.into(),
            master_key: MasterKey::generate(),
            prf_salt,
            credential_id: vec![1, 2, 3],
        };
        let mut accumulated =
            vec![namespace_match("first", [1; 32]), namespace_match("second", [2; 32])];

        merge_namespace_matches(
            &mut accumulated,
            vec![namespace_match("second", [9; 32]), namespace_match("third", [3; 32])],
        );

        assert_eq!(
            accumulated
                .iter()
                .map(|namespace_match| namespace_match.namespace_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
        assert_eq!(accumulated[1].prf_salt, [9; 32]);
    }
}
