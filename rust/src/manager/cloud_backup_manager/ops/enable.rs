use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::{BlockingCloudStep, RustCloudBackupManager, blocking_cloud_error};
use crate::manager::cloud_backup_manager::actors::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupUploadedWallet,
    CloudBackupWriteClient,
};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer,
    PasskeyMaterialOutcome, PreparedWalletBackup, StagedPrfKey, UnpersistedPrfKey,
    WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableOutcome, CloudBackupError, CloudBackupPasskeyHint,
    CloudBackupRestoreOutcome, CloudBackupStatus, CloudBackupStore, PendingEnableSession,
    PendingVerificationUpload, is_connectivity_related_issue, master_key_wrapper_revision_hash,
};
use crate::wallet::metadata::WalletMetadata;

pub(crate) struct MergeNamespace {
    matched: NamespaceMatch,
    wallet_record_ids: Vec<String>,
}

pub(crate) struct MergedNamespaceWallets {
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

pub(crate) struct CloudBackupRegisteredEnablePasskey {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<StagedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
}

pub(crate) enum CloudBackupEnablePasskeyRegistration {
    Registered(CloudBackupRegisteredEnablePasskey),
    Cancelled { context: CloudBackupEnableContext },
}

pub(crate) enum CloudBackupEnablePasskeyPreparation {
    Ready(CloudBackupReadyEnableUpload),
    Registered(CloudBackupRegisteredEnablePasskey),
    Cancelled { context: CloudBackupEnableContext },
}

pub(crate) enum CloudBackupEnablePreparation {
    CreateNew {
        context: CloudBackupEnableContext,
    },
    ExistingBackupFound {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    PasskeyChoice {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    Recover {
        context: CloudBackupEnableContext,
        matches: Vec<NamespaceMatch>,
    },
}

pub(crate) struct CloudBackupEnableRecoveryPreparation {
    context: CloudBackupEnableContext,
    merge_namespaces: Vec<MergeNamespace>,
    active_index: usize,
    active_namespace_id: String,
    active_master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    active_critical_key: Zeroizing<[u8; 32]>,
}

pub(crate) struct CloudBackupEnableRecoveryCompletion {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) active_critical_key: Zeroizing<[u8; 32]>,
    pub(crate) uploaded_wallets: Vec<CloudBackupUploadedWallet>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for CloudBackupEnableRecoveryCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupEnableRecoveryCompletion")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("uploaded_wallets_count", &self.uploaded_wallets.len())
            .field("pending_uploads_count", &self.pending_uploads.len())
            .field("cleanup_sources_count", &self.cleanup_sources.len())
            .finish()
    }
}

pub(crate) enum CloudBackupNoDiscoveryEnablePreparation {
    RegisterPasskey {
        context: CloudBackupEnableContext,
    },
    ExistingBackupFound {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
}

pub(crate) struct CloudBackupReadyEnableUpload {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
}

pub(crate) struct CloudBackupUploadedEnableBackup {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup,
    pub(crate) master_key_wrapper_revision: String,
    pub(crate) uploaded_at: u64,
    pub(crate) uploaded_wallets: Vec<PreparedWalletBackup>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
}

pub(crate) enum CloudBackupSavedPasskeyConfirmation {
    Confirmed(CloudBackupReadyEnableUpload),
    Retry { pending: PendingEnableSession, error: CloudBackupError },
    Failed(CloudBackupError),
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

    pub(crate) async fn prepare_enable_recovery(
        &self,
        context: CloudBackupEnableContext,
        matches: Vec<NamespaceMatch>,
    ) -> Result<CloudBackupEnableRecoveryPreparation, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let merge_namespaces = self.load_enable_merge_namespaces(&cloud, matches).await?;
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

        Ok(CloudBackupEnableRecoveryPreparation {
            context,
            merge_namespaces,
            active_index,
            active_namespace_id,
            active_master_key: Zeroizing::new(active_master_key),
            active_critical_key: Zeroizing::new(active_critical_key),
        })
    }

    pub(crate) fn save_enable_recovery_master_key(
        &self,
        preparation: &CloudBackupEnableRecoveryPreparation,
    ) -> Result<(), CloudBackupError> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.save_master_key(&preparation.active_master_key)
            .map_err_prefix("save recovered master key", CloudBackupError::Internal)
    }

    pub(crate) fn rollback_enable_recovery_master_key(&self) {
        warn!("Enable: rolling back recovered local master key after recovery failure");
        cove_cspp::Cspp::new(Keychain::global().clone()).delete_master_key();
    }

    pub(crate) async fn prepare_enable_recovery_completion(
        &self,
        preparation: CloudBackupEnableRecoveryPreparation,
        writes: CloudBackupWriteClient,
    ) -> Result<CloudBackupEnableRecoveryCompletion, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let active_namespace_id = preparation.active_namespace_id.clone();
        let active_critical_key = preparation.active_critical_key;
        let merged_wallets =
            self.restore_enable_merge_wallets(&cloud, &preparation.merge_namespaces).await?;

        for merged in &merged_wallets {
            info!(
                "Enable: recovered {} wallet(s) from matched namespace {}",
                merged.restored_wallets.len(),
                merged.source.namespace_id
            );
        }

        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&writes, &cloud, &active_namespace_id, &active_critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Enable, error))?;
        let pending_uploads = Self::pending_verification_uploads(&uploaded_wallets);

        let active_match = &preparation.merge_namespaces[preparation.active_index].matched;
        let credential_id = active_match.credential_id.clone();
        let prf_salt = active_match.prf_salt;
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
        let cleanup_sources = merged_wallets
            .into_iter()
            .filter(|merged| merged.source.namespace_id != active_namespace_id)
            .map(|merged| merged.source)
            .collect::<Vec<_>>();

        Ok(CloudBackupEnableRecoveryCompletion {
            context: preparation.context,
            namespace_id: active_namespace_id,
            credential_id,
            prf_salt,
            active_critical_key,
            uploaded_wallets,
            pending_uploads,
            cleanup_sources,
        })
    }

    async fn load_enable_merge_namespaces(
        &self,
        cloud: &CloudStorageClient,
        matches: Vec<NamespaceMatch>,
    ) -> Result<Vec<MergeNamespace>, CloudBackupError> {
        let mut merge_namespaces = Vec::with_capacity(matches.len());

        for matched in matches {
            let wallet_record_ids =
                match cloud.list_wallet_backups(matched.namespace_id.clone()).await {
                    Ok(wallet_record_ids) => wallet_record_ids,
                    Err(error) => {
                        return Err(blocking_cloud_error(
                            BlockingCloudStep::Enable,
                            CloudBackupError::cloud_storage_context("list wallet backups", error),
                        ));
                    }
                };

            merge_namespaces.push(MergeNamespace { matched, wallet_record_ids });
        }

        Ok(merge_namespaces)
    }

    async fn restore_enable_merge_wallets(
        &self,
        cloud: &CloudStorageClient,
        namespaces: &[MergeNamespace],
    ) -> Result<Vec<MergedNamespaceWallets>, CloudBackupError> {
        let existing_identities = crate::wallet_identity::collect_existing_wallet_identities()
            .map_err_prefix("collect wallet identities", CloudBackupError::Internal)?;
        let mut restore_session = WalletRestoreSession::new(existing_identities);
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
                let wallet = match reader.lookup(record_id).await {
                    Ok(WalletBackupLookup::Found(wallet)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: Some(wallet.entry.content_revision_hash.clone()),
                        });

                        wallet
                    }
                    Ok(WalletBackupLookup::NotFound) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!("Enable: matched namespace listed a missing wallet backup");
                        continue;
                    }
                    Ok(WalletBackupLookup::UnsupportedVersion(version)) => {
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!(
                            "Enable: matched namespace uses unsupported wallet backup version {version}"
                        );
                        continue;
                    }
                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(BlockingCloudStep::Enable, error));
                        }
                        expected_wallets.push(CleanupExpectedWalletRecord {
                            record_id: record_id.clone(),
                            content_revision_hash: None,
                        });
                        warn!("Enable: failed to inspect wallet during namespace merge: {error}");
                        continue;
                    }
                };

                match restore_session.restore_downloaded(&wallet) {
                    Ok(WalletRestoreOutcome::Restored { .. }) => {
                        restored_wallets.push(wallet.metadata)
                    }
                    Ok(WalletRestoreOutcome::SkippedDuplicate) => {}
                    Err(error) => {
                        if is_connectivity_related_issue(&error) {
                            return Err(blocking_cloud_error(BlockingCloudStep::Enable, error));
                        }
                        warn!("Enable: failed to restore wallet during namespace merge: {error}");
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

    pub(crate) async fn prepare_enable(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<CloudBackupEnablePreparation, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let passkey = PasskeyAccess::global();
        if !passkey.is_prf_supported() {
            return Err(CloudBackupError::NotSupported(
                "PRF extension not supported on this device".into(),
            ));
        }

        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global_explicit_client();

        let has_local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .is_some();

        if has_local_master_key {
            return Ok(CloudBackupEnablePreparation::CreateNew { context });
        }

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
            return Ok(CloudBackupEnablePreparation::CreateNew { context });
        }

        info!("Enable: found {} existing namespace(s), attempting recovery", namespaces.len());
        let passkey_hint = self.best_passkey_hint_for_namespaces(&cloud, &namespaces).await;

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey);
        let match_outcome = matcher.match_namespaces(&namespaces).await?;
        match match_outcome {
            NamespaceMatchOutcome::Matched(matches) => {
                if matches.is_empty() {
                    return Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                        context,
                        passkey_hint,
                    });
                }

                Ok(CloudBackupEnablePreparation::Recover { context, matches })
            }

            NamespaceMatchOutcome::UserDeclined => {
                info!("Enable: user cancelled passkey picker during namespace matching");
                Ok(CloudBackupEnablePreparation::PasskeyChoice {
                    context,
                    passkey_hint,
                })
            }

            NamespaceMatchOutcome::NoMatch => {
                info!("Enable: passkey didn't match existing backups, asking user to confirm");
                Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                    context,
                    passkey_hint,
                })
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

    pub(crate) async fn prepare_create_new_enable_passkey(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<CloudBackupEnablePasskeyPreparation, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
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
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
        match acquirer.discover_or_register_for_enable().await {
            Ok(PasskeyMaterialOutcome::Authenticated(passkey)) => {
                Ok(CloudBackupEnablePasskeyPreparation::Ready(CloudBackupReadyEnableUpload {
                    master_key: Zeroizing::new(master_key),
                    passkey: Zeroizing::new(passkey),
                    context,
                }))
            }
            Ok(PasskeyMaterialOutcome::RegisteredForConfirmation(passkey)) => {
                Ok(CloudBackupEnablePasskeyPreparation::Registered(
                    CloudBackupRegisteredEnablePasskey {
                        master_key: Zeroizing::new(master_key),
                        passkey: Zeroizing::new(passkey),
                        context,
                    },
                ))
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.rollback_new_local_master_key(
                    &cspp,
                    had_local_master_key,
                    "Enable cancelled before passkey setup finished",
                );
                Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context })
            }
            Err(error) => {
                self.rollback_new_local_master_key(
                    &cspp,
                    had_local_master_key,
                    "Enable failed before passkey setup finished",
                );
                Err(error)
            }
        }
    }

    pub(crate) async fn prepare_no_discovery_enable(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<CloudBackupNoDiscoveryEnablePreparation, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
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
            return Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                context,
                passkey_hint,
            });
        }

        Ok(CloudBackupNoDiscoveryEnablePreparation::RegisterPasskey { context })
    }

    pub(crate) async fn prepare_new_enable_passkey_for_confirmation(
        &self,
        context: CloudBackupEnableContext,
        flow: EnablePasskeyRegistrationFlow,
    ) -> Result<CloudBackupEnablePasskeyRegistration, CloudBackupError> {
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
                return Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context });
            }
        };

        info!("{log_context}: passkey registered, confirming availability");
        Ok(CloudBackupEnablePasskeyRegistration::Registered(CloudBackupRegisteredEnablePasskey {
            master_key: Zeroizing::new(master_key),
            passkey: Zeroizing::new(passkey),
            context,
        }))
    }

    pub(crate) fn clear_enable_progress(&self, status: CloudBackupStatus) {
        let preserve_awaiting_prompt = matches!(status, CloudBackupStatus::Disabled)
            && self.state.read().is_awaiting_enable_prompt();

        self.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        if preserve_awaiting_prompt {
            self.reconcile_runtime_status(CloudBackupStatus::Enabling);
        } else {
            self.reconcile_runtime_status(status);
        }
    }

    pub(crate) async fn confirm_saved_passkey_from_session(
        &self,
        pending: PendingEnableSession,
    ) -> CloudBackupSavedPasskeyConfirmation {
        let context = pending.context();
        let (master_key, staged_passkey) = match pending.into_staged_parts() {
            Ok(parts) => parts,
            Err(error) => return CloudBackupSavedPasskeyConfirmation::Failed(error),
        };
        let passkey_access = PasskeyAccess::global();
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);

        match acquirer.confirm_registered_for_enable(&staged_passkey).await {
            Ok(passkey) => {
                CloudBackupSavedPasskeyConfirmation::Confirmed(CloudBackupReadyEnableUpload {
                    master_key,
                    passkey: Zeroizing::new(passkey),
                    context,
                })
            }
            Err(error @ CloudBackupError::PasskeyDiscoveryCancelled)
            | Err(error @ CloudBackupError::Passkey(_)) => {
                let pending = PendingEnableSession::awaiting_saved_passkey_confirmation(
                    Zeroizing::new(cove_cspp::master_key::MasterKey::from_bytes(
                        *master_key.as_bytes(),
                    )),
                    Zeroizing::new(staged_passkey.copy_for_retry()),
                    context,
                );
                CloudBackupSavedPasskeyConfirmation::Retry { pending, error }
            }
            Err(error) => CloudBackupSavedPasskeyConfirmation::Failed(error),
        }
    }

    pub(crate) async fn upload_ready_enable_backup(
        &self,
        ready: CloudBackupReadyEnableUpload,
        writes: CloudBackupWriteClient,
    ) -> Result<CloudBackupUploadedEnableBackup, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Enable)?;
        let namespace_id = ready.master_key.namespace_id();
        let cloud = CloudStorage::global_explicit_client();

        let uploaded_at = crate::manager::cloud_backup_manager::current_timestamp();
        let encrypted_master = master_key_crypto::encrypt_master_key_with_remote_metadata(
            &ready.master_key,
            &ready.passkey.prf_key,
            &ready.passkey.prf_salt,
            ready.passkey.provider_hint.clone(),
            RemotePayloadMetadata::master_key(&namespace_id, uploaded_at),
        )
        .map_err_str(CloudBackupError::Crypto)?;
        let master_json =
            serde_json::to_vec(&encrypted_master).map_err_str(CloudBackupError::Internal)?;
        let master_key_wrapper_revision = master_key_wrapper_revision_hash(&master_json);

        info!("Enable: uploading master key");
        writes
            .upload_master_key_backup(cloud.clone(), namespace_id.clone(), master_json)
            .await
            .map_err(|error| {
                let error = match error {
                    CloudBackupError::CloudStorage(source) => {
                        CloudBackupError::cloud_storage_context("upload master key backup", source)
                    }
                    error => error,
                };

                blocking_cloud_error(BlockingCloudStep::Enable, error)
            })?;

        info!("Enable: uploading wallets");
        let critical_key = Zeroizing::new(ready.master_key.critical_data_key());
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets(&writes, &cloud, &namespace_id, &critical_key)
            .await
            .map_err(|error| blocking_cloud_error(BlockingCloudStep::Enable, error))?;
        let pending_uploads = Self::pending_verification_uploads(&uploaded_wallets);

        Ok(CloudBackupUploadedEnableBackup {
            master_key: ready.master_key,
            passkey: ready.passkey,
            context: ready.context,
            namespace_id,
            encrypted_master,
            master_key_wrapper_revision,
            uploaded_at,
            uploaded_wallets,
            pending_uploads,
        })
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
