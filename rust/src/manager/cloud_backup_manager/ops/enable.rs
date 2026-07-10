use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::master_key_crypto;
use cove_cspp::{MasterKeyPromotionStatus, master_key::MasterKey};
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
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
    CloudBackupEnableContext, CloudBackupEnableState, CloudBackupError, CloudBackupKeychain,
    CloudBackupPasskeyHint, CloudBackupProgress, CloudBackupRestoreOutcome, CloudBackupStatus,
    CloudBackupStore, PendingEnableJournal, PendingEnableJournalPhase,
    PendingEnableNamespaceOwnership, PendingEnablePasskeyMetadata, PendingEnableSession,
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

impl CloudBackupEnableRecoveryPreparation {
    fn recovered_passkey_metadata(&self) -> PendingEnablePasskeyMetadata {
        let matched = &self.merge_namespaces[self.active_index].matched;

        PendingEnablePasskeyMetadata {
            credential_id: matched.credential_id.clone(),
            prf_salt: matched.prf_salt,
            provider_hint: None,
        }
    }
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
        if active_master_key.namespace_id() != active_namespace_id {
            return Err(CloudBackupError::Internal(
                "recovered master key did not match its cloud namespace".into(),
            ));
        }
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
        let keychain = Keychain::global().clone();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain);
        let recovered_passkey = preparation.recovered_passkey_metadata();

        if let Some(mut journal) = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
        {
            let staged = cspp.load_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context("load staged recovered master key", source)
            })?;
            let is_matching_recovery = journal.namespace_ownership()
                == PendingEnableNamespaceOwnership::RecoveredExisting
                && journal.namespace_id() == preparation.active_namespace_id
                && journal.context() == preparation.context
                && staged.as_ref().is_some_and(|master_key| {
                    master_key.as_bytes() == preparation.active_master_key.as_bytes()
                });
            if !is_matching_recovery
                || matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_))
                || !journal.register_passkey(recovered_passkey)
            {
                return Err(CloudBackupError::Internal(
                    "a different pending Cloud Backup enable must be recovered first".into(),
                ));
            }

            return cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
                CloudBackupError::internal_context("save recovered pending enable passkey", source)
            });
        }

        match cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect recovered master key stage", source)
        })? {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                cspp.discard_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context(
                        "discard unowned staged recovered master key",
                        source,
                    )
                })?;
            }
            MasterKeyPromotionStatus::Pending(_) => {
                return Err(CloudBackupError::Internal(
                    "a prior Cloud Backup master key promotion must be recovered first".into(),
                ));
            }
        }

        cspp.save_staged_master_key(&preparation.active_master_key).map_err(|source| {
            CloudBackupError::internal_context("stage recovered master key", source)
        })?;
        let mut journal = PendingEnableJournal::staged(
            preparation.context,
            preparation.active_namespace_id.clone(),
            PendingEnableNamespaceOwnership::RecoveredExisting,
            cloud_keychain.snapshot_passkey_metadata(),
        );
        if !journal.register_passkey(recovered_passkey) {
            let _ = cspp.discard_staged_master_key();

            return Err(CloudBackupError::Internal(
                "could not record recovered Cloud Backup passkey".into(),
            ));
        }
        if let Err(source) = cloud_keychain.save_pending_enable_journal(&journal) {
            let _ = cspp.discard_staged_master_key();

            return Err(CloudBackupError::internal_context(
                "save recovered pending enable",
                source,
            ));
        }

        Ok(())
    }

    pub(crate) fn rollback_enable_recovery_master_key(&self) -> Result<(), CloudBackupError> {
        warn!("Enable: rolling back recovered local master key after recovery failure");
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending recovered Cloud Backup state is missing".into())
            })?;
        if journal.namespace_ownership() != PendingEnableNamespaceOwnership::RecoveredExisting {
            return Err(CloudBackupError::Internal(
                "refusing to roll back a non-recovery pending enable as recovered state".into(),
            ));
        }

        cove_cspp::Cspp::new(Keychain::global().clone()).rollback_master_key_promotion().map_err(
            |source| CloudBackupError::internal_context("roll back recovered master key", source),
        )?;
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior Cloud Backup passkey metadata",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear rolled back recovered enable state", source)
        })
    }

    pub(crate) async fn prepare_enable_recovery_completion(
        &self,
        preparation: CloudBackupEnableRecoveryPreparation,
        writes: CloudBackupWriteClient,
    ) -> Result<CloudBackupEnableRecoveryCompletion, CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let active_namespace_id = preparation.active_namespace_id.clone();
        let merged_wallets =
            self.restore_enable_merge_wallets(&cloud, &preparation.merge_namespaces).await?;

        for merged in &merged_wallets {
            info!(
                "Enable: recovered {} wallet(s) from matched namespace {}",
                merged.restored_wallets.len(),
                merged.source.namespace_id
            );
        }

        self.mark_enable_recovery_remote_writes_started(&preparation)?;
        let active_critical_key = preparation.active_critical_key;
        let uploaded_wallets = CloudBackupStore::global()
            .upload_all_wallets_with_progress(
                &writes,
                &cloud,
                &active_namespace_id,
                &active_critical_key,
                0,
                |progress| self.report_enable_progress(progress),
            )
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

    fn mark_enable_recovery_remote_writes_started(
        &self,
        preparation: &CloudBackupEnableRecoveryPreparation,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let expected_passkey = preparation.recovered_passkey_metadata();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending recovered Cloud Backup state is missing".into())
            })?;
        if journal.namespace_ownership() != PendingEnableNamespaceOwnership::RecoveredExisting
            || journal.namespace_id() != preparation.active_namespace_id
            || journal.passkey() != Some(&expected_passkey)
            || !journal.mark_remote_writes_started()
        {
            return Err(CloudBackupError::Internal(
                "pending recovered Cloud Backup state cannot start remote writes".into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context(
                "mark recovered pending enable remote writes",
                source,
            )
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
            .map_err(|source| {
                CloudBackupError::internal_context("collect wallet identities", source)
            })?;
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
            .map_err(|source| CloudBackupError::internal_context("load local master key", source))?
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

        info!("Enable: staging fresh master key");
        let (master_key, context) = self.stage_fresh_enable_master(context)?;

        let namespace_id = master_key.namespace_id();
        info!("Enable: namespace_id={namespace_id}, getting passkey");
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
        match acquirer.discover_or_register_for_enable().await {
            Ok(PasskeyMaterialOutcome::Authenticated(passkey)) => {
                self.record_pending_enable_passkey(&master_key, &passkey)?;
                Ok(CloudBackupEnablePasskeyPreparation::Ready(CloudBackupReadyEnableUpload {
                    master_key: Zeroizing::new(master_key),
                    passkey: Zeroizing::new(passkey),
                    context,
                }))
            }
            Ok(PasskeyMaterialOutcome::RegisteredForConfirmation(passkey)) => {
                self.record_pending_enable_staged_passkey(&master_key, &passkey)?;
                Ok(CloudBackupEnablePasskeyPreparation::Registered(
                    CloudBackupRegisteredEnablePasskey {
                        master_key: Zeroizing::new(master_key),
                        passkey: Zeroizing::new(passkey),
                        context,
                    },
                ))
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.discard_unpromoted_enable_stage(
                    "Enable cancelled before passkey setup finished",
                )?;
                Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context })
            }
            Err(error) => {
                self.discard_unpromoted_enable_stage(
                    "Enable failed before passkey setup finished",
                )?;
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
            .map_err(|source| CloudBackupError::internal_context("load local master key", source))?
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

        info!("{log_context}: staging fresh master key");
        let (master_key, context) = self.stage_fresh_enable_master(context)?;

        let namespace_id = master_key.namespace_id();
        info!("{log_context}: namespace_id={namespace_id}, creating passkey");
        let acquisition = self
            .acquire_enable_passkey(|| {
                let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
                async move { acquirer.register_for_enable().await }
            })
            .await;
        let passkey = match acquisition {
            Err(error) => {
                self.discard_unpromoted_enable_stage(failed_context)?;
                return Err(error);
            }
            Ok(EnablePasskeyAcquisition::Ready(passkey)) => passkey,
            Ok(EnablePasskeyAcquisition::Cancelled) => {
                self.discard_unpromoted_enable_stage(cancelled_context)?;
                return Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context });
            }
        };

        if let Err(error) = self.record_pending_enable_staged_passkey(&master_key, &passkey) {
            self.discard_unpromoted_enable_stage(failed_context)?;
            return Err(error);
        }

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

        self.clear_enable_progress_report();
        self.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        self.apply_enable_state(CloudBackupEnableState::Idle);
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
        self.mark_pending_enable_remote_writes_started(&ready.master_key, &ready.passkey)?;
        let namespace_id = ready.master_key.namespace_id();
        let cloud = CloudStorage::global_explicit_client();
        let store = CloudBackupStore::global();
        let total = store.wallet_count()?.saturating_add(1);
        self.report_enable_progress(CloudBackupProgress { completed: 0, total });

        let uploaded_at = crate::manager::cloud_backup_manager::current_timestamp();
        let encrypted_master = master_key_crypto::encrypt_master_key_with_remote_metadata(
            &ready.master_key,
            &ready.passkey.prf_key,
            &ready.passkey.prf_salt,
            ready.passkey.provider_hint.clone(),
            RemotePayloadMetadata::master_key(&namespace_id, uploaded_at),
        )
        .map_err(CloudBackupError::crypto)?;
        let master_json =
            serde_json::to_vec(&encrypted_master).map_err(CloudBackupError::internal)?;
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
        let uploaded_wallets = store
            .upload_all_wallets_with_progress(
                &writes,
                &cloud,
                &namespace_id,
                &critical_key,
                1,
                |progress| self.report_enable_progress(progress),
            )
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

    pub(crate) async fn acquire_enable_passkey<F, Fut>(
        &self,
        acquire: F,
    ) -> Result<EnablePasskeyAcquisition, CloudBackupError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<StagedPrfKey, CloudBackupError>>,
    {
        match acquire().await {
            Ok(passkey) => Ok(EnablePasskeyAcquisition::Ready(passkey)),
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                Ok(EnablePasskeyAcquisition::Cancelled)
            }
            Err(error) => Err(error),
        }
    }

    fn stage_fresh_enable_master(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(MasterKey, CloudBackupEnableContext), CloudBackupError> {
        let keychain = Keychain::global().clone();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain);

        if let Some(journal) = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
        {
            if journal.namespace_ownership() != PendingEnableNamespaceOwnership::FreshOwned {
                return Err(CloudBackupError::Internal(
                    "an existing-backup recovery must be resumed before starting a new backup"
                        .into(),
                ));
            }
            if !matches!(journal.phase(), PendingEnableJournalPhase::Staged) {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable must be resumed before starting another backup"
                        .into(),
                ));
            }
            let staged = cspp.load_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context("load staged master key", source)
            })?;
            let Some(staged) = staged else {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable is missing its staged master key".into(),
                ));
            };
            if staged.namespace_id() != journal.namespace_id() {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable has mismatched staged ownership".into(),
                ));
            }

            return Ok((staged, journal.context()));
        }

        match cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect staged master key", source)
        })? {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                cspp.discard_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context("discard unowned staged master key", source)
                })?;
            }
            MasterKeyPromotionStatus::Pending(_) => {
                return Err(CloudBackupError::Internal(
                    "a prior Cloud Backup master key promotion must be recovered first".into(),
                ));
            }
        }

        let master_key = MasterKey::generate();
        cspp.save_staged_master_key(&master_key).map_err(|source| {
            CloudBackupError::internal_context("stage fresh master key", source)
        })?;
        let journal = PendingEnableJournal::staged(
            context,
            master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            cloud_keychain.snapshot_passkey_metadata(),
        );
        if let Err(source) = cloud_keychain.save_pending_enable_journal(&journal) {
            let _ = cspp.discard_staged_master_key();
            return Err(CloudBackupError::internal_context("save pending enable", source));
        }

        Ok((master_key, context))
    }

    fn record_pending_enable_staged_passkey(
        &self,
        master_key: &MasterKey,
        passkey: &StagedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey_metadata(
            master_key,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    fn record_pending_enable_passkey(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey_metadata(
            master_key,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    fn record_pending_enable_passkey_metadata(
        &self,
        master_key: &MasterKey,
        passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if journal.namespace_id() != master_key.namespace_id() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched passkey ownership".into(),
            ));
        }
        if !journal.register_passkey(passkey) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable passkey changed unexpectedly".into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("save pending enable passkey", source)
        })
    }

    fn mark_pending_enable_remote_writes_started(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey(master_key, passkey)?;

        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !journal.mark_remote_writes_started() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot start remote writes before passkey setup"
                    .into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("mark pending enable remote writes", source)
        })
    }

    pub(crate) fn begin_pending_enable_local_promotion(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.begin_pending_enable_local_promotion_with_metadata(
            &master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    pub(crate) fn begin_enable_recovery_local_promotion(
        &self,
        namespace_id: &str,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<(), CloudBackupError> {
        self.begin_pending_enable_local_promotion_with_metadata(
            namespace_id,
            PendingEnableNamespaceOwnership::RecoveredExisting,
            PendingEnablePasskeyMetadata {
                credential_id: credential_id.to_vec(),
                prf_salt,
                provider_hint: None,
            },
        )
    }

    fn begin_pending_enable_local_promotion_with_metadata(
        &self,
        namespace_id: &str,
        namespace_ownership: PendingEnableNamespaceOwnership,
        expected_passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let staged = cspp.load_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("load staged master key for promotion", source)
        })?;
        if staged.as_ref().is_none_or(|master_key| master_key.namespace_id() != namespace_id) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched staged master key ownership".into(),
            ));
        }

        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if journal.namespace_ownership() != namespace_ownership
            || journal.namespace_id() != namespace_id
            || journal.passkey() != Some(&expected_passkey)
            || !journal.mark_local_promotion_started()
        {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot promote unowned staged material".into(),
            ));
        }
        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("mark pending enable promotion", source)
        })?;

        if let Err(source) = cspp.promote_staged_master_key() {
            return self
                .restore_pending_enable_local_promotion_for_retry()
                .and(Err(CloudBackupError::internal_context("promote staged master key", source)));
        }
        if let Err(source) = cloud_keychain.save_passkey_and_namespace(
            &expected_passkey.credential_id,
            expected_passkey.prf_salt,
            journal.namespace_id(),
        ) {
            return self.restore_pending_enable_local_promotion_for_retry().and(Err(
                CloudBackupError::internal_context("promote Cloud Backup passkey metadata", source),
            ));
        }

        Ok(())
    }

    pub(crate) fn restore_pending_enable_local_promotion_for_retry(
        &self,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !journal.roll_back_local_promotion() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has no local promotion to restore".into(),
            ));
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.restore_prior_master_key_for_retry().map_err(|source| {
            CloudBackupError::internal_context("restore prior master key for retry", source)
        })?;
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior Cloud Backup passkey metadata",
                source,
            )
        })?;
        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("restore pending enable retry state", source)
        })
    }

    pub(crate) fn commit_pending_enable_local_promotion(&self) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable local promotion is not ready to commit".into(),
            ));
        }

        cove_cspp::Cspp::new(Keychain::global().clone()).commit_master_key_promotion().map_err(
            |source| {
                CloudBackupError::internal_context("commit staged master key promotion", source)
            },
        )?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear committed pending enable state", source)
        })
    }

    fn discard_unpromoted_enable_stage(&self, context: &str) -> Result<(), CloudBackupError> {
        warn!("{context}: discarding isolated staged master key");
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.discard_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("discard staged master key", source)
        })?;
        CloudBackupKeychain::global().delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear pending enable state", source)
        })
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
