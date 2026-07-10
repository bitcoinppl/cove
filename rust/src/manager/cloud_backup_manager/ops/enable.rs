mod recovery;
mod types;

use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use tracing::info;
use zeroize::Zeroizing;

use super::{BlockingCloudStep, RustCloudBackupManager, blocking_cloud_error};
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer,
    PasskeyMaterialOutcome, PreparedWalletBackup, StagedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableState, CloudBackupError, CloudBackupProgress,
    CloudBackupRestoreOutcome, CloudBackupStatus, CloudBackupStore, PendingEnableSession,
    PendingVerificationUpload, master_key_wrapper_revision_hash,
};

use types::EnablePasskeyAcquisition;
pub(crate) use types::{
    CloudBackupEnablePasskeyPreparation, CloudBackupEnablePasskeyRegistration,
    CloudBackupEnablePreparation, CloudBackupEnableRecoveryCompletion,
    CloudBackupEnableRecoveryPreparation, CloudBackupNoDiscoveryEnablePreparation,
    CloudBackupReadyEnableUpload, CloudBackupRegisteredEnablePasskey,
    CloudBackupSavedPasskeyConfirmation, CloudBackupUploadedEnableBackup,
    EnablePasskeyRegistrationFlow,
};

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

            NamespaceMatchOutcome::Inconclusive => {
                Err(self.offline_error_for_step(BlockingCloudStep::Enable))
            }

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
        let (master_key, context) = self.pending_enable.stage_fresh_enable_master(context)?;

        let namespace_id = master_key.namespace_id();
        info!("Enable: namespace_id={namespace_id}, getting passkey");
        let acquirer = PasskeyMaterialAcquirer::new(passkey_access);
        match acquirer.discover_or_register_for_enable().await {
            Ok(PasskeyMaterialOutcome::Authenticated(passkey)) => {
                self.pending_enable.record_pending_enable_passkey(&master_key, &passkey)?;
                Ok(CloudBackupEnablePasskeyPreparation::Ready(CloudBackupReadyEnableUpload {
                    master_key: Zeroizing::new(master_key),
                    passkey: Zeroizing::new(passkey),
                    context,
                }))
            }
            Ok(PasskeyMaterialOutcome::RegisteredForConfirmation(passkey)) => {
                self.pending_enable.record_pending_enable_staged_passkey(&master_key, &passkey)?;
                Ok(CloudBackupEnablePasskeyPreparation::Registered(
                    CloudBackupRegisteredEnablePasskey {
                        master_key: Zeroizing::new(master_key),
                        passkey: Zeroizing::new(passkey),
                        context,
                    },
                ))
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                self.pending_enable.discard_unpromoted_enable_stage(
                    "Enable cancelled before passkey setup finished",
                )?;
                Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context })
            }
            Err(error) => {
                self.pending_enable.discard_unpromoted_enable_stage(
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
        let (master_key, context) = self.pending_enable.stage_fresh_enable_master(context)?;

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
                self.pending_enable.discard_unpromoted_enable_stage(failed_context)?;
                return Err(error);
            }
            Ok(EnablePasskeyAcquisition::Ready(passkey)) => passkey,
            Ok(EnablePasskeyAcquisition::Cancelled) => {
                self.pending_enable.discard_unpromoted_enable_stage(cancelled_context)?;
                return Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context });
            }
        };

        if let Err(error) =
            self.pending_enable.record_pending_enable_staged_passkey(&master_key, &passkey)
        {
            self.pending_enable.discard_unpromoted_enable_stage(failed_context)?;
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
        self.pending_enable
            .mark_pending_enable_remote_writes_started(&ready.master_key, &ready.passkey)?;
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
}
