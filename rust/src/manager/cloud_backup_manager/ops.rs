use std::collections::HashSet;

use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use futures::stream::{self, StreamExt as _};
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::cloud_inventory::CloudWalletInventory;
use super::wallets::{
    DownloadedWalletBackup, NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher,
    PasskeyMaterialAcquirer, UnpersistedPrfKey, WalletBackupLookup, WalletBackupReader,
    WalletRestoreSession,
};

#[cfg(test)]
use super::PendingUploadVerificationState;
use super::workers::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob,
    RestoredPasskeyMaterial,
};
use super::{
    BlockingCloudStep, CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupKeychain,
    CloudBackupPasskeyChoiceFlow, CloudBackupRestoreProgress, CloudBackupRestoreReport,
    CloudBackupRestoreStage, CloudBackupStatus, CloudBackupStore, CloudBackupWalletItem,
    CloudBackupWalletStatus, DeepVerificationReport, PendingEnableSession,
    PendingVerificationCompletion, PendingVerificationUpload, RestoreOperation,
    RustCloudBackupManager, VerificationState, blocking_cloud_error,
    current_namespace_wallet_record_ids, is_connectivity_related_issue,
};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};

const CLOUD_ONLY_FETCH_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before wallets not on this device can be loaded";
const CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before this wallet can be restored";
const RECREATE_MANIFEST_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before the backup index can be recreated";
const UNSUPPORTED_CLOUD_ONLY_WALLET_NAME: &str = "Unsupported wallet backup";
enum FinalizeUploadStateMode {
    PreserveVerification,
    ResetVerification,
}

enum EnablePasskeyAcquisition {
    Ready(UnpersistedPrfKey),
    Cancelled,
}

struct RestorableNamespace {
    namespace_id: String,
    master_key: cove_cspp::master_key::MasterKey,
    passkey: Option<RestorableNamespacePasskey>,
}

struct MergeNamespace {
    matched: NamespaceMatch,
    wallet_record_ids: Vec<String>,
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

struct MergedNamespaceWallets {
    source: CleanupSourceNamespace,
    restored_wallets: Vec<crate::wallet::metadata::WalletMetadata>,
}

impl RustCloudBackupManager {
    async fn lookup_wallet_backup(
        reader: WalletBackupReader,
        record_id: String,
    ) -> (String, Result<WalletBackupLookup<DownloadedWalletBackup>, CloudBackupError>) {
        let lookup = reader.lookup(&record_id).await;
        (record_id, lookup)
    }

    fn clear_enable_progress(&self, status: CloudBackupStatus) {
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_status(status);
    }

    async fn keep_awaiting_force_new_confirmation(&self) -> bool {
        if !self.has_awaiting_force_new_pending_enable_session().await {
            return false;
        }

        self.set_existing_backup_found_prompt();
        self.clear_enable_progress(CloudBackupStatus::Disabled);
        true
    }

    fn rollback_new_local_master_key(
        &self,
        cspp: &cove_cspp::Cspp<Keychain>,
        had_local_master_key: bool,
        context: &str,
    ) {
        if had_local_master_key {
            return;
        }

        warn!("{context}: deleting new local master key");
        cspp.delete_master_key();
    }

    async fn acquire_enable_passkey<F, Fut>(
        &self,
        cspp: &cove_cspp::Cspp<Keychain>,
        had_local_master_key: bool,
        cancelled_context: &str,
        failed_context: &str,
        acquire: F,
    ) -> Result<EnablePasskeyAcquisition, CloudBackupError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<UnpersistedPrfKey, CloudBackupError>>,
    {
        match acquire().await {
            Ok(passkey) => Ok(EnablePasskeyAcquisition::Ready(passkey)),
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.rollback_new_local_master_key(cspp, had_local_master_key, cancelled_context);
                Ok(EnablePasskeyAcquisition::Cancelled)
            }
            Err(error) => {
                self.rollback_new_local_master_key(cspp, had_local_master_key, failed_context);
                Err(error)
            }
        }
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

    async fn finalize_uploaded_wallets(
        &self,
        cloud: &CloudStorageClient,
        namespace_id: &str,
        uploaded_wallets: Vec<super::wallets::PreparedWalletBackup>,
        state_mode: FinalizeUploadStateMode,
    ) -> Result<(), CloudBackupError> {
        let db = Database::global();
        let wallet_count = cloud
            .list_wallet_backups(namespace_id.to_owned())
            .await
            .map(|ids| ids.len() as u32)
            .unwrap_or(uploaded_wallets.len() as u32);
        match state_mode {
            FinalizeUploadStateMode::PreserveVerification => {
                CloudBackupStore::new(&db).persist_enabled(wallet_count)?;
            }
            FinalizeUploadStateMode::ResetVerification => {
                CloudBackupStore::new(&db).persist_enabled_reset_verification(wallet_count)?;
            }
        }

        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        for wallet in uploaded_wallets {
            self.mark_blob_uploaded_pending_confirmation(
                namespace_id,
                Some(wallet.metadata.id),
                wallet.record_id,
                wallet.revision_hash,
                uploaded_at,
            )?;
        }

        Ok(())
    }

    fn pending_verification_uploads(
        uploaded_wallets: &[super::wallets::PreparedWalletBackup],
    ) -> Vec<PendingVerificationUpload> {
        uploaded_wallets
            .iter()
            .map(|wallet| {
                PendingVerificationUpload::new(
                    wallet.record_id.clone(),
                    wallet.revision_hash.clone(),
                )
            })
            .collect()
    }

    async fn seed_post_enable_verification_from_fresh_passkey_material(
        &self,
        encrypted_master: &cove_cspp::backup_data::EncryptedMasterKeyBackup,
        master_key: &cove_cspp::master_key::MasterKey,
        passkey: &UnpersistedPrfKey,
        namespace_id: &str,
        pending_uploads: Vec<PendingVerificationUpload>,
    ) -> Result<(), CloudBackupError> {
        let decrypted_master =
            master_key_crypto::decrypt_master_key(encrypted_master, &passkey.prf_key)
                .map_err_str(CloudBackupError::Crypto)?;
        if decrypted_master.as_bytes() != master_key.as_bytes() {
            return Err(CloudBackupError::Crypto(
                "fresh passkey material decrypted the wrong master key".into(),
            ));
        }

        self.record_runtime_passkey_authorization(
            namespace_id.to_owned(),
            passkey.credential_id.clone(),
            passkey.prf_salt,
        )
        .await?;

        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        let mut pending_uploads = pending_uploads;
        pending_uploads.insert(0, PendingVerificationUpload::master_key_wrapper());

        self.replace_pending_verification_completion(PendingVerificationCompletion::new(
            report,
            namespace_id.to_owned(),
            pending_uploads,
        ));
        self.set_verification(VerificationState::Idle);

        Ok(())
    }

    pub(crate) async fn do_sync_unsynced_wallets(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Sync)?;
        let namespace = self.current_namespace_id()?;
        info!("Sync: listing cloud wallet backups for namespace {namespace}");
        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids = cloud.list_wallet_backups(namespace).await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::Sync,
                CloudBackupError::cloud_storage_context("list wallet backups", error),
            )
        })?;
        let remote_wallet_truth =
            self.load_remote_wallet_truth(&wallet_record_ids, cloud.clone()).await?;
        let inventory =
            CloudWalletInventory::load_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
                .await?;

        info!("Sync: found {} wallet(s) in cloud", inventory.cloud_wallet_count());
        let unsynced = inventory.upload_candidate_wallets();

        if unsynced.is_empty() {
            info!("Sync: all wallets already synced");
            return Ok(());
        }

        info!("Sync: {} wallet(s) need backup", unsynced.len());
        self.do_backup_wallets(&unsynced)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Sync, error))
    }

    pub(crate) async fn do_fetch_cloud_only_wallets(
        &self,
    ) -> Result<Vec<CloudBackupWalletItem>, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::FetchCloudOnly)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let wallet_record_ids =
            cloud.list_wallet_backups(namespace.clone()).await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::FetchCloudOnly,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                )
            })?;

        let db = Database::global();
        let local_record_ids: std::collections::HashSet<_> = CloudBackupStore::new(&db)
            .all_wallets()?
            .iter()
            .map(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()))
            .collect();

        let orphan_ids: Vec<_> = wallet_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .cloned()
            .collect();

        if orphan_ids.is_empty() {
            return Ok(Vec::new());
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                CLOUD_ONLY_FETCH_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error))?;

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut items = Vec::new();
        let mut lookups = stream::iter(
            orphan_ids
                .into_iter()
                .map(|record_id| Self::lookup_wallet_backup(reader.clone(), record_id)),
        )
        .buffered(CLOUD_BACKUP_IO_CONCURRENCY);

        while let Some((record_id, lookup)) = lookups.next().await {
            let wallet = match lookup {
                Ok(WalletBackupLookup::Found(wallet)) => wallet,
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    warn!(
                        "Cloud-only wallet {record_id} uses unsupported wallet backup version {version}"
                    );
                    items.push(CloudBackupWalletItem {
                        name: UNSUPPORTED_CLOUD_ONLY_WALLET_NAME.into(),
                        network: None,
                        wallet_mode: None,
                        wallet_type: None,
                        fingerprint: None,
                        label_count: None,
                        backup_updated_at: None,
                        sync_status: CloudBackupWalletStatus::UnsupportedVersion,
                        record_id: record_id.clone(),
                    });
                    continue;
                }
                Ok(WalletBackupLookup::NotFound) => {
                    warn!("Failed to load cloud-only wallet {record_id}: not found");
                    continue;
                }
                Err(error) => {
                    if is_connectivity_related_issue(error.cloud_storage_issue()) {
                        return Err(blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error));
                    }
                    warn!("Failed to load cloud-only wallet {record_id}: {error}");
                    continue;
                }
            };
            let metadata = wallet.metadata;

            items.push(CloudBackupWalletItem {
                name: metadata.name,
                network: Some(metadata.network),
                wallet_mode: Some(metadata.wallet_mode),
                wallet_type: Some(metadata.wallet_type),
                fingerprint: metadata
                    .master_fingerprint
                    .as_ref()
                    .map(|fingerprint| fingerprint.as_ref().as_uppercase()),
                label_count: Some(wallet.entry.labels_count),
                backup_updated_at: Some(wallet.entry.updated_at),
                sync_status: CloudBackupWalletStatus::DeletedFromDevice,
                record_id: record_id.clone(),
            });
        }

        Ok(items)
    }

    pub(crate) async fn do_restore_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<super::wallets::WalletRestoreOutcome, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RestoreCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud(
                &namespace,
                CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;
        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );

        let db = Database::global();
        let existing_fingerprints: Vec<_> = CloudBackupStore::new(&db)
            .all_wallets()?
            .iter()
            .filter_map(|wallet| {
                wallet
                    .master_fingerprint
                    .as_ref()
                    .map(|fp| (**fp, wallet.network, wallet.wallet_mode))
            })
            .collect();
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);

        let outcome = restore_session
            .restore_record(&reader, record_id)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;
        info!("Restored cloud wallet {record_id}");
        Ok(outcome)
    }

    pub(crate) async fn do_delete_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();

        cloud.delete_wallet_backup(namespace.clone(), record_id.to_string()).await.map_err(
            |error| {
                blocking_cloud_error(
                    BlockingCloudStep::DeleteCloudWallet,
                    CloudBackupError::cloud_storage_context("delete wallet backup", error),
                )
            },
        )?;
        self.remove_blob_sync_states(std::iter::once(record_id.to_string()))
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error))?;

        let wallet_record_ids = cloud.list_wallet_backups(namespace).await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::DeleteCloudWallet,
                CloudBackupError::cloud_storage_context("list wallet backups", error),
            )
        })?;
        let wallet_count = wallet_record_ids.len() as u32;
        let db = Database::global();
        if let Ok(mut current) = db.cloud_backup_state.get() {
            current.wallet_count = Some(wallet_count);
            let _ = self.persist_cloud_backup_state(
                &current,
                "persist cloud backup state after deleting cloud wallet",
            );
        }

        info!("Deleted cloud wallet {record_id}");
        Ok(())
    }

    pub(crate) async fn do_recover_other_backups(
        &self,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RecoverOtherBackups)?;
        let current_namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let passkey = PasskeyAccess::global();
        let namespaces = self
            .other_backup_namespaces(
                &cloud,
                &current_namespace,
                BlockingCloudStep::RecoverOtherBackups,
            )
            .await?;
        if namespaces.is_empty() {
            return Err(CloudBackupError::Internal("no other cloud backups found".into()));
        }

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey);
        let matches = match matcher.match_namespaces(&namespaces).await? {
            NamespaceMatchOutcome::Matched(matches) => matches,
            NamespaceMatchOutcome::UserDeclined => {
                return Err(CloudBackupError::PasskeyDiscoveryCancelled);
            }
            NamespaceMatchOutcome::NoMatch => return Err(CloudBackupError::PasskeyMismatch),
            NamespaceMatchOutcome::Inconclusive => {
                return Err(self.offline_error_for_step(BlockingCloudStep::RecoverOtherBackups));
            }
            NamespaceMatchOutcome::UnsupportedVersions => {
                return Err(CloudBackupError::Internal(
                    "some cloud backups use a newer format, please update the app".into(),
                ));
            }
        };

        self.restore_wallets_from_namespaces(&cloud, matches).await
    }

    pub(crate) async fn do_delete_other_backups(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteOtherBackups)?;
        let current_namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let namespaces = self
            .other_backup_namespaces(
                &cloud,
                &current_namespace,
                BlockingCloudStep::DeleteOtherBackups,
            )
            .await?;

        for namespace in namespaces {
            cloud.delete_namespace(namespace.clone()).await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::DeleteOtherBackups,
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error),
                )
            })?;
            info!("Deleted other cloud backup namespace {namespace}");
        }

        Ok(())
    }

    /// Re-upload all local wallets to cloud
    ///
    /// Reuses the master key from keychain (no passkey interaction needed)
    pub(crate) async fn do_reupload_all_wallets(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RecreateManifest)?;
        info!("Re-uploading all wallets to cloud");

        let namespace = self.current_namespace_id()?;
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                RECREATE_MANIFEST_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|error| blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let cloud = CloudStorage::global_explicit_client();
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&cloud, &namespace, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;

        self.finalize_uploaded_wallets(
            &cloud,
            &namespace,
            uploaded_wallets,
            FinalizeUploadStateMode::PreserveVerification,
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn do_enable_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(Keychain::global(), master_key, passkey)
                .await;
        }

        if self.keep_awaiting_force_new_confirmation().await {
            return Ok(());
        }

        let passkey = PasskeyAccess::global();
        if !passkey.is_prf_supported() {
            return Err(CloudBackupError::NotSupported(
                "PRF extension not supported on this device".into(),
            ));
        }

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global_explicit_client();

        let has_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();

        if has_local_master_key {
            return self.do_enable_cloud_backup_create_new().await;
        }

        // no local master key — check iCloud for existing namespaces to recover
        let mut namespaces = cloud
            .list_namespaces()
            .await
            .map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::Enable,
                    CloudBackupError::cloud_storage_context(
                        "could not check for existing cloud backups, please try again when cloud storage is available",
                        error,
                    ),
                )
            })?;
        namespaces.sort();

        if namespaces.is_empty() {
            return self.do_enable_cloud_backup_create_new().await;
        }

        info!("Enable: found {} existing namespace(s), attempting recovery", namespaces.len());

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey);
        let match_outcome = matcher.match_namespaces(&namespaces).await?;
        match match_outcome {
            NamespaceMatchOutcome::Matched(matches) => {
                if matches.is_empty() {
                    self.set_existing_backup_found_prompt();
                    self.clear_enable_progress(CloudBackupStatus::Disabled);
                    return Ok(());
                }

                self.complete_recovery(&cloud_keychain, &cloud, &cspp, matches).await
            }

            NamespaceMatchOutcome::UserDeclined => {
                info!("Enable: user cancelled passkey picker during namespace matching");
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceFlow::Enable);
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                Ok(())
            }

            NamespaceMatchOutcome::NoMatch => {
                info!("Enable: passkey didn't match existing backups, asking user to confirm");
                self.set_existing_backup_found_prompt();
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                Ok(())
            }

            NamespaceMatchOutcome::Inconclusive => Err(self.offline_error_for_step(
                BlockingCloudStep::Enable,
            )),

            NamespaceMatchOutcome::UnsupportedVersions => Err(CloudBackupError::Internal(
                "some cloud backups use a newer format, please update the app to access all backups"
                    .into(),
            )),
        }
    }

    /// Complete recovery from matched cloud namespaces
    async fn complete_recovery(
        &self,
        cloud_keychain: &CloudBackupKeychain,
        cloud: &CloudStorageClient,
        cspp: &cove_cspp::Cspp<Keychain>,
        matches: Vec<super::wallets::NamespaceMatch>,
    ) -> Result<(), CloudBackupError> {
        let merge_namespaces = self.load_enable_merge_namespaces(cloud, matches).await?;
        let Some(active_index) = active_merge_namespace_index(&merge_namespaces) else {
            return Err(CloudBackupError::Internal(
                "no matching cloud backup namespaces found".into(),
            ));
        };
        let active_namespace_id = merge_namespaces[active_index].matched.namespace_id.clone();
        let active_master_key = cove_cspp::master_key::MasterKey::from_bytes(
            *merge_namespaces[active_index].matched.master_key.as_bytes(),
        );
        let active_critical_key = active_master_key.critical_data_key();

        info!(
            "Enable: merging {} recovered namespace(s) into active namespace {}",
            merge_namespaces.len(),
            active_namespace_id
        );

        cspp.save_master_key(&active_master_key)
            .map_err_prefix("save recovered master key", CloudBackupError::Internal)?;

        let result = async {
            let merged_wallets =
                self.restore_enable_merge_wallets(cloud, &merge_namespaces).await?;

            for metadata in merged_wallets.iter().flat_map(|merged| &merged.restored_wallets) {
                info!("Enable: recovered wallet {} from matched namespace", metadata.name);
            }

            let critical_key = Zeroizing::new(active_critical_key);
            let uploaded_wallets = CloudBackupStore::global()
                .upload_all_wallets(cloud, &active_namespace_id, &critical_key)
                .await
                .map_err(|error| blocking_cloud_error(BlockingCloudStep::Enable, error))?;

            let active_match = &merge_namespaces[active_index].matched;
            cloud_keychain
                .save_passkey_and_namespace(
                    &active_match.credential_id,
                    active_match.prf_salt,
                    &active_namespace_id,
                )
                .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

            self.record_runtime_passkey_authorization(
                active_namespace_id.clone(),
                active_match.credential_id.clone(),
                active_match.prf_salt,
            )
            .await?;

            self.finalize_uploaded_wallets(
                cloud,
                &active_namespace_id,
                uploaded_wallets,
                FinalizeUploadStateMode::ResetVerification,
            )
            .await?;

            let cleanup_sources = merged_wallets
                .into_iter()
                .filter(|merged| merged.source.namespace_id != active_namespace_id)
                .map(|merged| merged.source)
                .collect::<Vec<_>>();
            self.enqueue_cleanup(CloudBackupCleanupJob {
                cloud: cloud.clone(),
                active_namespace_id: active_namespace_id.clone(),
                active_critical_key,
                sources: cleanup_sources,
            });

            Ok(())
        }
        .await;

        if let Err(error) = result {
            warn!("Enable: rolling back recovered local master key after recovery failure");
            cspp.delete_master_key();
            return Err(error);
        }

        self.clear_pending_enable_session();
        self.clear_enable_progress(CloudBackupStatus::Enabled);
        info!("Cloud backup enabled (recovered existing namespace)");
        Ok(())
    }

    async fn load_enable_merge_namespaces(
        &self,
        cloud: &CloudStorageClient,
        matches: Vec<NamespaceMatch>,
    ) -> Result<Vec<MergeNamespace>, CloudBackupError> {
        let mut merge_namespaces = Vec::with_capacity(matches.len());

        for matched in matches {
            let wallet_record_ids =
                cloud.list_wallet_backups(matched.namespace_id.clone()).await.map_err(|error| {
                    blocking_cloud_error(
                        BlockingCloudStep::Enable,
                        CloudBackupError::cloud_storage_context("list wallet backups", error),
                    )
                })?;

            merge_namespaces.push(MergeNamespace { matched, wallet_record_ids });
        }

        Ok(merge_namespaces)
    }

    async fn restore_enable_merge_wallets(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[MergeNamespace],
    ) -> Result<Vec<MergedNamespaceWallets>, CloudBackupError> {
        let existing_fingerprints = crate::backup::import::collect_existing_fingerprints()
            .map_err_prefix("collect fingerprints", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);
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
                match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: Some(wallet.entry.content_revision_hash.clone()),
                        });

                        match restore_session.restore_downloaded(&wallet) {
                            Ok(_) => restored_wallets.push(wallet.metadata),
                            Err(error) => {
                                if is_connectivity_related_issue(error.cloud_storage_issue()) {
                                    return Err(blocking_cloud_error(
                                        BlockingCloudStep::Enable,
                                        error,
                                    ));
                                }
                                warn!(
                                    "Enable: failed to restore wallet {}/{} during namespace merge: {error}",
                                    namespace.matched.namespace_id, record_id
                                );
                            }
                        }
                    }
                    Ok(WalletBackupLookup::NotFound) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!(
                            "Enable: matched namespace {}/{} listed a missing wallet backup",
                            namespace.matched.namespace_id, record_id
                        );
                    }
                    Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!(
                            "Enable: matched namespace {}/{} uses unsupported wallet backup version {version}",
                            namespace.matched.namespace_id, record_id
                        );
                    }
                    Err(error) => {
                        if is_connectivity_related_issue(error.cloud_storage_issue()) {
                            return Err(blocking_cloud_error(BlockingCloudStep::Enable, error));
                        }
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!(
                            "Enable: failed to inspect wallet {}/{} during namespace merge: {error}",
                            namespace.matched.namespace_id, record_id
                        );
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

    /// Create a new cloud backup from scratch — no recovery attempt
    ///
    /// Called directly when `do_enable_cloud_backup` determines no recovery is needed,
    /// or via `do_enable_cloud_backup_force_new` when the user confirms creating a
    /// new backup after being warned about existing ones
    pub(crate) async fn do_enable_cloud_backup_create_new(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(Keychain::global(), master_key, passkey)
                .await;
        }
        if self.keep_awaiting_force_new_confirmation().await {
            return Ok(());
        }

        let passkey_access = PasskeyAccess::global();
        let keychain = Keychain::global();

        info!("Enable: getting master key");
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let had_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();
        let master_key = cspp
            .get_or_create_master_key()
            .map_err_prefix("master key", CloudBackupError::Internal)?;

        let namespace_id = master_key.namespace_id();
        info!("Enable: namespace_id={namespace_id}, getting passkey");
        let passkey = match self
            .acquire_enable_passkey(
                &cspp,
                had_local_master_key,
                "Enable cancelled before passkey setup finished",
                "Enable failed before passkey setup finished",
                || {
                    let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
                    async move { acquirer.discover_or_create_for_enable().await }
                },
            )
            .await?
        {
            EnablePasskeyAcquisition::Ready(passkey) => passkey,
            EnablePasskeyAcquisition::Cancelled => {
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceFlow::Enable);
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                return Ok(());
            }
        };

        info!("Enable: passkey created, uploading backup");
        self.enable_cloud_backup_with_passkey_material(
            keychain,
            Zeroizing::new(master_key),
            Zeroizing::new(passkey),
        )
        .await
    }

    pub(crate) async fn do_enable_cloud_backup_force_new(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let keychain = Keychain::global();

        if let Some(pending) = self.take_pending_enable_session().await {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: committing pending create-first cloud backup");
            return self
                .enable_cloud_backup_with_passkey_material(keychain, master_key, passkey)
                .await;
        }

        self.do_enable_cloud_backup_create_new().await
    }

    /// Same as `do_enable_cloud_backup_create_new` but skips passkey discovery,
    /// going straight to passkey registration
    pub(crate) async fn do_enable_cloud_backup_no_discovery(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable (no discovery): retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(Keychain::global(), master_key, passkey)
                .await;
        }
        if self.keep_awaiting_force_new_confirmation().await {
            return Ok(());
        }

        let passkey_access = PasskeyAccess::global();
        let keychain = Keychain::global();
        let cloud = CloudStorage::global_explicit_client();

        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let had_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();
        let existing_namespaces = if had_local_master_key {
            Vec::new()
        } else {
            cloud.list_namespaces().await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::Enable,
                    CloudBackupError::cloud_storage_context(
                        "could not check for existing cloud backups, please try again when cloud storage is available",
                        error,
                    ),
                )
            })?
        };

        info!("Enable (no discovery): getting master key");
        let master_key = cspp
            .get_or_create_master_key()
            .map_err_prefix("master key", CloudBackupError::Internal)?;

        let namespace_id = master_key.namespace_id();
        info!("Enable (no discovery): namespace_id={namespace_id}, creating passkey");
        let passkey = match self
            .acquire_enable_passkey(
                &cspp,
                had_local_master_key,
                "Enable (no discovery) cancelled before passkey setup finished",
                "Enable (no discovery) failed before passkey setup finished",
                || {
                    let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
                    async move { acquirer.create_for_enable().await }
                },
            )
            .await?
        {
            EnablePasskeyAcquisition::Ready(passkey) => passkey,
            EnablePasskeyAcquisition::Cancelled => {
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceFlow::Enable);
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                return Ok(());
            }
        };

        if !had_local_master_key && !existing_namespaces.is_empty() {
            info!(
                "Enable (no discovery): created passkey with {} existing namespace(s), waiting for confirmation",
                existing_namespaces.len()
            );
            self.replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                master_key, passkey,
            ))
            .await;
            self.set_existing_backup_found_prompt();
            self.clear_enable_progress(CloudBackupStatus::Disabled);
            return Ok(());
        }

        info!("Enable (no discovery): passkey created, uploading backup");
        self.enable_cloud_backup_with_passkey_material(
            keychain,
            Zeroizing::new(master_key),
            Zeroizing::new(passkey),
        )
        .await
    }

    pub(super) async fn do_restore_from_cloud_backup(
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

        // download and restore wallets
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
        let state = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Enabled,
            last_sync: Some(now),
            wallet_count: Some(wallet_count),
            last_verified_at: None,
            last_verification_requested_at: None,
            last_verification_dismissed_at: None,
            pending_verification_completion: None,
        };
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
                    if is_connectivity_related_issue(error.cloud_storage_issue()) {
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

    async fn enable_cloud_backup_with_passkey_material(
        &self,
        keychain: &Keychain,
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<UnpersistedPrfKey>,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let namespace_id = master_key.namespace_id();
        let cloud = CloudStorage::global_explicit_client();
        self.replace_pending_enable_session(PendingEnableSession::retry_upload(
            cove_cspp::master_key::MasterKey::from_bytes(*master_key.as_bytes()),
            passkey.copy_for_retry(),
        ))
        .await;

        let encrypted_master = master_key_crypto::encrypt_master_key_with_provider_hint(
            &master_key,
            &passkey.prf_key,
            &passkey.prf_salt,
            passkey.provider_hint.clone(),
        )
        .map_err_str(CloudBackupError::Crypto)?;
        let master_json =
            serde_json::to_vec(&encrypted_master).map_err_str(CloudBackupError::Internal)?;

        info!("Enable: uploading master key");
        cloud.upload_master_key_backup(namespace_id.clone(), master_json).await.map_err(
            |error| {
                blocking_cloud_error(
                    BlockingCloudStep::Enable,
                    CloudBackupError::cloud_storage_context("upload master key backup", error),
                )
            },
        )?;

        info!("Enable: uploading wallets");
        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&cloud, &namespace_id, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Enable, error))?;
        let pending_uploads = Self::pending_verification_uploads(&uploaded_wallets);

        info!("Enable: persisting cloud backup state");
        CloudBackupKeychain::new(keychain.clone())
            .save_passkey_and_namespace(&passkey.credential_id, passkey.prf_salt, &namespace_id)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        self.mark_blob_uploaded_pending_confirmation(
            &namespace_id,
            None,
            super::cspp_master_key_record_id(),
            "master-key-wrapper".into(),
            uploaded_at,
        )?;

        self.finalize_uploaded_wallets(
            &cloud,
            &namespace_id,
            uploaded_wallets,
            FinalizeUploadStateMode::ResetVerification,
        )
        .await?;

        self.seed_post_enable_verification_from_fresh_passkey_material(
            &encrypted_master,
            &master_key,
            &passkey,
            &namespace_id,
            pending_uploads,
        )
        .await?;
        self.clear_pending_enable_session();
        self.clear_enable_progress(CloudBackupStatus::Enabled);
        self.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        Ok(())
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

    async fn restore_wallets_from_namespaces(
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
                                if is_connectivity_related_issue(error.cloud_storage_issue()) {
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
                        if is_connectivity_related_issue(error.cloud_storage_issue()) {
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

async fn try_restore_from_local_master_key<S>(
    cloud: &CloudStorageClient,
    cspp: &cove_cspp::Cspp<S>,
) -> Result<Option<(cove_cspp::master_key::MasterKey, String)>, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    let Some(master_key) = cspp
        .load_master_key_from_store()
        .map_err_prefix("loading master key from store", CloudBackupError::Internal)?
    else {
        return Ok(None);
    };
    let namespace_id = master_key.namespace_id();

    let has_wallets =
        cloud.list_wallet_backups(namespace_id.clone()).await.map(|ids| !ids.is_empty()).map_err(
            |error| CloudBackupError::cloud_storage_context("list wallet backups", error),
        )?;

    if has_wallets {
        info!("Restore: found local master key with wallets, namespace_id={namespace_id}");
        Ok(Some((master_key, namespace_id)))
    } else {
        info!(
            "Restore: local master key found but no wallets in cloud, falling through to passkey matching"
        );
        Ok(None)
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

#[cfg(test)]
async fn restore_from_local_master_key_fallback<S>(
    cloud: &CloudStorageClient,
    store: &S,
    cspp: &cove_cspp::Cspp<S>,
) -> Result<(cove_cspp::master_key::MasterKey, String), CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    let (master_key, namespace_id) = try_restore_from_local_master_key(cloud, cspp)
        .await?
        .ok_or(CloudBackupError::PasskeyMismatch)?;
    store
        .save(super::keychain::CSPP_NAMESPACE_ID_KEY.into(), namespace_id.to_owned())
        .map_err_prefix("save namespace_id", CloudBackupError::Internal)?;
    Ok((master_key, namespace_id))
}

pub(super) async fn load_master_key_for_cloud_action<S, F, Fut>(
    cspp: &cove_cspp::Cspp<S>,
    recover_missing: F,
) -> Result<cove_cspp::master_key::MasterKey, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<cove_cspp::master_key::MasterKey, CloudBackupError>>,
{
    match cspp
        .load_master_key_from_store()
        .map_err_prefix("load local master key", CloudBackupError::Internal)?
    {
        Some(master_key) => Ok(master_key),
        None => recover_missing().await,
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use act_zero::call;
    use cove_cspp::CsppStore;
    use cove_cspp::backup_data::{
        WalletEntry, WalletMode as CloudWalletMode, WalletSecret, wallet_filename_from_record_id,
        wallet_record_id,
    };
    use cove_device::cloud_storage::{
        CloudAccessPolicy, CloudStorage, CloudStorageError, CloudSyncHealth,
    };
    use cove_device::keychain::Keychain;
    use cove_device::passkey::{
        DiscoveredPasskeyResult, PasskeyAccess, PasskeyError, PasskeyFailureReason,
        PasskeyOperation,
    };

    use super::test_support::*;
    use super::*;
    use crate::database::Database;
    use crate::database::cloud_backup::{
        CloudBlobDirtyState, CloudBlobFailedState, CloudBlobFailureIssue,
        CloudBlobUploadedPendingConfirmationState, CloudBlobUploadingState,
        PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBlobState,
        PersistedCloudBlobSyncState,
    };
    use crate::label_manager::LabelManager;
    use crate::manager::cloud_backup_manager::{
        CLOUD_BACKUP_MANAGER, CloudBackupDetailResult, CloudBackupKeychain,
        CloudBackupOtherBackupsState, CloudBackupPromptIntent, DeepVerificationFailure,
        DeepVerificationReport, DeepVerificationResult, PendingVerificationCompletion,
        PendingVerificationUpload, VerificationState,
    };
    use crate::manager::cloud_backup_manager::{
        SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE, cspp_master_key_record_id,
        keychain::{
            CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY,
            CloudBackupKeychainError,
        },
    };
    use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};
    use crate::manager::wallet_manager::RustWalletManager;
    use crate::wallet::{
        Wallet,
        metadata::{WalletMetadata, WalletMode, WalletType},
    };
    use bip39::Mnemonic;

    fn platform_authorization_failed() -> PasskeyError {
        PasskeyError::RequestFailed {
            operation: PasskeyOperation::DiscoverAssertion,
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
        }
    }

    async fn wait_for_discover_count(globals: &TestGlobals, expected_count: usize) {
        for _ in 0..20 {
            if globals.passkey.discover_count() == expected_count {
                return;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(globals.passkey.discover_count(), expected_count);
    }

    mod cove_tokio {
        pub(super) fn init() {
            super::init_test_runtime();
        }
    }

    fn init_manager() -> Arc<RustCloudBackupManager> {
        init_test_runtime();
        RustCloudBackupManager::init()
    }

    fn seed_verifiable_cloud_master_key(globals: &TestGlobals) {
        let prf_key = [7u8; 32];
        let prf_salt = [9u8; 32];
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap();
        let encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &prf_salt)
                .unwrap();

        globals.cloud.set_master_key_backup(namespace, serde_json::to_vec(&encrypted).unwrap());
        CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));
    }

    fn global_manager() -> Arc<RustCloudBackupManager> {
        init_test_runtime();
        CLOUD_BACKUP_MANAGER.clone()
    }

    fn persist_pending_master_key_confirmation(namespace_id: String) {
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id,
                wallet_id: None,
                record_id: crate::manager::cloud_backup_manager::cspp_master_key_record_id(),
                state: PersistedCloudBlobState::UploadedPendingConfirmation(
                    CloudBlobUploadedPendingConfirmationState {
                        revision_hash: "master-key-wrapper".into(),
                        uploaded_at: jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
                        attempt_count: 0,
                        last_checked_at: None,
                    },
                ),
            })
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_treats_missing_credential_as_no_match() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Err(PasskeyError::NoCredentialFound));

        let outcome = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[namespace])
        .await
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_treats_user_cancel_as_user_declined() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let outcome = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[namespace])
        .await
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::UserDeclined));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_mixed_supported_and_unsupported_versions_returns_no_match() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let supported_namespace = format!("{}-supported", master_key.namespace_id());
        let unsupported_namespace = format!("{}-unsupported", master_key.namespace_id());
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        let mut unsupported_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        unsupported_master.version = 2;

        globals.cloud.set_master_key_backup(
            supported_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            unsupported_namespace.clone(),
            serde_json::to_vec(&unsupported_master).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![8; 32],
            credential_id: vec![1, 2, 3],
        }));

        let outcome = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[supported_namespace, unsupported_namespace])
        .await
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_discovery_propagates_unsupported_provider() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

        let result = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[namespace])
        .await;
        let error = match result {
            Ok(_) => panic!("expected unsupported passkey provider error"),
            Err(error) => error,
        };

        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_targeted_auth_propagates_unsupported_provider() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let first_namespace = format!("{}-first", master_key.namespace_id());
        let second_namespace = format!("{}-second", master_key.namespace_id());
        let first_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        let second_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[8; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            first_namespace.clone(),
            serde_json::to_vec(&first_encrypted).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            second_namespace.clone(),
            serde_json::to_vec(&second_encrypted).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![1; 32],
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Err(PasskeyError::PrfUnsupportedProvider));

        let result = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[first_namespace, second_namespace])
        .await;
        let error = match result {
            Ok(_) => panic!("expected unsupported passkey provider error"),
            Err(error) => error,
        };

        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passkey_match_allows_one_credential_to_match_multiple_namespaces() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let prf_key = [7u8; 32];
        let first_master_key = cove_cspp::master_key::MasterKey::generate();
        let second_master_key = cove_cspp::master_key::MasterKey::generate();
        let first_namespace = first_master_key.namespace_id();
        let second_namespace = second_master_key.namespace_id();
        let first_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
                .unwrap();
        let second_encrypted = cove_cspp::master_key_crypto::encrypt_master_key(
            &second_master_key,
            &prf_key,
            &[8; 32],
        )
        .unwrap();

        globals.cloud.set_master_key_backup(
            first_namespace.clone(),
            serde_json::to_vec(&first_encrypted).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            second_namespace.clone(),
            serde_json::to_vec(&second_encrypted).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

        let outcome = NamespacePasskeyMatcher::new(
            &CloudStorage::global_explicit_client(),
            PasskeyAccess::global(),
        )
        .match_namespaces(&[first_namespace.clone(), second_namespace.clone()])
        .await
        .unwrap();

        let NamespaceMatchOutcome::Matched(matches) = outcome else {
            panic!("expected multiple namespace matches");
        };
        let matched_namespaces =
            matches.into_iter().map(|matched| matched.namespace_id).collect::<Vec<_>>();

        assert_eq!(matched_namespaces, vec![first_namespace, second_namespace]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_master_key_upload_persists_uploaded_bytes() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let namespace = "namespace-1".to_string();
        let uploaded = vec![1, 2, 3, 4];
        CloudStorage::global_explicit_client()
            .upload_master_key_backup(namespace.clone(), uploaded.clone())
            .await
            .unwrap();

        assert_eq!(
            CloudStorage::global_explicit_client()
                .download_master_key_backup(namespace)
                .await
                .unwrap(),
            uploaded
        );
    }

    #[test]
    fn persist_xpub_wallets_saves_each_wallet_in_its_own_scope() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let first_wallet = xpub_only_wallet_metadata();
        let mut second_wallet = xpub_only_wallet_metadata();
        second_wallet.wallet_mode = WalletMode::Decoy;

        Database::global()
            .wallets()
            .save_all_wallets(first_wallet.network, first_wallet.wallet_mode, Vec::new())
            .unwrap();
        Database::global()
            .wallets()
            .save_all_wallets(second_wallet.network, second_wallet.wallet_mode, Vec::new())
            .unwrap();

        persist_xpub_wallets(vec![first_wallet.clone(), second_wallet.clone()]);

        assert!(
            Database::global()
                .wallets()
                .get(&first_wallet.id, first_wallet.network, first_wallet.wallet_mode)
                .unwrap()
                .is_some()
        );
        assert!(
            Database::global()
                .wallets()
                .get(&second_wallet.id, second_wallet.network, second_wallet.wallet_mode)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wrapper_repair_discovery_propagates_unsupported_provider() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();
        globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

        let acquirer = PasskeyMaterialAcquirer::new(PasskeyAccess::global());
        let discovery_result = acquirer.discover_or_create_for_wrapper_repair().await;
        let error = match discovery_result {
            Ok(_) => panic!("expected unsupported passkey provider error"),
            Err(error) => error,
        };

        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backup_wallets_uploads_when_cloud_backup_is_enabled() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 3);

        let metadata = xpub_only_wallet_metadata();
        let xpub = sample_xpub(&metadata);
        Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();

        manager.do_backup_wallets(&[metadata]).await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(4));
        assert!(Database::global().cloud_blob_sync_states.list().unwrap().into_iter().any(
            |state| matches!(state.state, PersistedCloudBlobState::UploadedPendingConfirmation(_))
        ));
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn backup_new_wallet_marks_verification_required() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 3);

        manager.backup_new_wallet(xpub_only_wallet_metadata());

        let state = Database::global().cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Unverified);
        assert!(state.last_verification_requested_at.is_some());

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backup_new_wallet_still_tracks_when_runtime_status_is_error() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        globals.reset();

        let namespace = "test-namespace".to_string();
        CloudBackupKeychain::global().save_namespace_id(&namespace).unwrap();
        Database::global().cloud_blob_sync_states.delete_all().unwrap();
        manager
            .persist_cloud_backup_state(
                &PersistedCloudBackupState {
                    status: PersistedCloudBackupStatus::Enabled,
                    wallet_count: Some(3),
                    ..PersistedCloudBackupState::default()
                },
                "set cloud backup enabled for test",
            )
            .unwrap();
        manager.sync_persisted_state();

        let metadata = xpub_only_wallet_metadata();
        let record_id = wallet_record_id(metadata.id.as_ref());
        manager.set_status(CloudBackupStatus::Error("offline".into()));

        manager.backup_new_wallet(metadata);

        let state = Database::global().cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Unverified);
        assert!(state.last_verification_requested_at.is_some());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));

        manager.clear_wallet_upload_debouncers_for_test().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_downloaded_wallet_does_not_reupload_wallet_or_mutate_backup_counts() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 5);

        let metadata = xpub_only_wallet_metadata();
        let wallet = DownloadedWalletBackup {
            metadata: metadata.clone(),
            entry: WalletEntry {
                wallet_id: metadata.id.to_string(),
                secret: WalletSecret::WatchOnly,
                metadata: serde_json::to_value(&metadata).unwrap(),
                descriptors: None,
                xpub: Some(sample_xpub(&metadata)),
                wallet_mode: CloudWalletMode::Main,
                labels_zstd_jsonl: None,
                labels_count: 0,
                labels_hash: None,
                labels_uncompressed_size: None,
                content_revision_hash: "test-content-hash".to_string(),
                updated_at: 42,
            },
        };

        WalletRestoreSession::new(Vec::new()).restore_downloaded(&wallet).unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(5));
        assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
        assert!(
            Database::global()
                .wallets()
                .get(&metadata.id, metadata.network, WalletMode::Main)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_downloaded_wallet_restores_labels_without_marking_cloud_backup_dirty() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 5);

        let metadata = xpub_only_wallet_metadata();
        let wallet = DownloadedWalletBackup {
            metadata: metadata.clone(),
            entry: wallet_entry_with_labels(&metadata, Some(sample_labels_jsonl())),
        };

        let outcome = WalletRestoreSession::new(Vec::new()).restore_downloaded(&wallet).unwrap();

        assert!(outcome.labels_warning.is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(5));
        assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());

        let exported = LabelManager::new(metadata.id.clone()).export().await.unwrap();
        assert!(exported.contains("\"label\":\"last txn received\""));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cloud_action_uses_existing_master_key_without_recovery() {
        cove_tokio::init();
        let store = Arc::new(MockStore::default());
        let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
        let expected = cove_cspp::master_key::MasterKey::generate();
        cspp.save_master_key(&expected).unwrap();

        let recovered = load_master_key_for_cloud_action(&cspp, || async {
            Err(CloudBackupError::RecoveryRequired("unexpected".into()))
        })
        .await
        .unwrap();

        assert_eq!(recovered.as_bytes(), expected.as_bytes());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cloud_action_does_not_create_master_key_when_missing() {
        cove_tokio::init();
        let store = Arc::new(MockStore::default());
        let cspp = cove_cspp::Cspp::new(MockStoreHandle(store.clone()));

        let result = load_master_key_for_cloud_action(&cspp, || async {
            Err(CloudBackupError::RecoveryRequired("needs recovery".into()))
        })
        .await;

        assert!(matches!(
            result,
            Err(CloudBackupError::RecoveryRequired(message)) if message == "needs recovery"
        ));
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(*store.save_count.lock(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_master_key_fallback_persists_namespace_id() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let store = Arc::new(MockStore::default());
        let store_handle = MockStoreHandle(store.clone());
        let cspp = cove_cspp::Cspp::new(store_handle.clone());
        let expected = cove_cspp::master_key::MasterKey::generate();
        let namespace_id = expected.namespace_id();
        cspp.save_master_key(&expected).unwrap();
        globals.cloud.set_wallet_files(namespace_id.clone(), vec!["wallet-test.json".into()]);

        let (restored, restored_namespace) = super::restore_from_local_master_key_fallback(
            &CloudStorage::global_explicit_client(),
            &store_handle,
            &cspp,
        )
        .await
        .unwrap();

        assert_eq!(restored.as_bytes(), expected.as_bytes());
        assert_eq!(restored_namespace, namespace_id.clone());
        assert_eq!(
            store_handle.get(CSPP_NAMESPACE_ID_KEY.into()).as_deref(),
            Some(namespace_id.as_str())
        );
    }

    #[test]
    fn save_passkey_rolls_back_on_second_save_failure() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.set_entries(vec![
            (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
            (CSPP_PRF_SALT_KEY, "old_salt"),
        ]);
        globals.keychain.fail_save_at(2);

        let error = CloudBackupKeychain::global().save_passkey(&[1, 2, 3], [7; 32]).unwrap_err();

        assert!(matches!(
            error,
            CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
        ));
        assert_eq!(
            globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
            Some("old_credential")
        );
        assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
    }

    #[test]
    fn save_passkey_and_namespace_rolls_back_on_third_save_failure() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.set_entries(vec![
            (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
            (CSPP_PRF_SALT_KEY, "old_salt"),
            (CSPP_NAMESPACE_ID_KEY, "old_namespace"),
        ]);
        globals.keychain.fail_save_at(3);

        let error = CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3], [9; 32], "new_namespace")
            .unwrap_err();

        assert!(matches!(
            error,
            CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
        ));
        assert_eq!(
            globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
            Some("old_credential")
        );
        assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
        assert_eq!(
            globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(),
            Some("old_namespace")
        );
    }

    #[test]
    fn load_credential_id_returns_none_for_invalid_hex_and_decodes_valid_hex() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, "not-hex")]);

        assert!(CloudBackupKeychain::global().load_credential_id().is_none());

        let credential_id = vec![1, 2, 3, 254, 255];
        let credential_hex = hex::encode(&credential_id);
        globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, &credential_hex)]);

        assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(credential_id));
    }

    #[test]
    fn clear_passkey_removes_credential_and_salt_only() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.set_entries(vec![
            (CSPP_CREDENTIAL_ID_KEY, "credential"),
            (CSPP_PRF_SALT_KEY, "salt"),
            (CSPP_NAMESPACE_ID_KEY, "namespace"),
        ]);

        CloudBackupKeychain::global().clear_passkey();

        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert_eq!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(), Some("namespace"));
    }

    #[test]
    fn clear_local_state_treats_empty_keychain_as_success() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        CloudBackupKeychain::global().clear_local_state().unwrap();
        assert!(CloudBackupKeychain::global().namespace_id().is_none());
    }

    #[test]
    fn clear_local_state_removes_master_key_and_passkey_metadata() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let master_key = cove_cspp::master_key::MasterKey::generate();
        cspp.save_master_key(&master_key).unwrap();
        cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();

        assert!(cspp.load_master_key_from_store().unwrap().is_some());
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_some());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_some());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_some());

        cloud_keychain.clear_local_state().unwrap();

        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    }

    #[test]
    fn clear_local_state_attempts_passkey_metadata_after_master_key_delete_failure() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let master_key = cove_cspp::master_key::MasterKey::generate();
        cspp.save_master_key(&master_key).unwrap();
        cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();

        globals.keychain.fail_delete_at(1);

        let error = cloud_keychain.clear_local_state().unwrap_err();

        assert!(matches!(
            error,
            CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Delete)
        ));
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_master_key_fallback_is_unavailable_after_local_cloud_state_clear() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace_id = master_key.namespace_id();
        cspp.save_master_key(&master_key).unwrap();
        globals.cloud.set_wallet_files(namespace_id, vec!["wallet-test.json".into()]);

        CloudBackupKeychain::global().clear_local_state().unwrap();

        let fallback =
            try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
                .await
                .unwrap();

        assert!(fallback.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn complete_recovery_rolls_back_local_master_key_when_wallet_upload_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        persist_xpub_wallets(vec![xpub_only_wallet_metadata()]);
        globals.cloud.fail_wallet_backup_upload("upload failed");

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let matched = NamespaceMatch {
            namespace_id: "matched-namespace".into(),
            master_key: cove_cspp::master_key::MasterKey::generate(),
            prf_salt: [9; 32],
            credential_id: vec![1, 2, 3],
        };

        let error = manager
            .complete_recovery(
                &cloud_keychain,
                &CloudStorage::global_explicit_client(),
                &cspp,
                vec![matched],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, CloudBackupError::CloudStorage(_)));
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn complete_recovery_rolls_back_local_master_key_when_keychain_save_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        globals.keychain.fail_save_at(3);

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let matched = NamespaceMatch {
            namespace_id: "matched-namespace".into(),
            master_key: cove_cspp::master_key::MasterKey::generate(),
            prf_salt: [9; 32],
            credential_id: vec![1, 2, 3],
        };

        let error = manager
            .complete_recovery(
                &cloud_keychain,
                &CloudStorage::global_explicit_client(),
                &cspp,
                vec![matched],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, CloudBackupError::Internal(_)));
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_from_local_master_key_propagates_store_read_errors() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let store = Arc::new(MockStore::default());
        let store_handle = MockStoreHandle(store.clone());
        let cspp = cove_cspp::Cspp::new(store_handle);
        let expected = cove_cspp::master_key::MasterKey::generate();
        cspp.save_master_key(&expected).unwrap();
        let key_to_corrupt =
            store.entries.lock().keys().next().cloned().expect("saved master key entry");
        store.entries.lock().insert(key_to_corrupt, "not-a-valid-master-key".into());

        let error =
            match try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
                .await
            {
                Ok(_) => panic!("expected local master key read failure"),
                Err(error) => error,
            };

        assert!(matches!(
            error,
            CloudBackupError::Internal(message)
                if message.starts_with("loading master key from store:")
        ));
    }

    #[test]
    fn blocking_cloud_error_rewrites_unavailable_storage_errors_to_offline() {
        let error = blocking_cloud_error(
            BlockingCloudStep::Enable,
            CloudBackupError::CloudStorage(CloudStorageError::NotAvailable(
                "iCloud Drive is not available".into(),
            )),
        );

        assert!(matches!(error, CloudBackupError::Offline(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_create_new_enable_does_not_persist_passkey_metadata() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();
        globals.cloud.fail_master_key_upload("boom");
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        let manager = init_manager();
        let error = manager.do_enable_cloud_backup_create_new().await.unwrap_err();
        assert!(matches!(
            error,
            CloudBackupError::CloudStorageContext {
                context,
                source: CloudStorageError::UploadFailed(message),
            } if context == "upload master key backup" && message == "boom"
        ));

        let keychain = Keychain::global();
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_no_discovery_enable_does_not_persist_passkey_metadata() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
        }));

        let manager = init_manager();
        let error = manager.do_enable_cloud_backup_no_discovery().await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::Passkey(message) if message.contains("boom")
        ));

        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_create_new_succeeds_with_new_passkey_auth() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

        manager.do_enable_cloud_backup_create_new().await.unwrap();

        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
        let state = manager.state();
        assert!(matches!(state.verification, VerificationState::Idle));
        assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);
        assert!(matches!(state.prompt_intent, CloudBackupPromptIntent::None));
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_some());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_some());

        let discover_count = globals.passkey.discover_count();
        let authenticate_count = globals.passkey.authenticate_count();

        call!(manager.supervisor.start_enter_detail()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(globals.passkey.discover_count(), discover_count);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn detail_entry_starts_discoverable_verification_without_runtime_authorization() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();
        manager.clear_runtime_passkey_authorization();
        manager.clear_pending_verification_completion();
        manager.set_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.set_verification(VerificationState::Idle);
        Database::global().cloud_blob_sync_states.delete_all().unwrap();
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
        globals.cloud.fail_list_wallet_files("list should not run before passkey auth");

        let discover_count = globals.passkey.discover_count();
        let list_count = globals.cloud.list_wallet_files_attempt_count();

        call!(manager.supervisor.start_enter_detail()).await.unwrap();
        wait_for_discover_count(globals, discover_count + 1).await;

        assert_eq!(globals.passkey.discover_count(), discover_count + 1);
        assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_count);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_authenticates_before_loading_wallet_inventory() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        configure_enabled_cloud_backup(&manager, globals, 0);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        seed_verifiable_cloud_master_key(globals);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
        globals.cloud.fail_list_wallet_files("list should not run before passkey auth");

        let discover_count = globals.passkey.discover_count();
        let list_count = globals.cloud.list_wallet_files_attempt_count();

        call!(manager.supervisor.start_verification(true)).await.unwrap();
        wait_for_discover_count(globals, discover_count + 1).await;

        assert_eq!(globals.passkey.discover_count(), discover_count + 1);
        assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_count);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_no_discovery_succeeds_with_new_passkey_auth() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();

        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_with_multiple_matching_namespaces_merges_into_largest_namespace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let prf_key = [7u8; 32];
        let first_master_key = cove_cspp::master_key::MasterKey::generate();
        let second_master_key = cove_cspp::master_key::MasterKey::generate();
        let first_namespace = first_master_key.namespace_id();
        let second_namespace = second_master_key.namespace_id();
        let first_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
                .unwrap();
        let second_encrypted = cove_cspp::master_key_crypto::encrypt_master_key(
            &second_master_key,
            &prf_key,
            &[8; 32],
        )
        .unwrap();

        globals.cloud.set_master_key_backup(
            first_namespace.clone(),
            serde_json::to_vec(&first_encrypted).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            second_namespace.clone(),
            serde_json::to_vec(&second_encrypted).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

        let first_wallet = xpub_only_wallet_metadata();
        let second_wallet = xpub_only_wallet_metadata();
        let third_wallet = xpub_only_wallet_metadata();
        let first_wallet = WalletMetadata { master_fingerprint: None, ..first_wallet };
        let second_wallet = WalletMetadata { master_fingerprint: None, ..second_wallet };
        let third_wallet = WalletMetadata { master_fingerprint: None, ..third_wallet };
        Keychain::global()
            .save_wallet_xpub(&first_wallet.id, sample_xpub(&first_wallet).parse().unwrap())
            .unwrap();
        Keychain::global()
            .save_wallet_xpub(&second_wallet.id, sample_xpub(&second_wallet).parse().unwrap())
            .unwrap();
        Keychain::global()
            .save_wallet_xpub(&third_wallet.id, sample_xpub(&third_wallet).parse().unwrap())
            .unwrap();

        let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
        let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
        let third_record_id = cove_cspp::backup_data::wallet_record_id(third_wallet.id.as_ref());
        let first_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &first_wallet,
            first_wallet.wallet_mode,
        )
        .await
        .unwrap()
        .revision_hash;
        let second_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &second_wallet,
            second_wallet.wallet_mode,
        )
        .await
        .unwrap()
        .revision_hash;
        let third_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &third_wallet,
            third_wallet.wallet_mode,
        )
        .await
        .unwrap()
        .revision_hash;
        globals.cloud.set_wallet_backup(
            first_namespace.clone(),
            first_record_id.clone(),
            encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, &first_revision, 1)
                .await,
        );
        globals.cloud.set_wallet_backup(
            second_namespace.clone(),
            second_record_id.clone(),
            encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, &second_revision, 1)
                .await,
        );
        globals.cloud.set_wallet_backup(
            second_namespace.clone(),
            third_record_id.clone(),
            encrypted_wallet_backup_bytes(&third_wallet, &second_master_key, &third_revision, 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            first_namespace.clone(),
            vec![wallet_filename_from_record_id(&first_record_id)],
        );
        globals.cloud.set_wallet_files(
            second_namespace.clone(),
            vec![
                wallet_filename_from_record_id(&second_record_id),
                wallet_filename_from_record_id(&third_record_id),
            ],
        );

        manager.do_enable_cloud_backup().await.unwrap();

        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
        assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(second_namespace.clone()));
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(3));
        assert!(globals.cloud.has_namespace(&second_namespace));
        wait_for_test_condition(
            Duration::from_secs(1),
            "merged source namespace should be deleted after proof",
            || !globals.cloud.has_namespace(&first_namespace),
        )
        .await;

        let active_records = CloudStorage::global_explicit_client()
            .list_wallet_backups(second_namespace)
            .await
            .unwrap();
        assert!(active_records.contains(&first_record_id));
        assert!(active_records.contains(&second_record_id));
        assert!(active_records.contains(&third_record_id));
    }

    async fn enqueue_cleanup_for_test(
        manager: &RustCloudBackupManager,
        active_namespace: String,
        active_master_key: &cove_cspp::master_key::MasterKey,
        source_namespace: String,
        record_id: String,
        revision_hash: Option<String>,
    ) {
        manager.enqueue_cleanup(CloudBackupCleanupJob {
            cloud: CloudStorage::global_explicit_client(),
            active_namespace_id: active_namespace,
            active_critical_key: active_master_key.critical_data_key(),
            sources: vec![CleanupSourceNamespace {
                namespace_id: source_namespace,
                expected_wallets: vec![CleanupExpectedWalletRecord {
                    record_id,
                    content_revision_hash: revision_hash,
                }],
            }],
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_deletes_source_namespace_after_active_record_proof() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &active_master_key, "matching-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            active_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("matching-revision".into()),
        )
        .await;

        assert!(!globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_keeps_source_namespace_when_active_record_is_missing() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let record_id = "missing-record".to_string();
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("expected-revision".into()),
        )
        .await;

        assert!(globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_keeps_source_namespace_when_active_record_is_undecryptable() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let wrong_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &wrong_master_key, "expected-revision", 1).await,
        );
        globals.cloud.set_wallet_files(
            active_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("expected-revision".into()),
        )
        .await;

        assert!(globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_keeps_source_namespace_when_active_record_is_unsupported() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 2)
                .await,
        );
        globals.cloud.set_wallet_files(
            active_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("expected-revision".into()),
        )
        .await;

        assert!(globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_keeps_source_namespace_when_active_revision_mismatches() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &active_master_key, "actual-revision", 1).await,
        );
        globals.cloud.set_wallet_files(
            active_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("expected-revision".into()),
        )
        .await;

        assert!(globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_keeps_source_namespace_when_delete_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        let active_master_key = cove_cspp::master_key::MasterKey::generate();
        let active_namespace = active_master_key.namespace_id();
        let source_namespace = "source-namespace".to_string();
        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            active_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.set_wallet_files(
            source_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.fail_delete_namespace("delete failed");

        enqueue_cleanup_for_test(
            &manager,
            active_namespace,
            &active_master_key,
            source_namespace.clone(),
            record_id,
            Some("expected-revision".into()),
        )
        .await;

        assert!(globals.cloud.has_namespace(&source_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn finalize_passkey_repair_keeps_existing_count_when_wallet_refresh_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 2);

        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::PasskeyMissing,
                wallet_count: Some(7),
                ..PersistedCloudBackupState::default()
            })
            .unwrap();
        manager.sync_persisted_state();
        globals.cloud.fail_list_wallet_files("timed out");

        manager.finalize_passkey_repair().await.unwrap();

        let state = Database::global().cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Enabled);
        assert_eq!(state.wallet_count, Some(7));
        assert_eq!(manager.state().status, CloudBackupStatus::Enabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wrapper_repair_refreshes_missing_master_key_sync_health_to_uploading() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
        manager.set_sync_health(CloudSyncHealth::Failed(
            SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into(),
        ));

        manager.do_repair_passkey_wrapper_no_discovery().await.unwrap();
        manager.finalize_passkey_repair().await.unwrap();

        for _ in 0..20 {
            if manager.state().sync_health == CloudSyncHealth::Uploading {
                break;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(manager.state().sync_health, CloudSyncHealth::Uploading);

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reupload_all_wallets_does_not_create_master_key_for_existing_namespace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        CloudBackupKeychain::global().save_namespace_id("existing-namespace").unwrap();

        let manager = init_manager();
        let error = manager.do_reupload_all_wallets().await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::RecoveryRequired(message)
                if message == RECREATE_MANIFEST_RECOVERY_MESSAGE
        ));

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn reupload_all_wallets_persists_full_cloud_wallet_count() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
        globals
            .cloud
            .set_wallet_files(namespace, vec![wallet_filename_from_record_id("cloud-only-record")]);

        manager.do_reupload_all_wallets().await.unwrap();

        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(2));
        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_cloud_only_wallets_surfaces_unsupported_versions() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let keychain = Keychain::global();
        let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let wallets = manager.do_fetch_cloud_only_wallets().await.unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].record_id, record_id);
        assert_eq!(wallets[0].name, UNSUPPORTED_CLOUD_ONLY_WALLET_NAME);
        assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
        assert_eq!(wallets[0].network, None);
        assert_eq!(wallets[0].wallet_mode, None);
        assert_eq!(wallets[0].wallet_type, None);
        assert_eq!(wallets[0].label_count, None);
        assert_eq!(wallets[0].backup_updated_at, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn detail_reports_other_backup_namespaces() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
        globals.cloud.set_wallet_files(
            current_namespace,
            vec![wallet_filename_from_record_id("current-wallet")],
        );

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.set_wallet_files(
            other_namespace,
            vec![
                wallet_filename_from_record_id("other-wallet-1"),
                wallet_filename_from_record_id("other-wallet-2"),
            ],
        );

        let Some(CloudBackupDetailResult::Success(detail)) =
            manager.refresh_cloud_backup_detail().await
        else {
            panic!("expected cloud backup detail");
        };

        let CloudBackupOtherBackupsState::Loaded { summary } = detail.other_backups else {
            panic!("expected loaded other backups");
        };
        assert_eq!(summary.namespace_count, 1);
        assert_eq!(summary.wallet_count, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn other_backup_summary_counts_only_wallets_missing_from_current_namespace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_wallet_files(
            current_namespace.clone(),
            ["wallet-1", "wallet-2", "wallet-3", "wallet-4"]
                .into_iter()
                .map(wallet_filename_from_record_id)
                .collect(),
        );

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
        globals.cloud.set_wallet_files(
            other_namespace,
            ["wallet-1", "wallet-2", "wallet-3", "wallet-4", "wallet-5"]
                .into_iter()
                .map(wallet_filename_from_record_id)
                .collect(),
        );

        let summary =
            manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
        assert_eq!(summary.namespace_count, 1);
        assert_eq!(summary.wallet_count, 1);

        globals.cloud.set_wallet_files(
            current_namespace,
            ["wallet-1", "wallet-2", "wallet-3", "wallet-4", "wallet-5"]
                .into_iter()
                .map(wallet_filename_from_record_id)
                .collect(),
        );

        let summary =
            manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
        assert_eq!(summary.namespace_count, 1);
        assert_eq!(summary.wallet_count, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn detail_refresh_marks_other_backups_failed_when_namespace_inspection_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_wallet_files(current_namespace, Vec::new());

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
        globals.cloud.fail_master_key_download_offline(
            other_namespace,
            "offline while inspecting namespace",
        );

        let Some(CloudBackupDetailResult::Success(detail)) =
            manager.refresh_cloud_backup_detail().await
        else {
            panic!("expected cloud backup detail");
        };

        let CloudBackupOtherBackupsState::LoadFailed { error } = detail.other_backups else {
            panic!("expected failed other backups state");
        };
        assert_eq!(
            error,
            "offline: Reconnect to the internet, then try refreshing cloud backup details again"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_other_backups_keeps_current_passkey_metadata() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[9, 8, 7], [6; 32], &current_namespace)
            .unwrap();

        let prf_key = [7u8; 32];
        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));

        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            other_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
        );
        globals.cloud.set_wallet_files(
            other_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        let report = manager.do_recover_other_backups().await.unwrap();

        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 0);
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(!globals.cloud.has_namespace(&other_namespace));
        assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(current_namespace.clone()));
        assert_eq!(
            globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(),
            Some(current_namespace.as_str())
        );
        assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(vec![9, 8, 7]));
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());

        let summary =
            manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
        assert_eq!(summary.namespace_count, 0);
        assert_eq!(summary.wallet_count, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_other_backups_keeps_partially_moved_namespace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let prf_key = [7u8; 32];
        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));

        let restored_wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&restored_wallet.id, sample_xpub(&restored_wallet).parse().unwrap())
            .unwrap();
        let restored_record_id =
            cove_cspp::backup_data::wallet_record_id(restored_wallet.id.as_ref());
        let missing_wallet = xpub_only_wallet_metadata();
        let missing_record_id =
            cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());

        globals.cloud.set_wallet_backup(
            other_namespace.clone(),
            restored_record_id.clone(),
            encrypted_wallet_backup_bytes(&restored_wallet, &other_master_key, "other-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            other_namespace.clone(),
            vec![
                wallet_filename_from_record_id(&restored_record_id),
                wallet_filename_from_record_id(&missing_record_id),
            ],
        );

        let report = manager.do_recover_other_backups().await.unwrap();

        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 1);
        assert!(globals.cloud.has_namespace(&other_namespace));

        let summary =
            manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
        assert_eq!(summary.namespace_count, 1);
        assert_eq!(summary.wallet_count, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_other_backups_keeps_namespace_when_current_upload_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        globals.cloud.fail_wallet_backup_upload("upload failed");

        let prf_key = [7u8; 32];
        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));

        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            other_namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
        );
        globals.cloud.set_wallet_files(
            other_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );

        let result = manager.do_recover_other_backups().await;

        assert!(result.is_err());
        assert!(globals.cloud.has_namespace(&other_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_other_backups_returns_offline_when_wallet_download_is_offline() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let prf_key = [7u8; 32];
        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));

        let wallet = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        globals.cloud.set_wallet_files(
            other_namespace.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        globals.cloud.fail_wallet_backup_download_offline(
            other_namespace,
            record_id,
            "offline while downloading wallet",
        );

        let result = manager.do_recover_other_backups().await;

        match result {
            Err(CloudBackupError::Offline(message)) => {
                assert_eq!(
                    message,
                    "Reconnect to the internet, then try recovering the other cloud backups again"
                );
            }
            Ok(report) => panic!(
                "expected offline error, got report with {} failed wallet(s)",
                report.wallets_failed
            ),
            Err(error) => panic!("expected offline error, got {error:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recover_other_backups_returns_offline_when_namespace_inspection_is_offline() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.fail_master_key_download_offline(
            other_namespace,
            "offline while inspecting namespace",
        );

        let result = manager.do_recover_other_backups().await;

        match result {
            Err(CloudBackupError::Offline(message)) => {
                assert_eq!(
                    message,
                    "Reconnect to the internet, then try recovering the other cloud backups again"
                );
            }
            Ok(report) => panic!(
                "expected offline error, got report with {} restored wallet(s)",
                report.wallets_restored
            ),
            Err(error) => panic!("expected offline error, got {error:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_other_backups_removes_only_non_current_namespaces() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
        globals.cloud.set_wallet_files(
            current_namespace.clone(),
            vec![wallet_filename_from_record_id("current-wallet")],
        );

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.set_wallet_files(
            other_namespace.clone(),
            vec![wallet_filename_from_record_id("other-wallet")],
        );

        manager.do_delete_other_backups().await.unwrap();

        assert!(globals.cloud.has_namespace(&current_namespace));
        assert!(!globals.cloud.has_namespace(&other_namespace));
        assert_eq!(
            globals.cloud.deleted_namespace_policies(),
            vec![CloudAccessPolicy::ConsentAllowed]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_other_backups_returns_offline_when_namespace_inspection_is_offline() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);

        let other_master_key = cove_cspp::master_key::MasterKey::generate();
        let other_namespace = other_master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            other_namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.fail_master_key_download_offline(
            other_namespace.clone(),
            "offline while inspecting namespace",
        );

        let result = manager.do_delete_other_backups().await;

        match result {
            Err(CloudBackupError::Offline(message)) => {
                assert_eq!(
                    message,
                    "Reconnect to the internet, then try deleting the other cloud backups again"
                );
            }
            Ok(()) => panic!("expected offline error"),
            Err(error) => panic!("expected offline error, got {error:?}"),
        }
        assert!(globals.cloud.has_namespace(&other_namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backup_wallets_does_not_create_master_key_or_upload_when_missing() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let namespace = "existing-namespace";
        CloudBackupKeychain::global().save_namespace_id(namespace).unwrap();

        let manager = init_manager();
        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = crate::wallet::metadata::WalletType::WatchOnly;

        let error = manager.do_backup_wallets(&[metadata]).await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::RecoveryRequired(message)
                if message == "Cloud backup needs verification before wallets can be uploaded"
        ));

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_does_not_create_master_key_for_existing_namespace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        globals.reset();

        let namespace = "existing-namespace";
        CloudBackupKeychain::global().save_namespace_id(namespace).unwrap();

        let manager = init_manager();
        let metadata = xpub_only_wallet_metadata();
        let xpub = sample_xpub(&metadata);
        Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();

        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id: namespace.into(),
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();

        let error = manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::RecoveryRequired(message)
                if message == "Cloud backup needs verification before wallets can be uploaded"
        ));

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }),
                ..
            })
        ));
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn deferred_live_wallet_upload_retries_without_restart() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_dirty_blob_state(metadata.id.clone());
        globals.cloud.fail_next_wallet_backup_upload_offline("offline");

        run_wallet_upload_for_test_async(&manager, metadata.id.clone()).await;

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
        assert!(manager.state().sync_error.is_none());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));

        wait_for_test_condition(
            Duration::from_secs(7),
            "deferred live upload should retry automatically after the backoff",
            || globals.cloud.wallet_backup_upload_attempt_count() >= 2,
        )
        .await;

        wait_for_test_condition(
            Duration::from_secs(1),
            "deferred live upload should eventually reach an uploaded state",
            || {
                matches!(
                    Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                    Some(PersistedCloudBlobSyncState {
                        state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                            | PersistedCloudBlobState::Confirmed(_),
                        ..
                    })
                )
            },
        )
        .await;
        assert!(globals.cloud.uploaded_wallet_backup_count() >= 1);
        assert!(manager.state().sync_error.is_none());

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn permanent_failed_wallet_upload_does_not_retry_without_restart() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_dirty_blob_state(metadata.id.clone());
        globals.cloud.fail_wallet_backup_upload_quota_exceeded();

        run_wallet_upload_for_test_async(&manager, metadata.id.clone()).await;

        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }),
                ..
            })
        ));

        assert_test_condition_stays_true(
            crate::manager::cloud_backup_manager::LIVE_UPLOAD_DEBOUNCE + Duration::from_secs(1),
            "non-retryable upload should not retry without restart",
            || globals.cloud.wallet_backup_upload_attempt_count() == 1,
        )
        .await;

        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);

        clear_wallet_upload_runtime_for_test_async(&manager).await;
        globals.cloud.clear_wallet_backup_upload_failure();
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_error_clears_only_after_last_failed_wallet_upload_recovers() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let first_wallet = xpub_only_wallet_metadata();
        let second_wallet = xpub_only_wallet_metadata();
        let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
        let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());

        persist_xpub_wallets(vec![first_wallet.clone(), second_wallet.clone()]);
        persist_dirty_blob_state(first_wallet.id.clone());
        persist_dirty_blob_state(second_wallet.id.clone());
        globals.cloud.fail_wallet_backup_upload("upload failed");

        run_wallet_upload_for_test_async(&manager, first_wallet.id.clone()).await;
        run_wallet_upload_for_test_async(&manager, second_wallet.id.clone()).await;

        assert!(manager.state().sync_error.is_some());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&first_record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
        ));
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
        ));

        globals.cloud.clear_wallet_backup_upload_failure();

        run_wallet_upload_for_test_async(&manager, first_wallet.id.clone()).await;

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(manager.state().sync_error.is_some());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&first_record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                    | PersistedCloudBlobState::Confirmed(_),
                ..
            })
        ));
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
        ));

        run_wallet_upload_for_test_async(&manager, second_wallet.id.clone()).await;

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 2);
        assert!(manager.state().sync_error.is_none());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                    | PersistedCloudBlobState::Confirmed(_),
                ..
            })
        ));

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[test]
    fn connectivity_reconnect_preserves_sync_error_when_failed_wallet_uploads_exist() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let wallet_id = xpub_only_wallet_metadata().id;
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id: CloudBackupKeychain::global().namespace_id().unwrap(),
                wallet_id: Some(wallet_id),
                record_id,
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: None,
                    error: "upload failed".into(),
                    retryable: false,
                    issue: None,
                    failed_at: 1,
                }),
            })
            .unwrap();
        manager.set_sync_error(Some("upload failed".into()));

        manager.handle_connectivity_change(ConnectivityStatus::Connected);

        assert_eq!(manager.state().sync_error.as_deref(), Some("upload failed"));
    }

    #[test]
    fn connectivity_reconnect_clears_sync_error_when_failed_wallet_uploads_are_gone() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        manager.set_sync_error(Some("upload failed".into()));

        manager.handle_connectivity_change(ConnectivityStatus::Connected);

        assert!(manager.state().sync_error.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconnect_retries_verification_after_offline_failure() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        seed_verifiable_cloud_master_key(globals);
        CONNECTIVITY_MANAGER.set_connection_state(false);

        call!(manager.supervisor.start_verification(false)).await.unwrap();
        wait_for_test_condition(
            Duration::from_secs(1),
            "expected offline verification failure",
            || matches!(manager.state().verification, VerificationState::Failed(_)),
        )
        .await;

        CONNECTIVITY_MANAGER.set_connection_state(true);

        wait_for_test_condition(
            Duration::from_secs(1),
            "expected reconnect to retry and verify backup",
            || matches!(manager.state().verification, VerificationState::Verified(_)),
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connected_connectivity_failure_retries_detail_refresh_once() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.cloud.fail_next_list_wallet_files_offline("offline");

        call!(manager.supervisor.start_refresh_detail()).await.unwrap();

        wait_for_test_condition(
            Duration::from_secs(1),
            "expected connectivity retry to refresh detail",
            || manager.state().detail.is_some(),
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connected_connectivity_failure_retries_verification_once() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        seed_verifiable_cloud_master_key(globals);
        globals.cloud.fail_next_list_wallet_files_offline("offline");

        call!(manager.supervisor.start_verification(false)).await.unwrap();

        wait_for_test_condition(
            Duration::from_secs(1),
            "expected connected offline failure to retry verification",
            || matches!(manager.state().verification, VerificationState::Verified(_)),
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_connectivity_does_not_block_verification() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        seed_verifiable_cloud_master_key(globals);
        CONNECTIVITY_MANAGER.set_connection_status(ConnectivityStatus::Unknown);

        call!(manager.supervisor.start_verification(false)).await.unwrap();

        wait_for_test_condition(
            Duration::from_secs(1),
            "expected unknown connectivity to attempt verification",
            || matches!(manager.state().verification, VerificationState::Verified(_)),
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_connectivity_verification_failure_does_not_retry_on_reconnect() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.cloud.fail_list_wallet_files("list failed");

        call!(manager.supervisor.start_verification(false)).await.unwrap();
        wait_for_test_condition(
            Duration::from_secs(1),
            "expected non-connectivity verification failure",
            || matches!(manager.state().verification, VerificationState::Failed(_)),
        )
        .await;

        CONNECTIVITY_MANAGER.set_connection_state(false);
        manager.handle_connectivity_change(ConnectivityStatus::Disconnected);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        manager.handle_connectivity_change(ConnectivityStatus::Connected);

        assert_test_condition_stays_true(
            Duration::from_millis(150),
            "non-connectivity failure should stay failed after reconnect",
            || matches!(manager.state().verification, VerificationState::Failed(_)),
        )
        .await;
    }

    #[test]
    fn reset_cloud_backup_test_state_clears_state_before_reconnect() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let wallet_id = xpub_only_wallet_metadata().id;
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id: CloudBackupKeychain::global().namespace_id().unwrap(),
                wallet_id: Some(wallet_id),
                record_id,
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: None,
                    error: "upload failed".into(),
                    retryable: false,
                    issue: None,
                    failed_at: 1,
                }),
            })
            .unwrap();
        manager.set_sync_error(Some("upload failed".into()));
        CONNECTIVITY_MANAGER.set_connection_state(false);

        reset_cloud_backup_test_state_with_hook(&manager, globals, || {
            assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
            assert!(manager.state().sync_error.is_none());
        });

        assert!(CONNECTIVITY_MANAGER.is_connected());
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn startup_resume_skips_non_retryable_failed_wallet_uploads() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_failed_blob_state(metadata.id.clone(), false);
        globals.cloud.fail_wallet_backup_upload_quota_exceeded();
        let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

        manager.resume_pending_cloud_upload_verification();

        assert_test_condition_stays_true(
            Duration::from_millis(250),
            "startup resume should not retry non-retryable failed uploads",
            || globals.cloud.wallet_backup_upload_attempt_count() == initial_attempt_count,
        )
        .await;

        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), initial_attempt_count);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }),
                ..
            })
        ));

        clear_wallet_upload_runtime_for_test_async(&manager).await;
        globals.cloud.clear_wallet_backup_upload_failure();
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn startup_resume_retries_authorization_failed_wallet_uploads() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_failed_blob_state_with_issue(
            metadata.id,
            false,
            Some(CloudBlobFailureIssue::AuthorizationRequired),
        );
        let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

        manager.resume_pending_cloud_upload_verification();

        wait_for_test_condition(
            Duration::from_secs(1),
            "startup resume should set sync error for authorization failures",
            || manager.state().sync_error.as_deref() == Some("failed"),
        )
        .await;
        wait_for_test_condition(
            Duration::from_secs(1),
            "startup resume should retry authorization failures",
            || globals.cloud.wallet_backup_upload_attempt_count() > initial_attempt_count,
        )
        .await;

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn cloud_storage_change_retries_authorization_failed_wallet_uploads() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_failed_blob_state_with_issue(
            metadata.id,
            false,
            Some(CloudBlobFailureIssue::AuthorizationRequired),
        );
        let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

        manager.cloud_storage_did_change();

        wait_for_test_condition(
            Duration::from_secs(3),
            "cloud storage change should retry authorization failures",
            || globals.cloud.wallet_backup_upload_attempt_count() > initial_attempt_count,
        )
        .await;

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_health_reports_authorization_required_for_persisted_auth_failures() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_failed_blob_state_with_issue(
            metadata.id,
            true,
            Some(CloudBlobFailureIssue::AuthorizationRequired),
        );

        assert_eq!(
            manager.compute_sync_health().await,
            CloudSyncHealth::AuthorizationRequired("failed".into()),
        );

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_health_reports_uploading_for_fresh_pending_master_key_confirmation() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        persist_pending_master_key_confirmation(namespace_id.clone());
        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading);

        globals.cloud.set_master_key_backup(namespace_id, vec![1, 2, 3]);

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::AllUploaded);
        assert_eq!(
            manager.state().pending_upload_verification,
            PendingUploadVerificationState::Idle
        );
        assert!(matches!(manager.state().verification, VerificationState::Idle));

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_blocks_on_cloud_authorization() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        persist_pending_master_key_confirmation(namespace_id.clone());
        globals.cloud.fail_master_key_download_authorization_required(
            namespace_id,
            "authorization required",
        );

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(has_more_pending);
        assert_eq!(
            manager.state().pending_upload_verification,
            PendingUploadVerificationState::BlockedOnAuthorization
        );

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_health_reports_uploading_for_fresh_pending_master_key_with_local_wallets() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);
        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        persist_pending_master_key_confirmation(namespace_id);

        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn force_new_reports_uploading_while_master_key_confirmation_is_pending() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
        globals.cloud.set_master_key_backup("existing-namespace".into(), vec![1, 2, 3]);
        globals.cloud.set_wallet_files("existing-namespace".into(), vec!["wallet-1.json".into()]);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();
        manager.do_enable_cloud_backup_force_new().await.unwrap();

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudStorage::global_silent_client()
            .delete_wallet_backup(namespace_id.clone(), cspp_master_key_record_id())
            .await
            .unwrap();
        persist_pending_master_key_confirmation(namespace_id);

        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_health_reports_missing_master_key_without_pending_confirmation() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        assert_eq!(
            manager.compute_sync_health().await,
            CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into()),
        );

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn sync_health_respects_master_key_upload_confirmation_grace() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);
        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();

        assert_eq!(
            manager.compute_sync_health_with_master_key_grace(Some(&namespace_id)).await,
            CloudSyncHealth::Uploading,
        );

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[expect(
        clippy::await_holding_lock,
        reason = "tests serialize shared cloud backup globals across awaits"
    )]
    #[tokio::test(flavor = "current_thread")]
    async fn startup_resume_retries_interrupted_uploading_wallets() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_uploading_blob_state(metadata.id, 1);

        manager.resume_pending_cloud_upload_verification();

        wait_for_test_condition(
            Duration::from_secs(5),
            "startup resume should retry interrupted uploads",
            || {
                let upload_state_is_pending_or_confirmed = matches!(
                    Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                    Some(PersistedCloudBlobSyncState {
                        state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                            | PersistedCloudBlobState::Confirmed(_),
                        ..
                    })
                );

                globals.cloud.wallet_backup_upload_attempt_count() >= 1
                    && upload_state_is_pending_or_confirmed
            },
        )
        .await;

        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn validate_metadata_marks_generated_wallet_names_dirty() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = global_manager();
        reset_cloud_backup_test_state(&manager, globals);

        let mut metadata = WalletMetadata::preview_new();
        metadata.name.clear();

        let wallet = Wallet::try_new_persisted_and_selected(
            metadata,
            Mnemonic::parse(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let wallet_id = wallet.id();
        let stored_metadata = Database::global()
            .wallets()
            .get(&wallet_id, wallet.network, wallet.metadata.wallet_mode)
            .unwrap()
            .unwrap();
        let expected_name = stored_metadata
            .master_fingerprint
            .as_deref()
            .map_or_else(|| "Unnamed Wallet".to_string(), |fingerprint| fingerprint.as_uppercase());

        enable_cloud_backup_without_reset(&manager, 1);

        let wallet_manager = RustWalletManager::try_new(wallet_id.clone()).unwrap();
        wallet_manager.validate_metadata();

        let updated_metadata = Database::global()
            .wallets()
            .get(&wallet_id, wallet.network, wallet.metadata.wallet_mode)
            .unwrap()
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());

        assert_eq!(updated_metadata.name, expected_name);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));

        manager.clear_wallet_upload_debouncers_for_test().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_removes_deleted_wallet_sync_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = WalletMetadata::preview_new();
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id: namespace.clone(),
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();

        manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

        assert!(Database::global().cloud_blob_sync_states.get(&record_id).unwrap().is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(namespace));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_and_integrity_skip_pending_upload_candidates() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id,
                wallet_id: Some(metadata.id.clone()),
                record_id,
                state: PersistedCloudBlobState::UploadedPendingConfirmation(
                    CloudBlobUploadedPendingConfirmationState {
                        revision_hash: "rev-1".into(),
                        uploaded_at: 10,
                        attempt_count: 0,
                        last_checked_at: None,
                    },
                ),
            })
            .unwrap();

        manager.do_sync_unsynced_wallets().await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(1));

        let warning =
            manager.verify_backup_integrity_impl().await.expect("expected passkey warning");

        assert!(!warning.contains("some wallets are not backed up"));
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_does_not_retry_sync_after_auto_backup_failure() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata])
            .unwrap();
        globals.cloud.fail_wallet_backup_upload("offline");

        let warning =
            manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

        assert!(warning.contains("some wallets are not backed up"));
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_warns_when_background_wallet_list_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.fail_list_wallet_files_non_interactive("offline");

        let warning =
            manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

        assert!(warning.contains("wallet backups could not be listed"));
        globals.cloud.clear_list_wallet_files_non_interactive_failure();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refresh_cloud_backup_detail_uses_interactive_wallet_listing() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);

        let keychain = Keychain::global();
        let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
        CloudBackupKeychain::new(keychain.clone())
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "interactive-revision", 1).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);
        globals.cloud.fail_list_wallet_files_non_interactive("offline");

        let Some(CloudBackupDetailResult::Success(detail)) =
            manager.refresh_cloud_backup_detail().await
        else {
            panic!("expected cloud backup detail");
        };

        assert_eq!(1, detail.up_to_date.len() + detail.needs_sync.len());
        let listed_record_id = detail
            .up_to_date
            .first()
            .map(|wallet| wallet.record_id.clone())
            .or_else(|| detail.needs_sync.first().map(|wallet| wallet.record_id.clone()))
            .expect("expected listed wallet");
        assert_eq!(listed_record_id, record_id);
        globals.cloud.clear_list_wallet_files_non_interactive_failure();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_preserves_unsupported_remote_wallet_backups() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);

        let keychain = Keychain::global();
        let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
        CloudBackupKeychain::new(keychain.clone())
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let warning = manager.verify_backup_integrity_impl().await;

        assert!(warning.is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);

        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refresh_cloud_backup_detail_marks_listed_wallet_unknown_when_master_key_is_missing() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        reset_cloud_backup_test_state(&manager, globals);

        let metadata = xpub_only_wallet_metadata();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let keychain = Keychain::global();
        CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
        cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();
        manager
            .persist_cloud_backup_state(
                &PersistedCloudBackupState {
                    status: PersistedCloudBackupStatus::Enabled,
                    wallet_count: Some(1),
                    ..PersistedCloudBackupState::default()
                },
                "set cloud backup enabled for test",
            )
            .unwrap();
        manager.sync_persisted_state();

        persist_xpub_wallets(vec![metadata.clone()]);

        let record_id = wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "rev-1", 1).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let cspp = cove_cspp::Cspp::new(keychain.clone());
        cspp.delete_master_key();
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

        let Some(CloudBackupDetailResult::Success(detail)) =
            manager.refresh_cloud_backup_detail().await
        else {
            panic!("expected cloud backup detail");
        };

        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::RemoteStateUnknown);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_skips_wallets_with_unknown_remote_truth() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        globals
            .cloud
            .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
        globals.cloud.set_wallet_backup(namespace, record_id, b"{".to_vec());

        manager.do_sync_unsynced_wallets().await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_refreshes_detail_after_auto_backup_success() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let warning = manager.verify_backup_integrity_impl().await;

        assert!(warning.is_none());
        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert_eq!(detail.up_to_date.len(), 1);
        assert!(detail.needs_sync.is_empty());
        assert_eq!(detail.up_to_date[0].record_id, record_id);
        assert_eq!(detail.up_to_date[0].sync_status, CloudBackupWalletStatus::Confirmed);
        manager.clear_wallet_upload_debouncers_for_test().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_auto_backup_continues_when_other_backup_summary_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.fail_list_namespaces("offline while listing namespaces");

        let warning = manager.verify_backup_integrity_impl().await;

        assert!(warning.is_none());
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert!(matches!(detail.other_backups, CloudBackupOtherBackupsState::LoadFailed { .. }));
        manager.clear_wallet_upload_debouncers_for_test().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_does_not_retry_sync_after_auto_backup_success_when_listing_stays_empty() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();

        let warning = manager.verify_backup_integrity_impl().await;

        assert!(warning.is_none());
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_refreshes_detail_after_auto_backup_failure() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        CloudBackupKeychain::global()
            .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.fail_wallet_backup_upload("offline");

        let warning =
            manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

        assert!(warning.contains("some wallets are not backed up"));
        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::Dirty);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id,
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();
        globals.cloud.dirty_wallet_on_next_upload(metadata.id.clone());

        manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_retries_stale_uploading_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(metadata.id.clone(), 1);

        manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
                ..
            })
        ));
        manager.clear_wallet_upload_debouncers_for_test().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_recovers_stale_uploading_state_while_offline() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(metadata.id.clone(), 1);
        CONNECTIVITY_MANAGER.set_connection_state(false);

        let error = manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap_err();

        assert!(matches!(error, CloudBackupError::Deferred(_)));
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_wallet_if_dirty_skips_fresh_uploading_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(
            metadata.id.clone(),
            jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
        );

        manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState { .. }),
                ..
            })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backup_wallets_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();
        globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

        manager.do_backup_wallets(&[metadata.clone()]).await.unwrap();

        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = WalletMetadata::preview_new();
        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id,
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::UploadedPendingConfirmation(
                    CloudBlobUploadedPendingConfirmationState {
                        revision_hash: "rev-1".into(),
                        uploaded_at: 10,
                        attempt_count: 0,
                        last_checked_at: None,
                    },
                ),
            })
            .unwrap();
        globals.cloud.dirty_wallet_on_next_backup_check(metadata.id.clone());

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_fails_when_auto_sync_upload_fails() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.fail_wallet_backup_upload("upload failed");

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::Failed(DeepVerificationFailure::Retry {
                message,
                detail,
                ..
            }) => {
                assert_eq!(
                    message,
                    "failed to auto-sync missing wallet backups: cloud storage error: upload failed: upload failed"
                );
                let detail = detail.expect("expected detail on retry failure");
                assert_eq!(detail.needs_sync.len(), 1);
                assert_eq!(detail.needs_sync[0].record_id, record_id);
            }
            other => panic!("expected retry failure, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_awaits_upload_confirmation_when_relist_still_misses_uploaded_wallet() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::AwaitingUploadConfirmation(report) => {
                let detail = report.detail.expect("expected verification detail");
                assert_eq!(detail.up_to_date.len(), 1);
                assert!(detail.needs_sync.is_empty());
                assert_eq!(detail.up_to_date[0].record_id, record_id);
            }
            other => panic!("expected awaiting upload confirmation, got {other:?}"),
        }

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manual_verification_clears_interactive_state_when_awaiting_upload_confirmation() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        manager.handle_start_verification(true).await;

        let state = manager.state();
        assert!(matches!(state.verification, VerificationState::Idle));
        assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);
        assert!(manager.pending_verification_completion().is_some());

        let detail = state.detail.expect("expected verification detail");
        assert_eq!(detail.up_to_date.len(), 1);
        assert!(detail.needs_sync.is_empty());
        assert_eq!(detail.up_to_date[0].record_id, record_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_finalizes_awaiting_deep_verify() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());
        assert_eq!(
            manager.state().pending_upload_verification,
            PendingUploadVerificationState::Idle
        );

        match manager.state().verification {
            VerificationState::Verified(report) => {
                assert_eq!(report.wallets_verified, 1);
                assert_eq!(report.wallets_failed, 0);
                assert_eq!(report.wallets_unsupported, 0);

                let detail = report.detail.expect("expected verification detail");
                assert_eq!(detail.up_to_date.len(), 1);
                assert!(detail.needs_sync.is_empty());
                assert_eq!(detail.up_to_date[0].record_id, record_id);
            }
            other => {
                panic!("expected verified result after pending upload verification, got {other:?}")
            }
        }

        assert!(!manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_refreshes_sync_health_to_all_uploaded() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);
        seed_verifiable_cloud_master_key(globals);

        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let metadata = xpub_only_wallet_metadata();
        let record_id = wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap();
        let prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &metadata,
            metadata.wallet_mode,
        )
        .await
        .unwrap();

        globals.cloud.set_wallet_backup(
            namespace_id.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, &prepared.revision_hash, 1).await,
        );
        globals.cloud.set_wallet_files(
            namespace_id.clone(),
            vec![wallet_filename_from_record_id(&record_id)],
        );
        persist_pending_master_key_confirmation(namespace_id.clone());
        manager
            .mark_blob_uploaded_pending_confirmation(
                &namespace_id,
                Some(metadata.id),
                record_id.clone(),
                prepared.revision_hash.clone(),
                jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
            )
            .unwrap();
        manager.replace_pending_verification_completion(PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            namespace_id,
            vec![
                PendingVerificationUpload::master_key_wrapper(),
                PendingVerificationUpload::new(record_id, prepared.revision_hash),
            ],
        ));

        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());
        assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::AllUploaded,);

        for _ in 0..20 {
            if manager.state().sync_health == CloudSyncHealth::AllUploaded {
                break;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(manager.state().sync_health, CloudSyncHealth::AllUploaded);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_survives_restart() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());

        let restarted_manager = init_manager();

        assert!(restarted_manager.pending_verification_completion().is_some());
        restarted_manager.sync_persisted_state();
        let has_more_pending = restarted_manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(restarted_manager.pending_verification_completion().is_none());
        match restarted_manager.state().verification {
            VerificationState::Verified(report) => {
                assert_eq!(report.wallets_verified, 1);
                assert_eq!(report.wallets_failed, 0);
                assert_eq!(report.wallets_unsupported, 0);

                let detail = report.detail.expect("expected verification detail");
                assert_eq!(detail.up_to_date.len(), 1);
                assert!(detail.needs_sync.is_empty());
                assert_eq!(detail.up_to_date[0].record_id, record_id);
            }
            other => {
                panic!("expected verified result after restart, got {other:?}")
            }
        }
        assert!(!restarted_manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_retries_until_expected_revision_is_readable() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap();
        let current_revision =
            crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
                &metadata,
                metadata.wallet_mode,
            )
            .await
            .unwrap()
            .revision_hash;

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        globals.cloud.set_wallet_backup_download_override(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1).await,
        );

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(has_more_pending);
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                    | PersistedCloudBlobState::Confirmed(_),
                ..
            })
        ));
        assert!(!matches!(
            manager.state().verification,
            VerificationState::Verified(_) | VerificationState::Failed(_)
        ));

        globals.cloud.set_wallet_backup_download_override(
            namespace,
            record_id,
            encrypted_wallet_backup_bytes(&metadata, &master_key, &current_revision, 1).await,
        );

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());
        match manager.state().verification {
            VerificationState::Verified(report) => {
                assert_eq!(report.wallets_verified, 1);
                assert_eq!(report.wallets_failed, 0);
                assert_eq!(report.wallets_unsupported, 0);
            }
            other => panic!("expected verified result after retry, got {other:?}"),
        }
        assert!(!manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_accepts_newer_revision_after_wallet_changes() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());

        manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
                ..
            })
        ));

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
        ));

        match manager.state().verification {
            VerificationState::Verified(report) => {
                assert_eq!(report.wallets_verified, 1);
                assert_eq!(report.wallets_failed, 0);
                assert_eq!(report.wallets_unsupported, 0);

                let detail = report.detail.expect("expected verification detail");
                assert_eq!(detail.up_to_date.len(), 1);
                assert!(detail.needs_sync.is_empty());
                assert_eq!(detail.up_to_date[0].record_id, record_id);
            }
            other => panic!("expected verified result after newer upload, got {other:?}"),
        }

        assert!(!manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_marks_invalid_wallet_json_failed() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());

        match manager.state().verification {
            VerificationState::Verified(report) => {
                assert_eq!(report.wallets_verified, 0);
                assert_eq!(report.wallets_failed, 1);
                assert_eq!(report.wallets_unsupported, 0);
            }
            other => {
                panic!("expected verified result after pending upload verification, got {other:?}")
            }
        }

        assert!(!manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_upload_verification_marks_terminal_live_upload_failures_failed() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id: namespace_id.clone(),
                wallet_id: Some(metadata.id),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::UploadedPendingConfirmation(
                    CloudBlobUploadedPendingConfirmationState {
                        revision_hash: "rev-1".into(),
                        uploaded_at: 10,
                        attempt_count: 0,
                        last_checked_at: None,
                    },
                ),
            })
            .unwrap();
        globals.cloud.set_wallet_backup(namespace_id, record_id.clone(), b"{".to_vec());

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(!has_more_pending);
        assert!(!manager.has_pending_cloud_upload_verification());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }),
                ..
            })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_pending_upload_without_remote_backup_remains_pending() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true).await;

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());

        CloudStorage::global_silent_client()
            .delete_wallet_backup(namespace_id.clone(), record_id.clone())
            .await
            .unwrap();
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                namespace_id,
                wallet_id: Some(metadata.id),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: Some("rev-1".into()),
                    retryable: false,
                    error: "terminal upload failure".into(),
                    issue: None,
                    failed_at: 10,
                }),
            })
            .unwrap();

        let has_more_pending = manager.verify_pending_uploads_once_for_test().await;

        assert!(has_more_pending);
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_preserves_unsupported_remote_wallet_backups() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let keychain = Keychain::global();
        let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::Verified(report) => {
                assert_eq!(report.wallets_verified, 0);
                assert_eq!(report.wallets_failed, 0);
                assert_eq!(report.wallets_unsupported, 1);
            }
            other => panic!("expected verified result, got {other:?}"),
        }

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(manager.pending_verification_completion().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_retries_when_remote_wallet_truth_is_unknown() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals
            .cloud
            .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
        globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::Failed(DeepVerificationFailure::Retry {
                message,
                detail,
                ..
            }) => {
                assert_eq!(message, "failed to refresh remote wallet truth for some wallets");

                let detail = detail.expect("expected verification detail");
                assert_eq!(detail.needs_sync.len(), 1);
                assert_eq!(detail.needs_sync[0].record_id, record_id);
                assert_eq!(
                    detail.needs_sync[0].sync_status,
                    CloudBackupWalletStatus::RemoteStateUnknown
                );
            }
            other => panic!("expected retry failure, got {other:?}"),
        }

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_succeeds_after_auto_sync_relist_confirms_wallet() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::Verified(report) => {
                let detail = report.detail.expect("expected verification detail");
                assert_eq!(detail.up_to_date.len(), 1);
                assert!(detail.needs_sync.is_empty());
                assert_eq!(
                    detail.up_to_date[0].record_id,
                    cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref())
                );
            }
            other => panic!("expected verified result, got {other:?}"),
        }

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
        ));
        assert!(!manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_verify_awaits_upload_confirmation_when_remote_revision_is_stale() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
        globals.cloud.set_wallet_backup_download_override(
            namespace,
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1).await,
        );

        let result = manager.deep_verify_cloud_backup(true).await;

        match result {
            DeepVerificationResult::AwaitingUploadConfirmation(report) => {
                let detail = report.detail.expect("expected verification detail");
                assert!(detail.up_to_date.is_empty());
                assert_eq!(detail.needs_sync.len(), 1);
                assert_eq!(detail.needs_sync[0].record_id, record_id);
            }
            other => panic!("expected awaiting upload confirmation, got {other:?}"),
        }

        assert!(manager.pending_verification_completion().is_some());
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
                ..
            })
        ));
        assert!(manager.has_pending_cloud_upload_verification());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn discard_pending_enable_clears_pending_session_and_local_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&master_key).unwrap();
        manager
            .replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                master_key,
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: vec![1, 2, 3],
                    provider_hint: None,
                },
            ))
            .await;

        manager.discard_pending_enable_cloud_backup();

        assert!(manager.take_pending_enable_session().await.is_none());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn discard_pending_enable_retry_upload_deletes_remote_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&master_key).unwrap();
        globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
        manager
            .replace_pending_enable_session(PendingEnableSession::retry_upload(
                master_key,
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: vec![1, 2, 3],
                    provider_hint: None,
                },
            ))
            .await;

        manager.discard_pending_enable_cloud_backup();

        wait_for_test_condition(
            Duration::from_secs(1),
            "remote master key backup should be deleted",
            || !globals.cloud.has_master_key_backup(&namespace),
        )
        .await;

        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        let existing_master_key = cove_cspp::master_key::MasterKey::generate();
        let existing_namespace = existing_master_key.namespace_id();
        let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key(
            &existing_master_key,
            &[7; 32],
            &[9; 32],
        )
        .unwrap();
        globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
        globals.cloud.set_master_key_backup(
            existing_namespace,
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];
        manager
            .replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                master_key,
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: expected_credential_id.clone(),
                    provider_hint: None,
                },
            ))
            .await;

        manager.do_enable_cloud_backup().await.unwrap();

        let pending = manager.take_pending_enable_session().await.unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_create_new_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];
        manager
            .replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                master_key,
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: expected_credential_id.clone(),
                    provider_hint: None,
                },
            ))
            .await;

        manager.do_enable_cloud_backup_create_new().await.unwrap();

        let pending = manager.take_pending_enable_session().await.unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_no_discovery_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];
        manager
            .replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                master_key,
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: expected_credential_id.clone(),
                    provider_hint: None,
                },
            ))
            .await;

        let create_count = globals.passkey.create_count();

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();

        assert_eq!(globals.passkey.create_count(), create_count);
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
        assert!(matches!(
            manager.state().prompt_intent,
            CloudBackupPromptIntent::ExistingBackupFound
        ));

        let pending = manager.take_pending_enable_session().await.unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn force_new_after_other_namespace_enter_detail_reuses_runtime_authorization() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);

        let existing_master_key = cove_cspp::master_key::MasterKey::generate();
        let existing_namespace = existing_master_key.namespace_id();
        let encrypted_existing_master = cove_cspp::master_key_crypto::encrypt_master_key(
            &existing_master_key,
            &[7; 32],
            &[9; 32],
        )
        .unwrap();
        globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
        globals.cloud.set_master_key_backup(
            existing_namespace,
            serde_json::to_vec(&encrypted_existing_master).unwrap(),
        );
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();

        assert_eq!(globals.passkey.create_count(), 1);
        assert_eq!(globals.passkey.authenticate_count(), 1);
        assert_eq!(globals.passkey.discover_count(), 0);
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
        assert!(matches!(
            manager.state().prompt_intent,
            CloudBackupPromptIntent::ExistingBackupFound
        ));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();

        assert_eq!(globals.passkey.create_count(), 1);
        assert_eq!(globals.passkey.authenticate_count(), 1);
        assert_eq!(globals.passkey.discover_count(), 0);

        manager.do_enable_cloud_backup_force_new().await.unwrap();

        assert!(manager.take_pending_enable_session().await.is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
        let create_count = globals.passkey.create_count();
        let authenticate_count = globals.passkey.authenticate_count();
        let discover_count = globals.passkey.discover_count();

        call!(manager.supervisor.start_enter_detail()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(globals.passkey.create_count(), create_count);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
        assert_eq!(globals.passkey.discover_count(), discover_count);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn detail_entry_after_restart_without_active_authorization_prompts_normally() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();
        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

        let restarted_manager = init_manager();
        restarted_manager.sync_persisted_state();
        restarted_manager.clear_pending_verification_completion();
        restarted_manager.set_pending_upload_verification(PendingUploadVerificationState::Idle);
        restarted_manager.set_verification(VerificationState::Idle);
        Database::global().cloud_blob_sync_states.delete_all().unwrap();
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let discover_count = globals.passkey.discover_count();

        call!(restarted_manager.supervisor.start_enter_detail()).await.unwrap();
        wait_for_discover_count(globals, discover_count + 1).await;

        assert_eq!(globals.passkey.discover_count(), discover_count + 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn enable_force_new_consumes_staged_session() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        CONNECTIVITY_MANAGER.set_connection_state(true);

        manager
            .replace_pending_enable_session(PendingEnableSession::awaiting_confirmation(
                cove_cspp::master_key::MasterKey::generate(),
                UnpersistedPrfKey {
                    prf_key: [7; 32],
                    prf_salt: [9; 32],
                    credential_id: vec![1, 2, 3],
                    provider_hint: None,
                },
            ))
            .await;

        manager.do_enable_cloud_backup_force_new().await.unwrap();

        assert!(manager.take_pending_enable_session().await.is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_enable_create_new_rolls_back_new_local_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        manager.do_enable_cloud_backup_create_new().await.unwrap();

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_enable_no_discovery_rolls_back_new_local_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);
        globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

        manager.do_enable_cloud_backup_no_discovery().await.unwrap();

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_passkey_restore_does_not_fall_back_to_local_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let local_master_key = cove_cspp::master_key::MasterKey::generate();
        let local_namespace_id = local_master_key.namespace_id();
        cove_cspp::Cspp::new(Keychain::global().clone())
            .save_master_key(&local_master_key)
            .unwrap();
        globals.cloud.set_wallet_files(local_namespace_id.clone(), vec!["wallet-test.json".into()]);

        let remote_master_key = cove_cspp::master_key::MasterKey::generate();
        let remote_namespace_id = remote_master_key.namespace_id();
        let remote_prf_key = [7u8; 32];
        let remote_prf_salt = [9u8; 32];
        let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key(
            &remote_master_key,
            &remote_prf_key,
            &remote_prf_salt,
        )
        .unwrap();
        globals.cloud.set_master_key_backup(
            remote_namespace_id.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        globals.cloud.set_wallet_files(remote_namespace_id, vec!["wallet-remote.json".into()]);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let operation = new_restore_operation_for_test(&manager).await;
        let error = manager.do_restore_from_cloud_backup(&operation).await.unwrap_err();

        assert!(matches!(error, CloudBackupError::PasskeyDiscoveryCancelled));
        assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_counts_unsupported_wallet_versions_as_failures() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

        let supported_wallet = xpub_only_wallet_metadata();
        let unsupported_wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
            .unwrap();
        Keychain::global()
            .save_wallet_xpub(
                &unsupported_wallet.id,
                sample_xpub(&unsupported_wallet).parse().unwrap(),
            )
            .unwrap();

        let supported_record_id =
            cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
        let unsupported_record_id =
            cove_cspp::backup_data::wallet_record_id(unsupported_wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            supported_record_id.clone(),
            encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            unsupported_record_id.clone(),
            encrypted_wallet_backup_bytes(
                &unsupported_wallet,
                &master_key,
                "unsupported-revision",
                2,
            )
            .await,
        );
        globals.cloud.set_wallet_files(
            namespace,
            vec![
                wallet_filename_from_record_id(&supported_record_id),
                wallet_filename_from_record_id(&unsupported_record_id),
            ],
        );

        let operation = new_restore_operation_for_test(&manager).await;
        manager.do_restore_from_cloud_backup(&operation).await.unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 1);
        assert_eq!(report.failed_wallet_errors.len(), 1);
        assert!(report.failed_wallet_errors[0].contains("unsupported wallet backup version 2"));
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(2));
        assert!(
            Database::global()
                .wallets()
                .get(&supported_wallet.id, supported_wallet.network, supported_wallet.wallet_mode,)
                .unwrap()
                .is_some()
        );
        assert!(
            Database::global()
                .wallets()
                .get(
                    &unsupported_wallet.id,
                    unsupported_wallet.network,
                    unsupported_wallet.wallet_mode,
                )
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_with_one_passkey_restores_wallets_from_all_matching_namespaces() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let prf_key = [7u8; 32];
        let first_master_key = cove_cspp::master_key::MasterKey::generate();
        let second_master_key = cove_cspp::master_key::MasterKey::generate();
        let first_namespace = first_master_key.namespace_id();
        let second_namespace = second_master_key.namespace_id();
        let first_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
                .unwrap();
        let second_encrypted = cove_cspp::master_key_crypto::encrypt_master_key(
            &second_master_key,
            &prf_key,
            &[8; 32],
        )
        .unwrap();

        globals.cloud.set_master_key_backup(
            first_namespace.clone(),
            serde_json::to_vec(&first_encrypted).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            second_namespace.clone(),
            serde_json::to_vec(&second_encrypted).unwrap(),
        );
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

        let first_wallet = xpub_only_wallet_metadata();
        let second_wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&first_wallet.id, sample_xpub(&first_wallet).parse().unwrap())
            .unwrap();
        Keychain::global()
            .save_wallet_xpub(&second_wallet.id, sample_xpub(&second_wallet).parse().unwrap())
            .unwrap();

        let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
        let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            first_namespace.clone(),
            first_record_id.clone(),
            encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, "first-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_backup(
            second_namespace.clone(),
            second_record_id.clone(),
            encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, "second-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            first_namespace,
            vec![wallet_filename_from_record_id(&first_record_id)],
        );
        globals.cloud.set_wallet_files(
            second_namespace,
            vec![wallet_filename_from_record_id(&second_record_id)],
        );

        let operation = new_restore_operation_for_test(&manager).await;
        manager.do_restore_from_cloud_backup(&operation).await.unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 2);
        assert_eq!(report.wallets_failed, 0);
        assert!(report.failed_wallet_errors.is_empty(), "{:?}", report.failed_wallet_errors);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_retries_platform_authorization_discover_failures() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let prf_key = [7u8; 32];
        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32])
                .unwrap();

        globals
            .cloud
            .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
        globals.passkey.push_discover_result(Err(platform_authorization_failed()));
        globals.passkey.push_discover_result(Err(platform_authorization_failed()));
        globals.passkey.push_discover_result(Err(platform_authorization_failed()));
        globals.passkey.push_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));

        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &master_key, "revision", 1).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let operation = new_restore_operation_for_test(&manager).await;
        manager.do_restore_from_cloud_backup(&operation).await.unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_does_not_persist_first_passkey_match_before_restore_work_succeeds() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let prf_key = [7u8; 32];
        let first_master_key = cove_cspp::master_key::MasterKey::generate();
        let second_master_key = cove_cspp::master_key::MasterKey::generate();
        let first_namespace = first_master_key.namespace_id();
        let second_namespace = second_master_key.namespace_id();
        let first_encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
                .unwrap();
        let second_encrypted = cove_cspp::master_key_crypto::encrypt_master_key(
            &second_master_key,
            &prf_key,
            &[8; 32],
        )
        .unwrap();

        globals.cloud.set_master_key_backup(
            first_namespace.clone(),
            serde_json::to_vec(&first_encrypted).unwrap(),
        );
        globals.cloud.set_master_key_backup(
            second_namespace.clone(),
            serde_json::to_vec(&second_encrypted).unwrap(),
        );
        globals.cloud.set_wallet_files(first_namespace, vec!["wallet-1.json".into()]);
        globals.cloud.set_wallet_files(second_namespace, vec!["wallet-2.json".into()]);
        globals.cloud.fail_list_wallet_files("list failed");
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: prf_key.to_vec(),
            credential_id: vec![1, 2, 3],
        }));
        globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

        let operation = new_restore_operation_for_test(&manager).await;
        let error = manager.do_restore_from_cloud_backup(&operation).await.unwrap_err();

        assert!(error.to_string().contains("list failed"), "{error}");
        assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_counts_listed_missing_wallet_backups_as_failures() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

        let supported_wallet = xpub_only_wallet_metadata();
        let missing_wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
            .unwrap();
        let supported_record_id =
            cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
        let missing_record_id =
            cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            supported_record_id.clone(),
            encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
                .await,
        );
        globals.cloud.set_wallet_files(
            namespace,
            vec![
                wallet_filename_from_record_id(&supported_record_id),
                wallet_filename_from_record_id(&missing_record_id),
            ],
        );

        let operation = new_restore_operation_for_test(&manager).await;
        manager.do_restore_from_cloud_backup(&operation).await.unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 1);
        assert!(
            report.failed_wallet_errors[0].contains("was listed but missing from cloud backup")
        );
        assert!(report.labels_failed_wallet_names.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_reports_label_warning_without_failing_wallet_restore() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

        let wallet = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        let entry = wallet_entry_with_labels(&wallet, Some("{"));
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let operation = new_restore_operation_for_test(&manager).await;
        manager.do_restore_from_cloud_backup(&operation).await.unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 0);
        assert_eq!(report.labels_failed_wallet_names, vec![wallet.name.clone()]);
        assert_eq!(report.labels_failed_errors.len(), 1);
        assert!(
            report.labels_failed_errors[0].contains("Failed to parse labels")
                || report.labels_failed_errors[0].contains("failed to parse")
        );
        assert!(
            Database::global()
                .wallets()
                .get(&wallet.id, wallet.network, wallet.wallet_mode)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_cloud_wallet_returns_label_warning_without_failing_restore() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        CloudBackupKeychain::global().save_namespace_id(&namespace).unwrap();
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
        manager
            .persist_cloud_backup_state(
                &PersistedCloudBackupState {
                    status: PersistedCloudBackupStatus::Enabled,
                    ..PersistedCloudBackupState::default()
                },
                "enable cloud backup for restore cloud wallet test",
            )
            .unwrap();

        let wallet = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        let entry = wallet_entry_with_labels(&wallet, Some("{"));
        globals.cloud.set_wallet_backup(
            namespace,
            record_id.clone(),
            encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
        );

        let outcome = manager.do_restore_cloud_wallet(&record_id).await.unwrap();

        let warning = outcome.labels_warning.expect("expected label warning");
        assert_eq!(warning.wallet_name, wallet.name);
        assert!(
            warning.error.contains("Failed to parse labels")
                || warning.error.contains("failed to parse")
        );
        assert!(
            Database::global()
                .wallets()
                .get(&wallet.id, wallet.network, wallet.wallet_mode)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_fails_when_all_wallet_backups_are_unsupported() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

        let wallet = xpub_only_wallet_metadata();
        Keychain::global()
            .save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap())
            .unwrap();

        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&wallet, &master_key, "unsupported-revision", 2).await,
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let operation = new_restore_operation_for_test(&manager).await;
        let error = manager.do_restore_from_cloud_backup(&operation).await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::Internal(message) if message == "all wallets failed to restore"
        ));

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 0);
        assert_eq!(report.wallets_failed, 1);
        assert!(report.failed_wallet_errors[0].contains("unsupported wallet backup version 2"));
        assert_eq!(
            Database::global().cloud_backup_state.get().unwrap().status,
            PersistedCloudBackupStatus::Disabled
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_fails_when_all_listed_wallet_backups_are_missing() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = init_manager();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let encrypted_master =
            cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32])
                .unwrap();
        globals.cloud.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted_master).unwrap(),
        );
        cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

        let missing_wallet = xpub_only_wallet_metadata();
        let missing_record_id =
            cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
        globals
            .cloud
            .set_wallet_files(namespace, vec![wallet_filename_from_record_id(&missing_record_id)]);

        let operation = new_restore_operation_for_test(&manager).await;
        let error = manager.do_restore_from_cloud_backup(&operation).await.unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::Internal(message) if message == "all wallets failed to restore"
        ));

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 0);
        assert_eq!(report.wallets_failed, 1);
        assert!(
            report.failed_wallet_errors[0].contains("was listed but missing from cloud backup")
        );
    }
}
