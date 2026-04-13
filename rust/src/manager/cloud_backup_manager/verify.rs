mod integrity;
mod passkey_auth;
mod pending_completion;
mod session;
mod wrapper_repair;

use cove_cspp::CsppStore as _;
use cove_cspp::backup_data::EncryptedMasterKeyBackup;
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::{CSPP_CREDENTIAL_ID_KEY, Keychain};
use cove_device::passkey::PasskeyAccess;
use cove_util::ResultExt as _;
use tracing::{error, info, warn};

use self::passkey_auth::{PasskeyAuthOutcome, PasskeyAuthPolicy, authenticate_with_policy};
use self::session::VerificationSession;
use self::wrapper_repair::{WrapperRepairOperation, WrapperRepairStrategy};
use super::wallets::persist_enabled_cloud_backup_state;
use super::{
    BlockingCloudStep, CloudBackupDetailResult, CloudBackupError, CloudBackupStatus,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult,
    PendingVerificationCompletion, RustCloudBackupManager, VerificationFailureKind,
};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_detail_manager::{RecoveryState, VerificationState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrityDowngrade {
    Unverified,
}

impl RustCloudBackupManager {
    /// Deep verification of cloud backup integrity
    ///
    /// Checks state, runs do_deep_verify, wraps errors, persists result
    pub(crate) fn deep_verify_cloud_backup(
        &self,
        force_discoverable: bool,
    ) -> DeepVerificationResult {
        let state = self.state.read().status.clone();
        if !matches!(state, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            return DeepVerificationResult::NotEnabled;
        }

        self.clear_pending_verification_completion();
        let result = match self.do_deep_verify_cloud_backup(force_discoverable) {
            Ok(result) => result,
            Err(error) => {
                error!("Deep verification unexpected error: {error}");
                DeepVerificationResult::Failed(DeepVerificationFailure {
                    kind: VerificationFailureKind::Retry,
                    message: error.to_string(),
                    detail: None,
                })
            }
        };

        self.persist_verification_result(&result);
        result
    }

    pub(crate) fn persist_verification_result(&self, result: &DeepVerificationResult) {
        let current = RustCloudBackupManager::load_persisted_state();
        if matches!(current.status, PersistedCloudBackupStatus::Disabled) {
            return;
        }

        let mut new_state = current.clone();
        match result {
            DeepVerificationResult::Verified(_) => {
                new_state.status = PersistedCloudBackupStatus::Enabled;
                new_state.last_verified_at =
                    Some(jiff::Timestamp::now().as_second().try_into().unwrap_or(0));
            }
            DeepVerificationResult::AwaitingUploadConfirmation(_) => return,
            DeepVerificationResult::PasskeyConfirmed(_) => return,
            DeepVerificationResult::PasskeyMissing(_) => {
                new_state.status = PersistedCloudBackupStatus::PasskeyMissing;
            }
            DeepVerificationResult::UserCancelled(_) | DeepVerificationResult::Failed(_) => {
                new_state.status = PersistedCloudBackupStatus::Unverified;
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

        match current.status {
            PersistedCloudBackupStatus::Enabled | PersistedCloudBackupStatus::Unverified => {
                let Some(mut new_state) =
                    downgrade_cloud_backup_state(&current, IntegrityDowngrade::Unverified)
                else {
                    return;
                };

                new_state.last_verification_requested_at =
                    Some(jiff::Timestamp::now().as_second().try_into().unwrap_or(0));

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

    pub(crate) fn do_repair_passkey_wrapper(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        self.do_repair_passkey_wrapper_with_strategy(WrapperRepairStrategy::DiscoverOrCreate)
    }

    pub(crate) fn do_repair_passkey_wrapper_no_discovery(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        self.do_repair_passkey_wrapper_with_strategy(WrapperRepairStrategy::CreateNew)
    }

    fn do_repair_passkey_wrapper_with_strategy(
        &self,
        strategy: WrapperRepairStrategy,
    ) -> Result<(), CloudBackupError> {
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global();
        let passkey = PasskeyAccess::global();
        let namespace = self.current_namespace_id()?;

        let local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
            .ok_or_else(|| CloudBackupError::Internal("no local master key".into()))?;

        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()) {
            Ok(ids) => ids,
            Err(CloudStorageError::NotFound(_)) => Vec::new(),
            Err(error) => {
                return Err(CloudBackupError::Cloud(format!("list wallet backups: {error}")));
            }
        };

        let repair = WrapperRepairOperation::new(self, keychain, cloud, passkey, &namespace);
        repair
            .run(&local_master_key, &wallet_record_ids, strategy)
            .map_err(|error| error.into_cloud_backup_error())?;

        info!("Repaired cloud master key wrapper with repaired passkey association");
        Ok(())
    }

    pub(crate) fn finalize_passkey_repair(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RepairPasskey)?;
        let namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global();
        let wallet_count = match cloud.list_wallet_backups(namespace) {
            Ok(wallet_record_ids) => wallet_record_ids.len() as u32,
            Err(error) => {
                warn!("Repair passkey: failed to refresh wallet backups after repair: {error}");
                Database::global()
                    .cloud_backup_state
                    .get()
                    .ok()
                    .and_then(|state| state.wallet_count)
                    .unwrap_or(0)
            }
        };

        persist_enabled_cloud_backup_state(&Database::global(), wallet_count)?;
        self.set_status(CloudBackupStatus::Enabled);

        match self.refresh_cloud_backup_detail() {
            Some(CloudBackupDetailResult::Success(detail)) => {
                self.set_detail(Some(detail));
            }
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Failed to refresh detail after passkey repair: {error}");
            }
            None => {}
        }

        Ok(())
    }

    pub(crate) fn do_deep_verify_cloud_backup(
        &self,
        force_discoverable: bool,
    ) -> Result<DeepVerificationResult, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Verify)?;
        VerificationSession::new(self, force_discoverable)?.run()
    }

    pub(crate) fn recover_local_master_key_from_cloud(
        &self,
        namespace: &str,
        recovery_message: &str,
    ) -> Result<MasterKey, CloudBackupError> {
        self.recover_local_master_key_from_cloud_with_policy(
            namespace,
            recovery_message,
            PasskeyAuthPolicy::StoredThenDiscover,
        )
    }

    pub(crate) fn recover_local_master_key_from_cloud_without_discovery(
        &self,
        namespace: &str,
        recovery_message: &str,
    ) -> Result<MasterKey, CloudBackupError> {
        self.recover_local_master_key_from_cloud_with_policy(
            namespace,
            recovery_message,
            PasskeyAuthPolicy::StoredOnly,
        )
    }

    fn recover_local_master_key_from_cloud_with_policy(
        &self,
        namespace: &str,
        recovery_message: &str,
        auth_policy: PasskeyAuthPolicy,
    ) -> Result<MasterKey, CloudBackupError> {
        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let cloud = CloudStorage::global();
        let passkey = PasskeyAccess::global();

        let master_json = match cloud.download_master_key_backup(namespace.to_string()) {
            Ok(json) => json,
            Err(CloudStorageError::NotFound(_)) => {
                return Err(CloudBackupError::RecoveryRequired(recovery_message.into()));
            }
            Err(error) => {
                return Err(CloudBackupError::Cloud(format!(
                    "download master key backup: {error}",
                )));
            }
        };

        let encrypted: EncryptedMasterKeyBackup =
            serde_json::from_slice(&master_json).map_err_str(CloudBackupError::Internal)?;
        if encrypted.version != 1 {
            let version = encrypted.version;
            return Err(CloudBackupError::Internal(format!(
                "master key backup version {version} is not supported",
            )));
        }

        let authenticated =
            match authenticate_with_policy(keychain, passkey, &encrypted.prf_salt, auth_policy)? {
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

        keychain
            .save_cspp_passkey(&authenticated.credential_id, encrypted.prf_salt)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;
        cspp.save_master_key(&master_key)
            .map_err_prefix("save recovered master key", CloudBackupError::Internal)?;

        info!("Recovered local master key from cloud");
        Ok(master_key)
    }
}

pub(super) fn load_stored_credential_id(keychain: &Keychain) -> Option<Vec<u8>> {
    keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).and_then(|hex_str| {
        hex::decode(hex_str)
            .inspect_err(|error| warn!("Failed to decode stored credential_id: {error}"))
            .ok()
    })
}

fn downgrade_cloud_backup_state(
    current: &PersistedCloudBackupState,
    downgrade: IntegrityDowngrade,
) -> Option<PersistedCloudBackupState> {
    match downgrade {
        IntegrityDowngrade::Unverified => match current.status {
            PersistedCloudBackupStatus::Enabled => Some(PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Unverified,
                ..current.clone()
            }),
            PersistedCloudBackupStatus::Unverified => Some(current.clone()),
            PersistedCloudBackupStatus::PasskeyMissing | PersistedCloudBackupStatus::Disabled => {
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downgrade_state_marks_enabled_as_unverified() {
        let current = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::Enabled,
            last_sync: Some(5),
            wallet_count: Some(2),
            last_verified_at: Some(21),
            ..PersistedCloudBackupState::default()
        };

        let updated =
            downgrade_cloud_backup_state(&current, IntegrityDowngrade::Unverified).unwrap();

        assert_eq!(
            updated,
            PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Unverified,
                last_sync: Some(5),
                wallet_count: Some(2),
                last_verified_at: Some(21),
                ..PersistedCloudBackupState::default()
            }
        );
    }

    #[test]
    fn downgrade_state_keeps_passkey_missing_when_only_unverified_requested() {
        let current = PersistedCloudBackupState {
            status: PersistedCloudBackupStatus::PasskeyMissing,
            last_sync: Some(11),
            wallet_count: Some(4),
            last_verified_at: Some(22),
            ..PersistedCloudBackupState::default()
        };

        let updated = downgrade_cloud_backup_state(&current, IntegrityDowngrade::Unverified);

        assert!(updated.is_none());
    }
}
