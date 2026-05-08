pub(crate) mod coordinator;
mod integrity;
mod passkey_auth;
mod pending_completion;
mod session;
mod wrapper_repair;

use cove_cspp::backup_data::{EncryptedMasterKeyBackup, MasterKeyBackupVersion};
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use tracing::{error, info, warn};

use self::passkey_auth::{PasskeyAuthOutcome, PasskeyAuthPolicy, PasskeyAuthenticator};
use self::session::VerificationSession;
use self::wrapper_repair::{WrapperRepairOperation, WrapperRepairStrategy};
use super::CloudBackupStore;
use super::{
    BlockingCloudStep, CloudBackupDetailResult, CloudBackupError, CloudBackupKeychain,
    CloudBackupRetryAction, CloudBackupRetryContext, CloudBackupStatus, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, PendingVerificationCompletion,
    PendingVerificationUpload, RustCloudBackupManager, is_connectivity_related_issue,
};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrityDowngrade {
    Unverified,
}

impl IntegrityDowngrade {
    fn apply_to(&self, current: &PersistedCloudBackupState) -> Option<PersistedCloudBackupState> {
        match self {
            Self::Unverified => match current.status() {
                PersistedCloudBackupStatus::Enabled => {
                    let mut state = current.clone();
                    state.mark_verification_required(state.last_verification_requested_at());
                    Some(state)
                }
                PersistedCloudBackupStatus::Unverified => Some(current.clone()),
                PersistedCloudBackupStatus::PasskeyMissing
                | PersistedCloudBackupStatus::Disabled => None,
            },
        }
    }
}

impl RustCloudBackupManager {
    /// Deep verification of cloud backup integrity
    ///
    /// Checks state, runs do_deep_verify, wraps errors, persists result
    pub(crate) async fn deep_verify_cloud_backup(
        &self,
        force_discoverable: bool,
    ) -> DeepVerificationResult {
        let state = self.state.read().status().clone();
        if !matches!(state, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            return DeepVerificationResult::NotEnabled;
        }

        self.clear_pending_verification_completion();
        let result = match self.do_deep_verify_cloud_backup(force_discoverable).await {
            Ok(result) => result,
            Err(error) => {
                error!("Deep verification unexpected error: {error}");
                let retry_context = is_connectivity_related_issue(&error).then(|| {
                    let action = if force_discoverable {
                        CloudBackupRetryAction::VerifyDiscoverable
                    } else {
                        CloudBackupRetryAction::Verify
                    };
                    CloudBackupRetryContext::connectivity(action)
                });

                DeepVerificationResult::Failed(DeepVerificationFailure::retry(
                    error.to_string(),
                    None,
                    retry_context,
                ))
            }
        };

        self.persist_verification_result(&result);
        result
    }

    pub(crate) fn persist_verification_result(&self, result: &DeepVerificationResult) {
        let current = RustCloudBackupManager::load_persisted_state();
        if matches!(current.status(), PersistedCloudBackupStatus::Disabled) {
            return;
        }

        let mut new_state = current.clone();
        match result {
            DeepVerificationResult::Verified(_) => {
                new_state
                    .mark_verified_at(jiff::Timestamp::now().as_second().try_into().unwrap_or(0));
            }
            DeepVerificationResult::AwaitingUploadConfirmation(_) => return,
            DeepVerificationResult::PasskeyConfirmed(_) => return,
            DeepVerificationResult::PasskeyMissing(_) => {
                new_state.mark_passkey_missing();
            }
            DeepVerificationResult::UserCancelled(_) | DeepVerificationResult::Failed(_) => {
                new_state.mark_verification_required(new_state.last_verification_requested_at());
            }
            DeepVerificationResult::NotEnabled => return,
        };

        if current != new_state
            && let Err(error) =
                self.persist_cloud_backup_state(&new_state, "persist verification state")
        {
            error!("Failed to persist verification state: {error}");
        }
    }

    pub(crate) fn mark_verification_required_after_wallet_change(&self) {
        let current = RustCloudBackupManager::load_persisted_state();

        match current.status() {
            PersistedCloudBackupStatus::Enabled | PersistedCloudBackupStatus::Unverified => {
                let Some(mut new_state) = IntegrityDowngrade::Unverified.apply_to(&current) else {
                    return;
                };

                new_state.mark_verification_required(Some(
                    jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
                ));

                if let Err(error) = self.persist_cloud_backup_state(
                    &new_state,
                    "mark cloud backup unverified after wallet change",
                ) {
                    error!("Failed to mark cloud backup unverified after wallet change: {error}");
                }
            }
            PersistedCloudBackupStatus::PasskeyMissing | PersistedCloudBackupStatus::Disabled => {}
        }
    }

    pub(crate) async fn do_repair_passkey_wrapper(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        self.do_repair_passkey_wrapper_with_strategy(WrapperRepairStrategy::DiscoverOrCreate).await
    }

    pub(crate) async fn do_repair_passkey_wrapper_no_discovery(
        &self,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        self.do_repair_passkey_wrapper_with_strategy(WrapperRepairStrategy::CreateNew).await
    }

    async fn do_repair_passkey_wrapper_with_strategy(
        &self,
        strategy: WrapperRepairStrategy,
    ) -> Result<(), CloudBackupError> {
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global_explicit_client();
        let passkey = PasskeyAccess::global();
        let namespace = self.current_namespace_id()?;
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());

        let local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .ok_or_else(|| CloudBackupError::Internal("no local master key".into()))?;

        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
            Ok(ids) => ids,
            Err(CloudStorageError::NotFound(_)) => Vec::new(),
            Err(error) => {
                return Err(CloudBackupError::cloud_storage_context("list wallet backups", error));
            }
        };

        let repair =
            WrapperRepairOperation::new(self, &cloud_keychain, &cloud, passkey, &namespace);
        repair
            .run(&local_master_key, &wallet_record_ids, strategy)
            .await
            .map_err(CloudBackupError::from)?;

        self.replace_pending_verification_completion(PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: true,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            namespace,
            vec![PendingVerificationUpload::master_key_wrapper()],
        ));

        info!("Repaired cloud master key wrapper with repaired passkey association");
        Ok(())
    }

    pub(crate) async fn finalize_passkey_repair(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let wallet_count = match cloud.list_wallet_backups(namespace).await {
            Ok(wallet_record_ids) => wallet_record_ids.len() as u32,
            Err(error) => {
                warn!("Repair passkey: failed to refresh wallet backups after repair: {error}");
                Database::global()
                    .cloud_backup_state
                    .get()
                    .ok()
                    .and_then(|state| state.wallet_count())
                    .unwrap_or(0)
            }
        };

        CloudBackupStore::global().persist_enabled(wallet_count)?;
        self.set_status(CloudBackupStatus::Enabled);

        match self.refresh_cloud_backup_detail().await {
            Some(CloudBackupDetailResult::Success(detail)) => {
                self.set_detail(Some(detail));
            }
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Failed to refresh detail after passkey repair: {error}");
            }
            None => {}
        }

        self.refresh_sync_health();
        Ok(())
    }

    pub(crate) async fn do_deep_verify_cloud_backup(
        &self,
        force_discoverable: bool,
    ) -> Result<DeepVerificationResult, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Verify)?;
        VerificationSession::new(self, force_discoverable)?.run().await
    }

    pub(crate) async fn recover_local_master_key_from_cloud(
        &self,
        namespace: &str,
        recovery_message: &str,
    ) -> Result<MasterKey, CloudBackupError> {
        self.recover_local_master_key_from_cloud_with_policy(
            namespace,
            recovery_message,
            PasskeyAuthPolicy::StoredThenDiscover,
            CloudStorage::global_explicit_client(),
        )
        .await
    }

    pub(crate) async fn recover_local_master_key_from_cloud_without_discovery(
        &self,
        namespace: &str,
        recovery_message: &str,
    ) -> Result<MasterKey, CloudBackupError> {
        self.recover_local_master_key_from_cloud_with_policy(
            namespace,
            recovery_message,
            PasskeyAuthPolicy::StoredOnly,
            CloudStorage::global_silent_client(),
        )
        .await
    }

    async fn recover_local_master_key_from_cloud_with_policy(
        &self,
        namespace: &str,
        recovery_message: &str,
        auth_policy: PasskeyAuthPolicy,
        cloud: cove_device::cloud_storage::CloudStorageClient,
    ) -> Result<MasterKey, CloudBackupError> {
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let passkey = PasskeyAccess::global();

        let master_json = match cloud.download_master_key_backup(namespace.to_string()).await {
            Ok(json) => json,
            Err(CloudStorageError::NotFound(_)) => {
                return Err(CloudBackupError::RecoveryRequired(recovery_message.into()));
            }
            Err(error) => {
                return Err(CloudBackupError::cloud_storage_context(
                    "download master key backup",
                    error,
                ));
            }
        };

        let encrypted: EncryptedMasterKeyBackup =
            serde_json::from_slice(&master_json).map_err_str(CloudBackupError::Internal)?;
        match encrypted.backup_version() {
            Ok(MasterKeyBackupVersion::V1) => {}
            Err(unsupported) => {
                let version = unsupported.0;
                return Err(CloudBackupError::Compatibility(format!(
                    "master key backup version {version} is not supported",
                )));
            }
        }

        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let authenticator = PasskeyAuthenticator::new(&cloud_keychain, passkey);
        let auth_outcome =
            authenticator.authenticate_with_policy(&encrypted.prf_salt, auth_policy).await?;
        let authenticated = match auth_outcome {
            PasskeyAuthOutcome::Authenticated(result) => result,
            PasskeyAuthOutcome::UserCancelled => {
                return Err(CloudBackupError::Passkey("user cancelled".into()));
            }
            PasskeyAuthOutcome::NoCredentialFound => {
                return Err(CloudBackupError::RecoveryRequired(recovery_message.into()));
            }
        };

        let master_key = master_key_crypto::decrypt_master_key(&encrypted, &authenticated.prf_key)
            .map_err(|_| match auth_policy {
                PasskeyAuthPolicy::StoredOnly => {
                    CloudBackupError::RecoveryRequired(recovery_message.into())
                }
                PasskeyAuthPolicy::StoredThenDiscover | PasskeyAuthPolicy::DiscoverOnly => {
                    CloudBackupError::Passkey(
                        "selected passkey didn't unlock this cloud backup".into(),
                    )
                }
            })?;

        CloudBackupKeychain::new(keychain.clone())
            .save_passkey(&authenticated.credential_id, encrypted.prf_salt)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;
        cspp.save_master_key(&master_key)
            .map_err_prefix("save recovered master key", CloudBackupError::Internal)?;

        info!("Recovered local master key from cloud");
        Ok(master_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
        PersistedPasskeyState,
    };

    fn configured_state(
        passkey: PersistedPasskeyState,
        verification: PersistedBackupVerificationState,
        last_sync: Option<u64>,
        wallet_count: Option<u32>,
    ) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey,
            verification,
            sync: PersistedBackupSyncState { last_sync, wallet_count },
            pending_verification_completion: None,
        })
    }

    #[test]
    fn downgrade_state_marks_enabled_as_unverified() {
        let current = configured_state(
            PersistedPasskeyState::Available,
            PersistedBackupVerificationState::Verified {
                last_verified_at: 21,
                requested_at: None,
                dismissed_at: None,
            },
            Some(5),
            Some(2),
        );

        let updated = IntegrityDowngrade::Unverified.apply_to(&current).unwrap();

        assert_eq!(
            updated,
            configured_state(
                PersistedPasskeyState::Available,
                PersistedBackupVerificationState::Required {
                    last_verified_at: Some(21),
                    requested_at: None,
                    dismissed_at: None,
                },
                Some(5),
                Some(2),
            )
        );
    }

    #[test]
    fn downgrade_state_keeps_passkey_missing_when_only_unverified_requested() {
        let current = configured_state(
            PersistedPasskeyState::Missing,
            PersistedBackupVerificationState::Verified {
                last_verified_at: 22,
                requested_at: None,
                dismissed_at: None,
            },
            Some(11),
            Some(4),
        );

        let updated = IntegrityDowngrade::Unverified.apply_to(&current);

        assert!(updated.is_none());
    }
}
