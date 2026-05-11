use std::time::Duration;

use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{
    BlockingCloudStep, RustCloudBackupManager, blocking_cloud_error, sync::FinalizeUploadStateMode,
};
use crate::database::cloud_backup::CloudBackupRecordKey;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer,
    PasskeyMaterialOutcome, PreparedWalletBackup, StagedPrfKey, UnpersistedPrfKey,
    WalletBackupLookup, WalletBackupReader, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::workers::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableState, CloudBackupError, CloudBackupKeychain,
    CloudBackupPasskeyChoiceIntent, CloudBackupRootPrompt, CloudBackupStatus, CloudBackupStore,
    PendingEnableSession, PendingVerificationCompletion, PendingVerificationUpload,
    SavedPasskeyConfirmationMode, VerificationState, is_connectivity_related_issue,
    master_key_wrapper_revision_hash,
};
use crate::wallet::metadata::WalletMetadata;

struct MergeNamespace {
    matched: NamespaceMatch,
    wallet_record_ids: Vec<String>,
}

struct MergedNamespaceWallets {
    source: CleanupSourceNamespace,
    restored_wallets: Vec<WalletMetadata>,
}

pub(crate) enum EnablePasskeyAcquisition {
    Ready(StagedPrfKey),
    Cancelled,
}

pub(crate) enum EnablePasskeyRegistrationFlow {
    ForceNew,
    NoDiscovery,
}

impl EnablePasskeyRegistrationFlow {
    pub(crate) fn log_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new",
            Self::NoDiscovery => "Enable (no discovery)",
        }
    }

    pub(crate) fn cancelled_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new cancelled before passkey setup finished",
            Self::NoDiscovery => "Enable (no discovery) cancelled before passkey setup finished",
        }
    }

    pub(crate) fn failed_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new failed before passkey setup finished",
            Self::NoDiscovery => "Enable (no discovery) failed before passkey setup finished",
        }
    }
}

impl RustCloudBackupManager {
    fn pending_verification_uploads(
        uploaded_wallets: &[PreparedWalletBackup],
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
        verification_source: crate::manager::cloud_backup_manager::CloudBackupVerificationSource,
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

        let report = crate::manager::cloud_backup_manager::DeepVerificationReport {
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

        self.replace_pending_verification_completion_for_source(
            PendingVerificationCompletion::new(report, namespace_id.to_owned(), pending_uploads),
            verification_source,
        );
        self.set_verification(VerificationState::Idle);

        Ok(())
    }

    /// Complete recovery from matched cloud namespaces
    pub(crate) async fn complete_recovery(
        &self,
        cloud_keychain: &CloudBackupKeychain,
        cloud: &CloudStorageClient,
        cspp: &cove_cspp::Cspp<Keychain>,
        matches: Vec<NamespaceMatch>,
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
                                if is_connectivity_related_issue(&error) {
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
                        if is_connectivity_related_issue(&error) {
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

    pub(crate) async fn do_enable_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.do_enable_cloud_backup_with_context(CloudBackupEnableContext::settings_manual()).await
    }

    pub(crate) async fn do_enable_cloud_backup_with_context(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let context = pending.context();
            let (master_key, passkey) = pending.into_ready_parts()?;
            info!("Enable: retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(
                    Keychain::global(),
                    master_key,
                    passkey,
                    context,
                )
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
            return self.do_enable_cloud_backup_create_new_with_context(context).await;
        }

        // no local master key means iCloud may already contain a backup to recover
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
            return self.do_enable_cloud_backup_create_new_with_context(context).await;
        }

        info!("Enable: found {} existing namespace(s), attempting recovery", namespaces.len());
        let passkey_hint = self.best_passkey_hint_for_namespaces(&cloud, &namespaces).await;

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey);
        let match_outcome = matcher.match_namespaces(&namespaces).await?;
        match match_outcome {
            NamespaceMatchOutcome::Matched(matches) => {
                if matches.is_empty() {
                    self.set_existing_backup_found_prompt(context, passkey_hint);
                    self.clear_enable_progress(CloudBackupStatus::Disabled);
                    return Ok(());
                }

                self.complete_recovery(&cloud_keychain, &cloud, &cspp, matches).await
            }

            NamespaceMatchOutcome::UserDeclined => {
                info!("Enable: user cancelled passkey picker during namespace matching");
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context,
                    passkey_hint,
                ));
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                Ok(())
            }

            NamespaceMatchOutcome::NoMatch => {
                info!("Enable: passkey didn't match existing backups, asking user to confirm");
                self.set_existing_backup_found_prompt(context, passkey_hint);
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

    pub(crate) async fn do_enable_cloud_backup_create_new_with_context(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let context = pending.context();
            let (master_key, passkey) = pending.into_ready_parts()?;
            info!("Enable: retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(
                    Keychain::global(),
                    master_key,
                    passkey,
                    context,
                )
                .await;
        }
        if self.keep_awaiting_force_new_confirmation().await {
            return Ok(());
        }
        if self.keep_awaiting_saved_passkey_confirmation().await {
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
        self.set_enable_state(CloudBackupEnableState::CreatingPasskey);
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
        let passkey = match acquirer.discover_or_register_for_enable().await {
            Ok(PasskeyMaterialOutcome::Authenticated(passkey)) => passkey,
            Ok(PasskeyMaterialOutcome::RegisteredForConfirmation(passkey)) => {
                info!("Enable: passkey registered, confirming availability");
                return self
                    .stage_registered_passkey_for_confirmation(master_key, passkey, context)
                    .await;
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.rollback_new_local_master_key(
                    &cspp,
                    had_local_master_key,
                    "Enable cancelled before passkey setup finished",
                );
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                return Ok(());
            }
            Err(error) => {
                self.rollback_new_local_master_key(
                    &cspp,
                    had_local_master_key,
                    "Enable failed before passkey setup finished",
                );
                return Err(error);
            }
        };

        info!("Enable: passkey created, uploading backup");
        self.set_enable_state(CloudBackupEnableState::UploadingBackup);
        self.enable_cloud_backup_with_passkey_material(
            keychain,
            Zeroizing::new(master_key),
            Zeroizing::new(passkey),
            context,
        )
        .await
    }

    pub(crate) async fn do_enable_cloud_backup_force_new_with_context(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let keychain = Keychain::global();

        if let Some(pending) = self.take_pending_enable_session().await {
            let pending_context = pending.context();
            let (master_key, passkey) = pending.into_ready_parts()?;
            info!("Enable: committing pending create-first cloud backup");
            return self
                .enable_cloud_backup_with_passkey_material(
                    keychain,
                    master_key,
                    passkey,
                    pending_context,
                )
                .await;
        }

        self.register_new_enable_passkey_with_context(
            context,
            EnablePasskeyRegistrationFlow::ForceNew,
        )
        .await
    }

    pub(crate) async fn do_enable_cloud_backup_no_discovery_with_context(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        if let Some(pending) = self.take_retry_pending_enable_session().await {
            let context = pending.context();
            let (master_key, passkey) = pending.into_ready_parts()?;
            info!("Enable (no discovery): retrying pending upload with existing passkey material");
            return self
                .enable_cloud_backup_with_passkey_material(
                    Keychain::global(),
                    master_key,
                    passkey,
                    context,
                )
                .await;
        }
        if self.keep_awaiting_force_new_confirmation().await {
            return Ok(());
        }
        if self.keep_awaiting_saved_passkey_confirmation().await {
            return Ok(());
        }

        let keychain = Keychain::global();
        let cloud = CloudStorage::global_explicit_client();

        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let has_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();
        let existing_namespaces = if has_local_master_key {
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

        if !existing_namespaces.is_empty() {
            info!(
                "Enable (no discovery): found {} existing namespace(s), waiting for confirmation before creating passkey",
                existing_namespaces.len()
            );
            let passkey_hint =
                self.best_passkey_hint_for_namespaces(&cloud, &existing_namespaces).await;
            self.set_existing_backup_found_prompt(context, passkey_hint);
            self.clear_enable_progress(CloudBackupStatus::Disabled);
            return Ok(());
        }

        self.register_new_enable_passkey_with_context(
            context,
            EnablePasskeyRegistrationFlow::NoDiscovery,
        )
        .await
    }

    async fn register_new_enable_passkey_with_context(
        &self,
        context: CloudBackupEnableContext,
        flow: EnablePasskeyRegistrationFlow,
    ) -> Result<(), CloudBackupError> {
        let log_context = flow.log_context();
        let cancelled_context = flow.cancelled_context();
        let failed_context = flow.failed_context();

        let passkey_access = PasskeyAccess::global();
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let had_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();

        info!("{log_context}: getting master key");
        let master_key = cspp
            .get_or_create_master_key()
            .map_err_prefix("master key", CloudBackupError::Internal)?;

        let namespace_id = master_key.namespace_id();
        info!("{log_context}: namespace_id={namespace_id}, creating passkey");
        self.set_enable_state(CloudBackupEnableState::CreatingPasskey);
        let passkey = match self
            .acquire_enable_passkey(
                &cspp,
                had_local_master_key,
                cancelled_context,
                failed_context,
                || {
                    let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
                    async move { acquirer.register_for_enable().await }
                },
            )
            .await?
        {
            EnablePasskeyAcquisition::Ready(passkey) => passkey,
            EnablePasskeyAcquisition::Cancelled => {
                self.set_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                self.clear_enable_progress(CloudBackupStatus::Disabled);
                return Ok(());
            }
        };

        info!("{log_context}: passkey registered, confirming availability");
        self.stage_registered_passkey_for_confirmation(master_key, passkey, context).await
    }

    async fn enable_cloud_backup_with_passkey_material(
        &self,
        keychain: &Keychain,
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<UnpersistedPrfKey>,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        self.set_enable_state(CloudBackupEnableState::UploadingBackup);
        let namespace_id = master_key.namespace_id();
        let cloud = CloudStorage::global_explicit_client();
        self.replace_pending_enable_session(PendingEnableSession::retry_upload(
            cove_cspp::master_key::MasterKey::from_bytes(*master_key.as_bytes()),
            passkey.copy_for_retry(),
            context,
        ))
        .await;

        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let encrypted_master = master_key_crypto::encrypt_master_key_with_remote_metadata(
            &master_key,
            &passkey.prf_key,
            &passkey.prf_salt,
            passkey.provider_hint.clone(),
            RemotePayloadMetadata::master_key(&namespace_id, uploaded_at),
        )
        .map_err_str(CloudBackupError::Crypto)?;
        let master_json =
            serde_json::to_vec(&encrypted_master).map_err_str(CloudBackupError::Internal)?;
        let master_key_wrapper_revision = master_key_wrapper_revision_hash(&master_json);

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

        self.mark_blob_uploaded_pending_confirmation(
            &namespace_id,
            CloudBackupRecordKey::MasterKeyWrapper,
            master_key_wrapper_revision,
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
            context.verification_source,
        )
        .await?;
        self.clear_pending_enable_session();
        self.clear_enable_progress(CloudBackupStatus::Enabled);
        self.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        Ok(())
    }

    pub(crate) fn clear_enable_progress(&self, status: CloudBackupStatus) {
        let snapshot = self.model_snapshot();
        let preserve_awaiting_prompt = matches!(status, CloudBackupStatus::Disabled)
            && matches!(snapshot.status, CloudBackupStatus::Enabling)
            && matches!(
                snapshot.root_prompt,
                CloudBackupRootPrompt::ExistingBackupFound(_, _)
                    | CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(
                        _,
                        _,
                    ))
            );

        self.set_progress(None);
        self.set_restore_progress(None);
        self.set_enable_state(CloudBackupEnableState::Idle);
        if preserve_awaiting_prompt {
            self.set_status(CloudBackupStatus::Enabling);
        } else {
            self.set_status(status);
        }
    }

    pub(crate) async fn stage_registered_passkey_for_confirmation(
        &self,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: StagedPrfKey,
        context: CloudBackupEnableContext,
    ) -> Result<(), CloudBackupError> {
        self.replace_pending_enable_session(
            PendingEnableSession::awaiting_saved_passkey_confirmation(master_key, passkey, context),
        )
        .await;
        self.set_enable_state(CloudBackupEnableState::CreatingPasskey);

        tokio::time::sleep(Duration::from_secs(3)).await;

        // do not poll credential presence here; platform presence checks can show passkey UI
        // after registration, so confirmation must happen only from an explicit user action
        self.set_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            context.saved_passkey_confirmation,
        ));
        Ok(())
    }

    pub(crate) async fn handle_confirm_saved_passkey_session(&self, pending: PendingEnableSession) {
        match self.confirm_saved_passkey_from_session(pending).await {
            Ok(()) => {}
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.set_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                    SavedPasskeyConfirmationMode::Manual,
                ));
            }
            Err(CloudBackupError::Passkey(_))
            | Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                self.set_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                    SavedPasskeyConfirmationMode::Manual,
                ));
            }
            Err(error) => {
                warn!("Confirm saved passkey failed: {error}");
                self.finish_background_operation_error(&error);
            }
        }
    }

    async fn confirm_saved_passkey_from_session(
        &self,
        pending: PendingEnableSession,
    ) -> Result<(), CloudBackupError> {
        let context = pending.context();
        let (master_key, staged_passkey) = pending.into_staged_parts()?;
        let passkey_access = PasskeyAccess::global();
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);

        match acquirer.confirm_registered_for_enable(&staged_passkey).await {
            Ok(passkey) => {
                self.set_enable_state(CloudBackupEnableState::UploadingBackup);
                self.enable_cloud_backup_with_passkey_material(
                    Keychain::global(),
                    master_key,
                    Zeroizing::new(passkey),
                    context,
                )
                .await
            }
            Err(error @ CloudBackupError::PasskeyDiscoveryCancelled)
            | Err(error @ CloudBackupError::Passkey(_))
            | Err(error @ CloudBackupError::UnsupportedPasskeyProvider) => {
                self.replace_pending_enable_session(
                    PendingEnableSession::awaiting_saved_passkey_confirmation(
                        cove_cspp::master_key::MasterKey::from_bytes(*master_key.as_bytes()),
                        staged_passkey.copy_for_retry(),
                        context,
                    ),
                )
                .await;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn keep_awaiting_force_new_confirmation(&self) -> bool {
        let Some(context) = self.awaiting_force_new_enable_context().await else {
            return false;
        };

        self.set_existing_backup_found_prompt(context, None);
        self.clear_enable_progress(CloudBackupStatus::Disabled);
        true
    }

    pub(crate) async fn keep_awaiting_saved_passkey_confirmation(&self) -> bool {
        if !self.has_awaiting_saved_passkey_confirmation_session().await {
            return false;
        }

        self.set_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));
        true
    }

    pub(crate) fn rollback_new_local_master_key(
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

    pub(crate) async fn acquire_enable_passkey<F, Fut>(
        &self,
        cspp: &cove_cspp::Cspp<Keychain>,
        had_local_master_key: bool,
        cancelled_context: &str,
        failed_context: &str,
        acquire: F,
    ) -> Result<EnablePasskeyAcquisition, CloudBackupError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<StagedPrfKey, CloudBackupError>>,
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
