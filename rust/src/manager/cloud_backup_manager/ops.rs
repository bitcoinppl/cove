use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::{CSPP_NAMESPACE_ID_KEY, Keychain};
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::cloud_inventory::CloudWalletInventory;
use super::wallets::{
    DownloadedWalletBackup, NamespaceMatchOutcome, UnpersistedPrfKey, WalletBackupLookup,
    WalletBackupReader, WalletRestoreSession, all_local_wallets, create_new_prf_key,
    discover_or_create_prf_key_without_persisting, persist_enabled_cloud_backup_state,
    persist_enabled_cloud_backup_state_reset_verification, try_match_namespace_with_passkey,
    upload_all_wallets,
};

use super::{
    BlockingCloudStep, CloudBackupError, CloudBackupPasskeyChoiceFlow, CloudBackupRestoreProgress,
    CloudBackupRestoreReport, CloudBackupRestoreStage, CloudBackupStatus, CloudBackupWalletItem,
    CloudBackupWalletStatus, PendingEnableSession, RestoreOperation, RustCloudBackupManager,
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

impl RustCloudBackupManager {
    fn clear_enable_progress(&self, status: CloudBackupStatus) {
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_status(status);
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

    fn acquire_enable_passkey<F>(
        &self,
        cspp: &cove_cspp::Cspp<Keychain>,
        had_local_master_key: bool,
        cancelled_context: &str,
        failed_context: &str,
        acquire: F,
    ) -> Result<EnablePasskeyAcquisition, CloudBackupError>
    where
        F: FnOnce() -> Result<UnpersistedPrfKey, CloudBackupError>,
    {
        match acquire() {
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

    fn send_restore_progress(
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
    }

    fn finalize_uploaded_wallets(
        &self,
        cloud: &CloudStorage,
        namespace_id: &str,
        uploaded_wallets: Vec<super::wallets::PreparedWalletBackup>,
        state_mode: FinalizeUploadStateMode,
    ) -> Result<(), CloudBackupError> {
        let db = Database::global();
        let wallet_count = cloud
            .list_wallet_backups(namespace_id.to_owned())
            .map(|ids| ids.len() as u32)
            .unwrap_or(uploaded_wallets.len() as u32);
        match state_mode {
            FinalizeUploadStateMode::PreserveVerification => {
                persist_enabled_cloud_backup_state(&db, wallet_count)?;
            }
            FinalizeUploadStateMode::ResetVerification => {
                persist_enabled_cloud_backup_state_reset_verification(&db, wallet_count)?;
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

    pub(crate) fn do_sync_unsynced_wallets(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Sync)?;
        let namespace = self.current_namespace_id()?;
        info!("Sync: listing cloud wallet backups for namespace {namespace}");
        let cloud = CloudStorage::global();
        let wallet_record_ids = cloud.list_wallet_backups(namespace).map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::Sync,
                CloudBackupError::Cloud(error.to_string()),
            )
        })?;
        let remote_wallet_truth = self.load_remote_wallet_truth(&wallet_record_ids)?;
        let inventory =
            CloudWalletInventory::load_with_remote_truth(&wallet_record_ids, remote_wallet_truth)?;

        info!("Sync: found {} wallet(s) in cloud", inventory.cloud_wallet_count());
        let unsynced = inventory.upload_candidate_wallets();

        if unsynced.is_empty() {
            info!("Sync: all wallets already synced");
            return Ok(());
        }

        info!("Sync: {} wallet(s) need backup", unsynced.len());
        self.do_backup_wallets(&unsynced)
            .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::Sync, error))
    }

    pub(crate) fn do_fetch_cloud_only_wallets(
        &self,
    ) -> Result<Vec<CloudBackupWalletItem>, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::FetchCloudOnly)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global();
        let wallet_record_ids = cloud.list_wallet_backups(namespace.clone()).map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::FetchCloudOnly,
                CloudBackupError::Cloud(error.to_string()),
            )
        })?;

        let db = Database::global();
        let local_record_ids: std::collections::HashSet<_> = all_local_wallets(&db)?
            .iter()
            .map(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()))
            .collect();

        let orphan_ids: Vec<_> = wallet_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
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
        .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error))?;

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut items = Vec::new();

        for record_id in orphan_ids {
            let wallet = match reader.lookup(record_id) {
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
                    if Self::is_connectivity_related_issue(self.cloud_backup_issue(&error)) {
                        return Err(
                            self.blocking_cloud_error(BlockingCloudStep::FetchCloudOnly, error)
                        );
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

    pub(crate) fn do_restore_cloud_wallet(
        &self,
        record_id: &str,
    ) -> Result<super::wallets::WalletRestoreOutcome, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RestoreCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud(
                &namespace,
                CLOUD_ONLY_RESTORE_RECOVERY_MESSAGE,
            )
        })
        .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error))?;
        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );

        let db = Database::global();
        let existing_fingerprints: Vec<_> = all_local_wallets(&db)?
            .iter()
            .filter_map(|wallet| {
                wallet
                    .master_fingerprint
                    .as_ref()
                    .map(|fp| (**fp, wallet.network, wallet.wallet_mode))
            })
            .collect();
        let mut restore_session = WalletRestoreSession::new(existing_fingerprints);

        let outcome = restore_session.restore_record(&reader, record_id).map_err(|error| {
            self.blocking_cloud_error(BlockingCloudStep::RestoreCloudWallet, error)
        })?;
        info!("Restored cloud wallet {record_id}");
        Ok(outcome)
    }

    pub(crate) fn do_delete_cloud_wallet(&self, record_id: &str) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteCloudWallet)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global();

        cloud.delete_wallet_backup(namespace.clone(), record_id.to_string()).map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::DeleteCloudWallet,
                CloudBackupError::Cloud(error.to_string()),
            )
        })?;
        self.remove_blob_sync_states(std::iter::once(record_id.to_string())).map_err(|error| {
            self.blocking_cloud_error(BlockingCloudStep::DeleteCloudWallet, error)
        })?;

        let wallet_record_ids = cloud.list_wallet_backups(namespace).map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::DeleteCloudWallet,
                CloudBackupError::Cloud(error.to_string()),
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

    /// Re-upload all local wallets to cloud
    ///
    /// Reuses the master key from keychain (no passkey interaction needed)
    pub(crate) fn do_reupload_all_wallets(&self) -> Result<(), CloudBackupError> {
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
        .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::RecreateManifest, error))?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let cloud = CloudStorage::global();
        let uploaded_wallets =
            upload_all_wallets(cloud, &namespace, &critical_key, &Database::global()).map_err(
                |error| self.blocking_cloud_error(BlockingCloudStep::RecreateManifest, error),
            )?;

        self.finalize_uploaded_wallets(
            cloud,
            &namespace,
            uploaded_wallets,
            FinalizeUploadStateMode::PreserveVerification,
        )?;

        Ok(())
    }

    pub(crate) fn do_enable_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session() {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: retrying pending upload with existing passkey material");
            return self.enable_cloud_backup_with_passkey_material(
                Keychain::global(),
                master_key,
                passkey,
            );
        }

        let passkey = PasskeyAccess::global();
        if !passkey.is_prf_supported() {
            return Err(CloudBackupError::NotSupported(
                "PRF extension not supported on this device".into(),
            ));
        }

        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global();

        let has_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();

        if has_local_master_key {
            return self.do_enable_cloud_backup_create_new();
        }

        // no local master key — check iCloud for existing namespaces to recover
        let namespaces = cloud
            .list_namespaces()
            .map_err(|error| {
                self.blocking_cloud_error(
                    BlockingCloudStep::Enable,
                    CloudBackupError::Cloud(format!(
                        "could not check for existing cloud backups, please try again when iCloud is available: {error}"
                    )),
                )
            })?;

        if namespaces.is_empty() {
            return self.do_enable_cloud_backup_create_new();
        }

        info!("Enable: found {} existing namespace(s), attempting recovery", namespaces.len());

        match try_match_namespace_with_passkey(cloud, passkey, &namespaces)? {
            NamespaceMatchOutcome::Matched(matched) => {
                self.complete_recovery(keychain, cloud, &cspp, matched)
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

    /// Complete recovery from a matched cloud namespace
    fn complete_recovery(
        &self,
        keychain: &Keychain,
        cloud: &CloudStorage,
        cspp: &cove_cspp::Cspp<Keychain>,
        matched: super::wallets::NamespaceMatch,
    ) -> Result<(), CloudBackupError> {
        info!("Enable: recovered namespace {}", matched.namespace_id);

        cspp.save_master_key(&matched.master_key)
            .map_err_prefix("save recovered master key", CloudBackupError::Internal)?;

        let critical_key = Zeroizing::new(matched.master_key.critical_data_key());
        let uploaded_wallets =
            upload_all_wallets(cloud, &matched.namespace_id, &critical_key, &Database::global())
                .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::Enable, error))?;

        // persist credentials AFTER uploads succeed
        keychain
            .save_cspp_passkey_and_namespace(
                &matched.credential_id,
                matched.prf_salt,
                &matched.namespace_id,
            )
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        self.finalize_uploaded_wallets(
            cloud,
            &matched.namespace_id,
            uploaded_wallets,
            FinalizeUploadStateMode::ResetVerification,
        )?;
        self.clear_pending_enable_session();
        self.clear_enable_progress(CloudBackupStatus::Enabled);
        info!("Cloud backup enabled (recovered existing namespace)");
        Ok(())
    }

    /// Create a new cloud backup from scratch — no recovery attempt
    ///
    /// Called directly when `do_enable_cloud_backup` determines no recovery is needed,
    /// or via `do_enable_cloud_backup_force_new` when the user confirms creating a
    /// new backup after being warned about existing ones
    pub(crate) fn do_enable_cloud_backup_create_new(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session() {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: retrying pending upload with existing passkey material");
            return self.enable_cloud_backup_with_passkey_material(
                Keychain::global(),
                master_key,
                passkey,
            );
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
        let passkey = match self.acquire_enable_passkey(
            &cspp,
            had_local_master_key,
            "Enable cancelled before passkey setup finished",
            "Enable failed before passkey setup finished",
            || discover_or_create_prf_key_without_persisting(passkey_access),
        )? {
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
    }

    pub(crate) fn do_enable_cloud_backup_force_new(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let keychain = Keychain::global();

        if let Some(pending) = self.take_pending_enable_session() {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable: committing pending create-first cloud backup");
            return self.enable_cloud_backup_with_passkey_material(keychain, master_key, passkey);
        }

        self.do_enable_cloud_backup_create_new()
    }

    /// Same as `do_enable_cloud_backup_create_new` but skips passkey discovery,
    /// going straight to passkey registration
    pub(super) fn do_enable_cloud_backup_no_discovery(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session() {
            let (master_key, passkey) = pending.into_parts();
            info!("Enable (no discovery): retrying pending upload with existing passkey material");
            return self.enable_cloud_backup_with_passkey_material(
                Keychain::global(),
                master_key,
                passkey,
            );
        }

        let passkey_access = PasskeyAccess::global();
        let keychain = Keychain::global();
        let cloud = CloudStorage::global();

        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let had_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();
        let existing_namespaces = if had_local_master_key {
            Vec::new()
        } else {
            cloud.list_namespaces().map_err(|error| {
                self.blocking_cloud_error(
                    BlockingCloudStep::Enable,
                    CloudBackupError::Cloud(format!(
                        "could not check for existing cloud backups, please try again when iCloud is available: {error}"
                    )),
                )
            })?
        };

        info!("Enable (no discovery): getting master key");
        let master_key = cspp
            .get_or_create_master_key()
            .map_err_prefix("master key", CloudBackupError::Internal)?;

        let namespace_id = master_key.namespace_id();
        info!("Enable (no discovery): namespace_id={namespace_id}, creating passkey");
        let passkey = match self.acquire_enable_passkey(
            &cspp,
            had_local_master_key,
            "Enable (no discovery) cancelled before passkey setup finished",
            "Enable (no discovery) failed before passkey setup finished",
            || create_new_prf_key(passkey_access, "Creating new passkey"),
        )? {
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
            ));
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
    }

    pub(super) fn do_restore_from_cloud_backup(
        &self,
        operation: &RestoreOperation,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Restore)?;
        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_restore_report(None);
        self.set_status_for_restore_operation(operation, CloudBackupStatus::Restoring)?;
        self.send_restore_progress(operation, CloudBackupRestoreStage::Finding, 0, None)?;

        let cloud = CloudStorage::global();
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());

        // passkey matching first, local master key as fallback
        let passkey = PasskeyAccess::global();
        let (master_key, namespace_id) = match self.restore_via_passkey_matching(cloud, passkey) {
            Ok(matched) => {
                operation.run_result(|| {
                    cspp.save_master_key(&matched.master_key)
                        .map_err_prefix("save master key", CloudBackupError::Internal)?;
                    Ok(())
                })?;
                operation.run_result(|| {
                    keychain
                        .save_cspp_passkey_and_namespace(
                            &matched.credential_id,
                            matched.prf_salt,
                            &matched.namespace_id,
                        )
                        .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;
                    Ok(())
                })?;

                (matched.master_key, matched.namespace_id)
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                info!("Restore: passkey discovery cancelled");
                return Err(CloudBackupError::PasskeyDiscoveryCancelled);
            }
            Err(CloudBackupError::PasskeyMismatch) => {
                info!("Restore: passkey didn't match, trying local master key fallback");
                let (master_key, namespace_id) = try_restore_from_local_master_key(cloud, &cspp)
                    .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::Restore, error))?
                    .ok_or(CloudBackupError::PasskeyMismatch)?;
                operation.run_result(|| {
                    persist_namespace_id(keychain, &namespace_id)?;
                    Ok(())
                })?;
                (master_key, namespace_id)
            }
            Err(e) => return Err(e),
        };

        // download and restore wallets
        self.ensure_current_restore_operation(operation)?;
        let wallet_record_ids =
            cloud.list_wallet_backups(namespace_id.clone()).map_err(|error| {
                self.blocking_cloud_error(
                    BlockingCloudStep::Restore,
                    CloudBackupError::Cloud(error.to_string()),
                )
            })?;

        let reader = WalletBackupReader::new(
            cloud.clone(),
            namespace_id.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
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

        let downloaded_wallets =
            self.download_wallets_for_restore(operation, &reader, &wallet_record_ids, &mut report)?;
        let restore_total = downloaded_wallets.len() as u32;

        self.send_restore_progress(
            operation,
            CloudBackupRestoreStage::Restoring,
            0,
            Some(restore_total),
        )?;

        for (index, (record_id, wallet)) in downloaded_wallets.iter().enumerate() {
            match operation.run_result(|| restore_session.restore_downloaded(wallet)) {
                Ok(outcome) => {
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
            )?;
        }

        if report.wallets_restored == 0 && report.wallets_failed > 0 {
            self.set_restore_progress_for_restore_operation(operation, None)?;
            self.set_restore_report_for_restore_operation(operation, Some(report))?;
            return Err(CloudBackupError::Internal("all wallets failed to restore".into()));
        }

        let wallet_count = cloud
            .list_wallet_backups(namespace_id.clone())
            .map(|record_ids| record_ids.len() as u32)
            .unwrap_or(wallet_record_ids.len() as u32);
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
        )?;

        self.set_restore_progress_for_restore_operation(operation, None)?;
        self.set_restore_report_for_restore_operation(operation, Some(report))?;
        self.set_status_for_restore_operation(operation, CloudBackupStatus::Enabled)?;

        info!("Cloud backup restore complete");
        Ok(())
    }

    fn download_wallets_for_restore(
        &self,
        operation: &RestoreOperation,
        reader: &WalletBackupReader,
        wallet_record_ids: &[String],
        report: &mut CloudBackupRestoreReport,
    ) -> Result<Vec<(String, DownloadedWalletBackup)>, CloudBackupError> {
        let total = wallet_record_ids.len() as u32;

        self.send_restore_progress(
            operation,
            CloudBackupRestoreStage::Downloading,
            0,
            Some(total),
        )?;

        let mut downloaded_wallets = Vec::with_capacity(wallet_record_ids.len());

        for (index, record_id) in wallet_record_ids.iter().enumerate() {
            self.ensure_current_restore_operation(operation)?;
            match reader.lookup(record_id) {
                Ok(WalletBackupLookup::Found(wallet)) => {
                    downloaded_wallets.push((record_id.clone(), wallet));
                }
                Ok(WalletBackupLookup::NotFound) => {
                    let error =
                        format!("wallet {record_id} was listed but missing from cloud backup");
                    warn!("Failed to download wallet {record_id}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                    let error = format!(
                        "wallet {record_id} uses unsupported wallet backup version {version}"
                    );
                    warn!("Failed to download wallet {record_id}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error);
                }
                Err(error) => {
                    if Self::is_connectivity_related_issue(self.cloud_backup_issue(&error)) {
                        return Err(self.blocking_cloud_error(BlockingCloudStep::Restore, error));
                    }
                    warn!("Failed to download wallet {record_id}: {error}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(error.to_string());
                }
            }

            self.send_restore_progress(
                operation,
                CloudBackupRestoreStage::Downloading,
                (index + 1) as u32,
                Some(total),
            )?;
        }

        Ok(downloaded_wallets)
    }

    fn enable_cloud_backup_with_passkey_material(
        &self,
        keychain: &Keychain,
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<UnpersistedPrfKey>,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let namespace_id = master_key.namespace_id();
        let cloud = CloudStorage::global();
        self.replace_pending_enable_session(PendingEnableSession::retry_upload(
            cove_cspp::master_key::MasterKey::from_bytes(*master_key.as_bytes()),
            passkey.copy_for_retry(),
        ));

        let encrypted_master =
            master_key_crypto::encrypt_master_key(&master_key, &passkey.prf_key, &passkey.prf_salt)
                .map_err_str(CloudBackupError::Crypto)?;
        let master_json =
            serde_json::to_vec(&encrypted_master).map_err_str(CloudBackupError::Internal)?;

        info!("Enable: uploading master key");
        cloud.upload_master_key_backup(namespace_id.clone(), master_json).map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::Enable,
                CloudBackupError::Cloud(error.to_string()),
            )
        })?;

        info!("Enable: uploading wallets");
        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let db = Database::global();
        let uploaded_wallets = upload_all_wallets(cloud, &namespace_id, &critical_key, &db)
            .map_err(|error| self.blocking_cloud_error(BlockingCloudStep::Enable, error))?;

        info!("Enable: persisting cloud backup state");
        keychain
            .save_cspp_passkey_and_namespace(
                &passkey.credential_id,
                passkey.prf_salt,
                &namespace_id,
            )
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
            cloud,
            &namespace_id,
            uploaded_wallets,
            FinalizeUploadStateMode::ResetVerification,
        )?;

        self.clear_pending_enable_session();
        self.clear_enable_progress(CloudBackupStatus::Enabled);
        info!("Cloud backup enabled successfully");
        Ok(())
    }

    /// Restore via passkey-based namespace matching (fresh device path)
    ///
    /// Tries the selected passkey across all downloaded namespaces. If it
    /// doesn't match any of them, returns `PasskeyMismatch` so the caller can
    /// try local master key fallback or prompt the user to try a different
    /// passkey
    fn restore_via_passkey_matching(
        &self,
        cloud: &CloudStorage,
        passkey: &PasskeyAccess,
    ) -> Result<super::wallets::NamespaceMatch, CloudBackupError> {
        let namespaces = cloud.list_namespaces().map_err(|error| {
            self.blocking_cloud_error(
                BlockingCloudStep::Restore,
                CloudBackupError::Cloud(error.to_string()),
            )
        })?;
        if namespaces.is_empty() {
            return Err(CloudBackupError::Internal("no cloud backup namespaces found".into()));
        }

        info!("Restore: authenticating with passkey across {} namespace(s)", namespaces.len());

        match try_match_namespace_with_passkey(cloud, passkey, &namespaces)? {
            NamespaceMatchOutcome::Matched(m) => {
                info!("Restore: matched namespace {}", m.namespace_id);
                Ok(m)
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
}

fn persist_namespace_id<S>(store: &S, namespace_id: &str) -> Result<(), CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    store
        .save(CSPP_NAMESPACE_ID_KEY.into(), namespace_id.to_owned())
        .map_err_prefix("save namespace_id", CloudBackupError::Internal)
}

fn try_restore_from_local_master_key<S>(
    cloud: &CloudStorage,
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

    let has_wallets = cloud
        .list_wallet_backups(namespace_id.clone())
        .map(|ids| !ids.is_empty())
        .map_err(|error| CloudBackupError::Cloud(error.to_string()))?;

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

#[cfg(test)]
fn restore_from_local_master_key_fallback<S>(
    cloud: &CloudStorage,
    store: &S,
    cspp: &cove_cspp::Cspp<S>,
) -> Result<(cove_cspp::master_key::MasterKey, String), CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    let (master_key, namespace_id) =
        try_restore_from_local_master_key(cloud, cspp)?.ok_or(CloudBackupError::PasskeyMismatch)?;
    persist_namespace_id(store, &namespace_id)?;
    Ok((master_key, namespace_id))
}

pub(super) fn load_master_key_for_cloud_action<S, F>(
    cspp: &cove_cspp::Cspp<S>,
    recover_missing: F,
) -> Result<cove_cspp::master_key::MasterKey, CloudBackupError>
where
    S: cove_cspp::CsppStore,
    F: FnOnce() -> Result<cove_cspp::master_key::MasterKey, CloudBackupError>,
{
    match cspp
        .load_master_key_from_store()
        .map_err_prefix("load local master key", CloudBackupError::Internal)?
    {
        Some(master_key) => Ok(master_key),
        None => recover_missing(),
    }
}

#[cfg(test)]
#[path = "ops/test_support.rs"]
mod test_support;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use cove_cspp::CsppStore;
    use cove_cspp::backup_data::{
        WalletEntry, WalletMode as CloudWalletMode, WalletSecret, wallet_filename_from_record_id,
        wallet_record_id,
    };
    use cove_device::cloud_storage::{CloudStorage, CloudStorageAccess};
    use cove_device::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY, Keychain,
    };
    use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyAccess, PasskeyError};

    use super::test_support::*;
    use super::*;
    use crate::database::Database;
    use crate::database::cloud_backup::{
        CloudBlobDirtyState, CloudBlobFailedState, CloudBlobUploadedPendingConfirmationState,
        CloudBlobUploadingState, CloudUploadKind, PersistedCloudBackupState,
        PersistedCloudBackupStatus, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    };
    use crate::label_manager::LabelManager;
    use crate::manager::cloud_backup_manager::{
        CLOUD_BACKUP_MANAGER, CloudBackupDetailResult, CloudConnectivityHint,
        DeepVerificationResult, VerificationFailureKind, VerificationState,
    };
    use crate::manager::wallet_manager::RustWalletManager;
    use crate::network::Network;
    use crate::wallet::{
        Wallet,
        metadata::{WalletMetadata, WalletMode, WalletType},
    };
    use bip39::Mnemonic;

    mod cove_tokio {
        pub(super) fn init() {
            super::init_test_runtime();
        }
    }

    #[test]
    fn passkey_match_treats_missing_credential_as_no_match() {
        let _guard = test_lock().lock();
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

        let outcome = try_match_namespace_with_passkey(
            CloudStorage::global(),
            PasskeyAccess::global(),
            &[namespace],
        )
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
    }

    #[test]
    fn passkey_match_treats_user_cancel_as_user_declined() {
        let _guard = test_lock().lock();
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

        let outcome = try_match_namespace_with_passkey(
            CloudStorage::global(),
            PasskeyAccess::global(),
            &[namespace],
        )
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::UserDeclined));
    }

    #[test]
    fn passkey_match_mixed_supported_and_unsupported_versions_returns_no_match() {
        let _guard = test_lock().lock();
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

        let outcome = try_match_namespace_with_passkey(
            CloudStorage::global(),
            PasskeyAccess::global(),
            &[supported_namespace, unsupported_namespace],
        )
        .unwrap();

        assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
    }

    #[test]
    fn mock_master_key_upload_persists_uploaded_bytes() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let namespace = "namespace-1".to_string();
        let uploaded = vec![1, 2, 3, 4];
        globals.cloud.upload_master_key_backup(namespace.clone(), uploaded.clone()).unwrap();

        assert_eq!(globals.cloud.download_master_key_backup(namespace).unwrap(), uploaded);
    }

    #[test]
    fn persist_xpub_wallets_saves_each_wallet_in_its_own_scope() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let first_wallet = xpub_only_wallet_metadata();
        let mut second_wallet = xpub_only_wallet_metadata();
        second_wallet.network = Network::Testnet;
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

    #[test]
    fn wrapper_repair_discovery_propagates_unsupported_provider() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

        let error = match discover_or_create_prf_key_without_persisting(PasskeyAccess::global()) {
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
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 3);

        let metadata = xpub_only_wallet_metadata();
        let xpub = sample_xpub(&metadata);
        Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();

        manager.do_backup_wallets(&[metadata]).unwrap();

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
        let manager = RustCloudBackupManager::init();
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
        let manager = RustCloudBackupManager::init();
        globals.reset();

        let namespace = "test-namespace".to_string();
        Keychain::global().save(CSPP_NAMESPACE_ID_KEY.into(), namespace).unwrap();
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

        manager.clear_wallet_upload_debouncers_for_test();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_downloaded_wallet_does_not_reupload_wallet_or_mutate_backup_counts() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
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
        let manager = RustCloudBackupManager::init();
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

        let exported = LabelManager::new(metadata.id.clone()).export_blocking().unwrap();
        assert!(exported.contains("\"label\":\"last txn received\""));
    }

    #[test]
    fn cloud_action_uses_existing_master_key_without_recovery() {
        let store = Arc::new(MockStore::default());
        let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
        let expected = cove_cspp::master_key::MasterKey::generate();
        cspp.save_master_key(&expected).unwrap();

        let recovered = load_master_key_for_cloud_action(&cspp, || {
            Err(CloudBackupError::RecoveryRequired("unexpected".into()))
        })
        .unwrap();

        assert_eq!(recovered.as_bytes(), expected.as_bytes());
    }

    #[test]
    fn cloud_action_does_not_create_master_key_when_missing() {
        let store = Arc::new(MockStore::default());
        let cspp = cove_cspp::Cspp::new(MockStoreHandle(store.clone()));

        let result = load_master_key_for_cloud_action(&cspp, || {
            Err(CloudBackupError::RecoveryRequired("needs recovery".into()))
        });

        assert!(matches!(
            result,
            Err(CloudBackupError::RecoveryRequired(message)) if message == "needs recovery"
        ));
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(*store.save_count.lock(), 0);
    }

    #[test]
    fn local_master_key_fallback_persists_namespace_id() {
        let _guard = test_lock().lock();
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
            CloudStorage::global(),
            &store_handle,
            &cspp,
        )
        .unwrap();

        assert_eq!(restored.as_bytes(), expected.as_bytes());
        assert_eq!(restored_namespace, namespace_id.clone());
        assert_eq!(
            store_handle.get(CSPP_NAMESPACE_ID_KEY.into()).as_deref(),
            Some(namespace_id.as_str())
        );
    }

    #[test]
    fn restore_from_local_master_key_propagates_store_read_errors() {
        let _guard = test_lock().lock();
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

        let error = match try_restore_from_local_master_key(CloudStorage::global(), &cspp) {
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
    fn blocking_cloud_error_rewrites_unavailable_messages_to_offline() {
        let manager = RustCloudBackupManager::init();

        let error = manager.blocking_cloud_error(
            BlockingCloudStep::Enable,
            CloudBackupError::Cloud("iCloud Drive is not available".into()),
        );

        assert!(matches!(error, CloudBackupError::Offline(_)));
    }

    #[test]
    fn failed_create_new_enable_does_not_persist_passkey_metadata() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.cloud.fail_master_key_upload("boom");
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        let manager = RustCloudBackupManager::init();
        let error = manager.do_enable_cloud_backup_create_new().unwrap_err();
        assert!(
            matches!(error, CloudBackupError::Cloud(message) if message.contains("upload failed: boom"))
        );

        let keychain = Keychain::global();
        assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    }

    #[test]
    fn failed_no_discovery_enable_does_not_persist_passkey_metadata() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
        globals
            .passkey
            .set_authenticate_result(Err(PasskeyError::AuthenticationFailed("boom".into())));

        let manager = RustCloudBackupManager::init();
        let error = manager.do_enable_cloud_backup_no_discovery().unwrap_err();

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

    #[test]
    fn finalize_passkey_repair_keeps_existing_count_when_wallet_refresh_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
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

        manager.finalize_passkey_repair().unwrap();

        let state = Database::global().cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Enabled);
        assert_eq!(state.wallet_count, Some(7));
        assert_eq!(manager.state().status, CloudBackupStatus::Enabled);
    }

    #[test]
    fn reupload_all_wallets_does_not_create_master_key_for_existing_namespace() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        Keychain::global().save(CSPP_NAMESPACE_ID_KEY.into(), "existing-namespace".into()).unwrap();

        let manager = RustCloudBackupManager::init();
        let error = manager.do_reupload_all_wallets().unwrap_err();

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
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
        globals
            .cloud
            .set_wallet_files(namespace, vec![wallet_filename_from_record_id("cloud-only-record")]);

        manager.do_reupload_all_wallets().unwrap();

        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(2));
        clear_wallet_upload_runtime_for_test_async(&manager).await;
    }

    #[test]
    fn fetch_cloud_only_wallets_surfaces_unsupported_versions() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let keychain = Keychain::global();
        let namespace = keychain.get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let wallets = manager.do_fetch_cloud_only_wallets().unwrap();

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

    #[test]
    fn backup_wallets_does_not_create_master_key_or_upload_when_missing() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let namespace = "existing-namespace";
        Keychain::global().save(CSPP_NAMESPACE_ID_KEY.into(), namespace.into()).unwrap();

        let manager = RustCloudBackupManager::init();
        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = crate::wallet::metadata::WalletType::WatchOnly;

        let error = manager.do_backup_wallets(&[metadata]).unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::RecoveryRequired(message)
                if message == "Cloud backup needs verification before wallets can be uploaded"
        ));

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    }

    #[test]
    fn upload_wallet_if_dirty_does_not_create_master_key_for_existing_namespace() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();

        let namespace = "existing-namespace";
        Keychain::global().save(CSPP_NAMESPACE_ID_KEY.into(), namespace.into()).unwrap();

        let manager = RustCloudBackupManager::init();
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
                kind: CloudUploadKind::BackupBlob,
                namespace_id: namespace.into(),
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();

        let error = manager.do_upload_wallet_if_dirty(&metadata.id).unwrap_err();

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
        let manager = CLOUD_BACKUP_MANAGER.clone();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_dirty_blob_state(metadata.id.clone());
        globals.cloud.fail_next_wallet_backup_upload("offline");

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
        let manager = CLOUD_BACKUP_MANAGER.clone();
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
            super::super::LIVE_UPLOAD_DEBOUNCE + Duration::from_secs(1),
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
        let manager = CLOUD_BACKUP_MANAGER.clone();
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
    fn update_connectivity_hint_preserves_sync_error_when_failed_wallet_uploads_exist() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let wallet_id = xpub_only_wallet_metadata().id;
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
                namespace_id: Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap(),
                wallet_id: Some(wallet_id),
                record_id,
                state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    revision_hash: None,
                    error: "upload failed".into(),
                    retryable: false,
                    failed_at: 1,
                }),
            })
            .unwrap();
        manager.set_sync_error(Some("upload failed".into()));

        manager.update_connectivity_hint(CloudConnectivityHint::Online);

        assert_eq!(manager.state().sync_error.as_deref(), Some("upload failed"));
    }

    #[test]
    fn update_connectivity_hint_clears_sync_error_when_failed_wallet_uploads_are_gone() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);
        manager.set_sync_error(Some("upload failed".into()));

        manager.update_connectivity_hint(CloudConnectivityHint::Online);

        assert!(manager.state().sync_error.is_none());
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
        let manager = CLOUD_BACKUP_MANAGER.clone();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_failed_blob_state(metadata.id.clone(), false);
        globals.cloud.fail_wallet_backup_upload_quota_exceeded();

        manager.resume_pending_cloud_upload_verification();

        assert_test_condition_stays_true(
            Duration::from_millis(250),
            "startup resume should not retry non-retryable failed uploads",
            || globals.cloud.wallet_backup_upload_attempt_count() == 0,
        )
        .await;

        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 0);

        clear_wallet_upload_runtime_for_test_async(&manager).await;
        globals.cloud.clear_wallet_backup_upload_failure();
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
        let manager = CLOUD_BACKUP_MANAGER.clone();
        clear_wallet_upload_runtime_for_test_async(&manager).await;
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        persist_uploading_blob_state(metadata.id, 1);

        manager.resume_pending_cloud_upload_verification();

        wait_for_test_condition(
            Duration::from_secs(1),
            "startup resume should retry interrupted uploads",
            || {
                matches!(
                    Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                    Some(PersistedCloudBlobSyncState {
                        state: PersistedCloudBlobState::Dirty(_)
                            | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                            | PersistedCloudBlobState::Confirmed(_),
                        ..
                    })
                )
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
        let manager = CLOUD_BACKUP_MANAGER.clone();
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

        manager.clear_wallet_upload_debouncers_for_test();
    }

    #[test]
    fn upload_wallet_if_dirty_removes_deleted_wallet_sync_state() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = WalletMetadata::preview_new();
        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
                namespace_id: namespace.clone(),
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();

        manager.do_upload_wallet_if_dirty(&metadata.id).unwrap();

        assert!(Database::global().cloud_blob_sync_states.get(&record_id).unwrap().is_none());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()), Some(namespace));
    }

    #[test]
    fn sync_and_integrity_skip_pending_upload_candidates() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();

        let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
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

        manager.do_sync_unsynced_wallets().unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(1));

        let warning = manager.verify_backup_integrity_impl().expect("expected passkey warning");

        assert!(!warning.contains("some wallets are not backed up"));
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count, Some(1));
    }

    #[test]
    fn integrity_does_not_retry_sync_after_auto_backup_failure() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        Keychain::global()
            .save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata])
            .unwrap();
        globals.cloud.fail_wallet_backup_upload("offline");

        let warning = manager.verify_backup_integrity_impl().expect("expected integrity warning");

        assert!(warning.contains("some wallets are not backed up"));
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    }

    #[test]
    fn integrity_warns_when_wallet_list_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        Keychain::global()
            .save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.fail_list_wallet_files("offline");

        let warning = manager.verify_backup_integrity_impl().expect("expected integrity warning");

        assert!(warning.contains("wallet backups could not be listed"));
        globals.cloud.clear_list_wallet_files_failure();
    }

    #[test]
    fn integrity_preserves_unsupported_remote_wallet_backups() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);

        let keychain = Keychain::global();
        let namespace = keychain.get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        keychain.save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace).unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let warning = manager.verify_backup_integrity_impl();

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
        let manager = RustCloudBackupManager::init();
        reset_cloud_backup_test_state(&manager, globals);

        let metadata = xpub_only_wallet_metadata();

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        let keychain = Keychain::global();
        keychain.save(CSPP_NAMESPACE_ID_KEY.into(), namespace.clone()).unwrap();
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
            encrypted_wallet_backup_bytes(&metadata, &master_key, "rev-1", 1),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let cspp = cove_cspp::Cspp::new(keychain.clone());
        cspp.delete_master_key();
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

        let Some(CloudBackupDetailResult::Success(detail)) = manager.refresh_cloud_backup_detail()
        else {
            panic!("expected cloud backup detail");
        };

        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::RemoteStateUnknown);
    }

    #[test]
    fn sync_skips_wallets_with_unknown_remote_truth() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 1);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        globals
            .cloud
            .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
        globals.cloud.set_wallet_backup(namespace, record_id, b"{".to_vec());

        manager.do_sync_unsynced_wallets().unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_refreshes_detail_after_auto_backup_success() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        Keychain::global()
            .save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let warning = manager.verify_backup_integrity_impl();

        assert!(warning.is_none());
        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert_eq!(detail.up_to_date.len(), 1);
        assert!(detail.needs_sync.is_empty());
        assert_eq!(detail.up_to_date[0].record_id, record_id);
        assert_eq!(detail.up_to_date[0].sync_status, CloudBackupWalletStatus::Confirmed);
        manager.clear_wallet_upload_debouncers_for_test();
    }

    #[test]
    fn integrity_does_not_retry_sync_after_auto_backup_success_when_listing_stays_empty() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata]);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        Keychain::global()
            .save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();

        let warning = manager.verify_backup_integrity_impl();

        assert!(warning.is_none());
        assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    }

    #[test]
    fn integrity_refreshes_detail_after_auto_backup_failure() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata]);

        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        Keychain::global()
            .save_cspp_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
            .unwrap();
        globals.cloud.fail_wallet_backup_upload("offline");

        let warning = manager.verify_backup_integrity_impl().expect("expected integrity warning");

        assert!(warning.contains("some wallets are not backed up"));
        let detail = manager.state().detail.expect("expected cloud backup detail");
        assert_eq!(detail.needs_sync.len(), 1);
        assert_eq!(detail.needs_sync[0].record_id, record_id);
        assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::Dirty);
    }

    #[test]
    fn upload_wallet_if_dirty_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();

        let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
                namespace_id,
                wallet_id: Some(metadata.id.clone()),
                record_id: record_id.clone(),
                state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            })
            .unwrap();
        globals.cloud.dirty_wallet_on_next_upload(metadata.id.clone());

        manager.do_upload_wallet_if_dirty(&metadata.id).unwrap();

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
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(metadata.id.clone(), 1);

        manager.do_upload_wallet_if_dirty(&metadata.id).unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
                ..
            })
        ));
        manager.clear_wallet_upload_debouncers_for_test();
    }

    #[test]
    fn upload_wallet_if_dirty_recovers_stale_uploading_state_while_offline() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(metadata.id.clone(), 1);
        manager.update_connectivity_hint(CloudConnectivityHint::Offline);

        let error = manager.do_upload_wallet_if_dirty(&metadata.id).unwrap_err();

        assert!(matches!(error, CloudBackupError::Deferred(_)));
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[test]
    fn upload_wallet_if_dirty_skips_fresh_uploading_state() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        persist_xpub_wallets(vec![metadata.clone()]);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_uploading_blob_state(
            metadata.id.clone(),
            jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
        );

        manager.do_upload_wallet_if_dirty(&metadata.id).unwrap();

        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState { .. }),
                ..
            })
        ));
    }

    #[test]
    fn backup_wallets_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        Database::global()
            .wallets()
            .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
            .unwrap();
        globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

        manager.do_backup_wallets(&[metadata.clone()]).unwrap();

        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[test]
    fn pending_upload_verification_preserves_newer_dirty_state() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = WalletMetadata::preview_new();
        let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
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

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

        assert!(!has_more_pending);
        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ));
    }

    #[test]
    fn deep_verify_fails_when_auto_sync_upload_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.fail_wallet_backup_upload("upload failed");

        let result = manager.deep_verify_cloud_backup(true);

        match result {
            DeepVerificationResult::Failed(failure) => {
                assert_eq!(failure.kind, VerificationFailureKind::Retry);
                assert_eq!(
                    failure.message,
                    "failed to auto-sync missing wallet backups: cloud storage error: upload failed: upload failed"
                );
                let detail = failure.detail.expect("expected detail on retry failure");
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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true);

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
    async fn pending_upload_verification_finalizes_awaiting_deep_verify() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true);

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

        assert!(!has_more_pending);
        assert!(manager.pending_verification_completion().is_none());

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
    async fn pending_upload_verification_survives_restart() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true);

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());

        let restarted_manager = RustCloudBackupManager::init();

        assert!(restarted_manager.pending_verification_completion().is_some());
        restarted_manager.sync_persisted_state();
        let has_more_pending = restarted_manager.verify_pending_uploads_once_for_test();

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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
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
            .unwrap()
            .revision_hash;

        let result = manager.deep_verify_cloud_backup(true);

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        globals.cloud.set_wallet_backup_download_override(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1),
        );

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

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
            encrypted_wallet_backup_bytes(&metadata, &master_key, &current_revision, 1),
        );

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

        let result = manager.deep_verify_cloud_backup(true);

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        assert!(manager.pending_verification_completion().is_some());
        assert!(manager.has_pending_cloud_upload_verification());

        manager.do_upload_wallet_if_dirty(&metadata.id).unwrap();

        assert!(matches!(
            Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
            Some(PersistedCloudBlobSyncState {
                state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
                ..
            })
        ));

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

        let result = manager.deep_verify_cloud_backup(true);

        assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
        globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

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
        let manager = RustCloudBackupManager::init();
        configure_enabled_cloud_backup(&manager, globals, 0);

        let metadata = xpub_only_wallet_metadata();
        let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        persist_xpub_wallets(vec![metadata.clone()]);
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState {
                kind: CloudUploadKind::BackupBlob,
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

        let has_more_pending = manager.verify_pending_uploads_once_for_test();

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

    #[test]
    fn deep_verify_preserves_unsupported_remote_wallet_backups() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let keychain = Keychain::global();
        let namespace = keychain.get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let master_key =
            cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let result = manager.deep_verify_cloud_backup(true);

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

    #[test]
    fn deep_verify_retries_when_remote_wallet_truth_is_unknown() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals
            .cloud
            .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
        globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

        let result = manager.deep_verify_cloud_backup(true);

        match result {
            DeepVerificationResult::Failed(failure) => {
                assert_eq!(failure.kind, VerificationFailureKind::Retry);
                assert_eq!(
                    failure.message,
                    "failed to refresh remote wallet truth for some wallets"
                );

                let detail = failure.detail.expect("expected verification detail");
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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

        let result = manager.deep_verify_cloud_backup(true);

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
        let manager = RustCloudBackupManager::init();
        let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
        let namespace = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap();
        globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
        globals.cloud.set_wallet_backup_download_override(
            namespace,
            record_id.clone(),
            encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1),
        );

        let result = manager.deep_verify_cloud_backup(true);

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

    #[test]
    fn discard_pending_enable_clears_pending_session_and_local_master_key() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&master_key).unwrap();
        manager.replace_pending_enable_session(PendingEnableSession::new(
            master_key,
            UnpersistedPrfKey { prf_key: [7; 32], prf_salt: [9; 32], credential_id: vec![1, 2, 3] },
        ));

        manager.discard_pending_enable_cloud_backup();

        assert!(manager.take_pending_enable_session().is_none());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
    }

    #[test]
    fn enable_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        manager.update_connectivity_hint(CloudConnectivityHint::Online);
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
        manager.replace_pending_enable_session(PendingEnableSession::new(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
            },
        ));

        manager.do_enable_cloud_backup().unwrap();

        let pending = manager.take_pending_enable_session().unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[test]
    fn enable_create_new_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        manager.update_connectivity_hint(CloudConnectivityHint::Online);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];
        manager.replace_pending_enable_session(PendingEnableSession::new(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
            },
        ));

        manager.do_enable_cloud_backup_create_new().unwrap();

        let pending = manager.take_pending_enable_session().unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[test]
    fn enable_no_discovery_preserves_awaiting_force_new_session() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        manager.update_connectivity_hint(CloudConnectivityHint::Online);
        globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let expected_namespace = master_key.namespace_id();
        let expected_credential_id = vec![1, 2, 3];
        manager.replace_pending_enable_session(PendingEnableSession::new(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
            },
        ));

        manager.do_enable_cloud_backup_no_discovery().unwrap();

        let pending = manager.take_pending_enable_session().unwrap();
        let (pending_master_key, pending_passkey) = pending.into_parts();
        assert_eq!(pending_master_key.namespace_id(), expected_namespace);
        assert_eq!(pending_passkey.credential_id, expected_credential_id);
    }

    #[test]
    fn enable_force_new_consumes_staged_session() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        manager.update_connectivity_hint(CloudConnectivityHint::Online);

        manager.replace_pending_enable_session(PendingEnableSession::new(
            cove_cspp::master_key::MasterKey::generate(),
            UnpersistedPrfKey { prf_key: [7; 32], prf_salt: [9; 32], credential_id: vec![1, 2, 3] },
        ));

        manager.do_enable_cloud_backup_force_new().unwrap();

        assert!(manager.take_pending_enable_session().is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    }

    #[test]
    fn cancelled_enable_create_new_rolls_back_new_local_master_key() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        manager.do_enable_cloud_backup_create_new().unwrap();

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
    }

    #[test]
    fn cancelled_enable_no_discovery_rolls_back_new_local_master_key() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);
        globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

        manager.do_enable_cloud_backup_no_discovery().unwrap();

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        assert!(cspp.load_master_key_from_store().unwrap().is_none());
        assert_eq!(manager.current_status(), CloudBackupStatus::Disabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_passkey_restore_does_not_fall_back_to_local_master_key() {
        let _guard = test_lock().lock();
        cove_tokio::init();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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

        let operation = new_restore_operation_for_test(&manager);
        let error = manager.do_restore_from_cloud_backup(&operation).unwrap_err();

        assert!(matches!(error, CloudBackupError::PasskeyDiscoveryCancelled));
        assert_eq!(Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()), None);
    }

    #[test]
    fn restore_counts_unsupported_wallet_versions_as_failures() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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
            encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1),
        );
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            unsupported_record_id.clone(),
            encrypted_wallet_backup_bytes(
                &unsupported_wallet,
                &master_key,
                "unsupported-revision",
                2,
            ),
        );
        globals.cloud.set_wallet_files(
            namespace,
            vec![
                wallet_filename_from_record_id(&supported_record_id),
                wallet_filename_from_record_id(&unsupported_record_id),
            ],
        );

        let operation = new_restore_operation_for_test(&manager);
        manager.do_restore_from_cloud_backup(&operation).unwrap();

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

    #[test]
    fn restore_counts_listed_missing_wallet_backups_as_failures() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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
            encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1),
        );
        globals.cloud.set_wallet_files(
            namespace,
            vec![
                wallet_filename_from_record_id(&supported_record_id),
                wallet_filename_from_record_id(&missing_record_id),
            ],
        );

        let operation = new_restore_operation_for_test(&manager);
        manager.do_restore_from_cloud_backup(&operation).unwrap();

        let report = manager.state().restore_report.expect("expected restore report");
        assert_eq!(report.wallets_restored, 1);
        assert_eq!(report.wallets_failed, 1);
        assert!(
            report.failed_wallet_errors[0].contains("was listed but missing from cloud backup")
        );
        assert!(report.labels_failed_wallet_names.is_empty());
    }

    #[test]
    fn restore_reports_label_warning_without_failing_wallet_restore() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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

        let operation = new_restore_operation_for_test(&manager);
        manager.do_restore_from_cloud_backup(&operation).unwrap();

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

    #[test]
    fn restore_cloud_wallet_returns_label_warning_without_failing_restore() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

        reset_cloud_backup_test_state(&manager, globals);

        let master_key = cove_cspp::master_key::MasterKey::generate();
        let namespace = master_key.namespace_id();
        Keychain::global().save(CSPP_NAMESPACE_ID_KEY.into(), namespace.clone()).unwrap();
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

        let outcome = manager.do_restore_cloud_wallet(&record_id).unwrap();

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

    #[test]
    fn restore_fails_when_all_wallet_backups_are_unsupported() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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
            encrypted_wallet_backup_bytes(&wallet, &master_key, "unsupported-revision", 2),
        );
        globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

        let operation = new_restore_operation_for_test(&manager);
        let error = manager.do_restore_from_cloud_backup(&operation).unwrap_err();

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

    #[test]
    fn restore_fails_when_all_listed_wallet_backups_are_missing() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        let manager = RustCloudBackupManager::init();

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

        let operation = new_restore_operation_for_test(&manager);
        let error = manager.do_restore_from_cloud_backup(&operation).unwrap_err();

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
