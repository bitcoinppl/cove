use cove_cspp::backup_data::EncryptedMasterKeyBackup;
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_device::passkey::{PasskeyAccess, PasskeyCredentialPresence};
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::super::{
    CloudBackupDetail, CloudBackupError, DeepVerificationFailure, DeepVerificationReport,
    DeepVerificationResult, PASSKEY_RP_ID, PendingVerificationCompletion,
    PendingVerificationUpload, RustCloudBackupManager, VerificationFailureKind,
    cloud_inventory::CloudWalletInventory,
    wallets::{WalletBackupLookup, WalletBackupReader, prepare_wallet_backup},
};
use super::load_stored_credential_id;
use super::passkey_auth::PasskeyAuthOutcome;
use super::wrapper_repair::{WrapperRepairError, WrapperRepairOperation, WrapperRepairStrategy};
use crate::manager::cloud_backup_manager::pending::remote_wallet_revision_matches;

const RECREATE_WARNING: &str = "Recreating from this device will remove references to wallets that only exist in the cloud backup";
const REINITIALIZE_WARNING: &str = "This will replace your entire cloud backup set. Wallets that only exist in the cloud backup will be lost";

enum EncryptedMasterKeyStep {
    Loaded(EncryptedMasterKeyBackup),
    Missing,
    Finished(DeepVerificationResult),
}

enum MasterKeyResolution {
    VerifiedCloudMasterKey(MasterKey),
    NeedsWrapperRepair { reuse_credential_id: Option<Vec<u8>> },
    Finished(DeepVerificationResult),
}

pub(crate) struct VerificationSession {
    pub(crate) manager: RustCloudBackupManager,
    pub(crate) keychain: Keychain,
    pub(crate) cspp: cove_cspp::Cspp<Keychain>,
    pub(crate) cloud: CloudStorage,
    pub(crate) passkey: PasskeyAccess,
    pub(crate) namespace: String,
    pub(crate) report: DeepVerificationReport,
    pub(crate) local_master_key: Option<MasterKey>,
    pub(crate) wallet_record_ids: Option<Vec<String>>,
    pub(crate) wallets_missing: bool,
    pub(crate) force_discoverable: bool,
}

impl VerificationSession {
    pub(crate) fn new(
        manager: &RustCloudBackupManager,
        force_discoverable: bool,
    ) -> Result<Self, CloudBackupError> {
        let keychain = Keychain::global().clone();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?;

        Ok(Self {
            manager: manager.clone(),
            keychain,
            cspp,
            cloud: CloudStorage::global().clone(),
            passkey: PasskeyAccess::global().clone(),
            namespace: manager.current_namespace_id()?,
            report: DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            local_master_key,
            wallet_record_ids: None,
            wallets_missing: false,
            force_discoverable,
        })
    }

    pub(crate) async fn run(mut self) -> Result<DeepVerificationResult, CloudBackupError> {
        if let Some(result) = self.load_wallet_inventory().await {
            return Ok(result);
        }

        let encrypted_master = match self.load_encrypted_master_key().await? {
            EncryptedMasterKeyStep::Loaded(encrypted_master) => Some(encrypted_master),
            EncryptedMasterKeyStep::Missing => None,
            EncryptedMasterKeyStep::Finished(result) => return Ok(result),
        };

        let master_key = match self.resolve_master_key_step(encrypted_master.as_ref()).await? {
            MasterKeyResolution::VerifiedCloudMasterKey(master_key) => {
                let master_key = self.apply_verified_cloud_master_key(master_key)?;
                if self.wallets_missing {
                    return Ok(self.recreate_manifest_result());
                }
                master_key
            }
            MasterKeyResolution::NeedsWrapperRepair { reuse_credential_id } => {
                let master_key =
                    match self.repair_wrapper_from_local_key(reuse_credential_id).await? {
                        MasterKeyResolution::VerifiedCloudMasterKey(master_key) => master_key,
                        MasterKeyResolution::Finished(result) => return Ok(result),
                        MasterKeyResolution::NeedsWrapperRepair { .. } => {
                            unreachable!("wrapper repair must resolve master key")
                        }
                    };

                if self.wallets_missing {
                    return Ok(self.recreate_manifest_result());
                }

                master_key
            }
            MasterKeyResolution::Finished(result) => return Ok(result),
        };

        if let Some(result) = self.verify_wallet_backups_and_autosync(&master_key).await {
            return Ok(result);
        }

        Ok(self.finish_verified())
    }

    async fn load_wallet_inventory(&mut self) -> Option<DeepVerificationResult> {
        match self.cloud.list_wallet_backups(self.namespace.clone()).await {
            Ok(ids) => {
                let remote_wallet_truth = match self.manager.load_remote_wallet_truth(&ids).await {
                    Ok(remote_wallet_truth) => remote_wallet_truth,
                    Err(error) => return Some(self.remote_truth_retry_result(&error)),
                };
                self.manager.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

                let detail = match self
                    .manager
                    .build_cloud_backup_detail_with_remote_truth(&ids, remote_wallet_truth)
                    .await
                {
                    Ok(detail) => detail,
                    Err(error) => return Some(self.local_inventory_retry_result(&error)),
                };

                self.report.detail = Some(detail);
                self.wallet_record_ids = Some(ids);
                None
            }
            Err(CloudStorageError::NotFound(_)) => {
                self.wallets_missing = true;
                self.wallet_record_ids = None;
                None
            }
            Err(error) => {
                Some(self.retry_result(format!("failed to list wallet backups: {error}")))
            }
        }
    }

    async fn load_encrypted_master_key(&self) -> Result<EncryptedMasterKeyStep, CloudBackupError> {
        match self.cloud.download_master_key_backup(self.namespace.clone()).await {
            Ok(json) => {
                let encrypted: EncryptedMasterKeyBackup =
                    serde_json::from_slice(&json).map_err_str(CloudBackupError::Internal)?;

                if encrypted.version != 1 {
                    return Ok(EncryptedMasterKeyStep::Finished(DeepVerificationResult::Failed(
                        DeepVerificationFailure {
                            kind: VerificationFailureKind::UnsupportedVersion,
                            message: format!(
                                "master key backup version {} is not supported",
                                encrypted.version
                            ),
                            detail: self.detail(),
                        },
                    )));
                }

                Ok(EncryptedMasterKeyStep::Loaded(encrypted))
            }
            Err(CloudStorageError::NotFound(_)) => {
                if self.local_master_key.is_some() {
                    return Ok(EncryptedMasterKeyStep::Missing);
                }

                Ok(EncryptedMasterKeyStep::Finished(
                    self.reinitialize_result(
                        "master key backup not found in iCloud and no local key",
                    ),
                ))
            }
            Err(error) => Ok(EncryptedMasterKeyStep::Finished(
                self.retry_result(format!("failed to download master key backup: {error}")),
            )),
        }
    }

    async fn resolve_master_key_step(
        &mut self,
        encrypted_master: Option<&EncryptedMasterKeyBackup>,
    ) -> Result<MasterKeyResolution, CloudBackupError> {
        let Some(encrypted_master) = encrypted_master else {
            return Ok(MasterKeyResolution::NeedsWrapperRepair { reuse_credential_id: None });
        };

        let prf_salt = encrypted_master.prf_salt;
        let authenticated = match self.authenticate_with_fallback(&prf_salt).await? {
            PasskeyAuthOutcome::Authenticated(result) => result,
            PasskeyAuthOutcome::UserCancelled => {
                return Ok(MasterKeyResolution::Finished(self.resolve_cancellation_outcome()));
            }
            PasskeyAuthOutcome::NoCredentialFound => {
                if self.local_master_key.is_some() {
                    return Ok(MasterKeyResolution::NeedsWrapperRepair {
                        reuse_credential_id: None,
                    });
                }

                return Ok(MasterKeyResolution::Finished(
                    self.reinitialize_result("no passkey found and no local master key"),
                ));
            }
        };

        self.report.credential_recovered = authenticated.credential_recovered;

        match master_key_crypto::decrypt_master_key(encrypted_master, &authenticated.prf_key) {
            Ok(master_key) => {
                if let Err(error) =
                    self.keychain.save_cspp_passkey(&authenticated.credential_id, prf_salt)
                {
                    return Ok(MasterKeyResolution::Finished(
                        self.retry_result(format!("save cspp credentials: {error}")),
                    ));
                }

                Ok(MasterKeyResolution::VerifiedCloudMasterKey(master_key))
            }
            Err(_) if self.local_master_key.is_some() => {
                Ok(MasterKeyResolution::NeedsWrapperRepair {
                    reuse_credential_id: Some(authenticated.credential_id),
                })
            }
            Err(_) => Ok(MasterKeyResolution::Finished(self.reinitialize_result(
                "could not decrypt cloud master key and no local key available",
            ))),
        }
    }

    fn apply_verified_cloud_master_key(
        &mut self,
        master_key: MasterKey,
    ) -> Result<MasterKey, CloudBackupError> {
        match &self.local_master_key {
            None => {
                self.cspp
                    .save_master_key(&master_key)
                    .map_err_prefix("repair local master key", CloudBackupError::Internal)?;
                self.report.local_master_key_repaired = true;
                info!("Repaired local master key from cloud");
            }
            Some(local_key) if local_key.as_bytes() != master_key.as_bytes() => {
                self.cspp
                    .save_master_key(&master_key)
                    .map_err_prefix("repair local master key", CloudBackupError::Internal)?;
                self.report.local_master_key_repaired = true;
                info!("Repaired local master key to match cloud");
            }
            Some(_) => {}
        }

        Ok(master_key)
    }

    async fn repair_wrapper_from_local_key(
        &mut self,
        reuse_credential_id: Option<Vec<u8>>,
    ) -> Result<MasterKeyResolution, CloudBackupError> {
        let Some(local_master_key) = self.local_master_key.as_ref() else {
            return Ok(MasterKeyResolution::Finished(
                self.reinitialize_result("no local master key available for wrapper repair"),
            ));
        };

        let repair = WrapperRepairOperation::new(
            &self.manager,
            &self.keychain,
            &self.cloud,
            &self.passkey,
            &self.namespace,
        );
        let strategy = match reuse_credential_id {
            Some(credential_id) => WrapperRepairStrategy::ReuseExisting(credential_id),
            None => WrapperRepairStrategy::CreateNew,
        };
        let wallet_record_ids = self.wallet_record_ids.as_deref().unwrap_or(&[]);

        match repair.run(local_master_key, wallet_record_ids, strategy).await {
            Ok(()) => {
                self.report.master_key_wrapper_repaired = true;
                info!("Repaired cloud master key wrapper");
                Ok(MasterKeyResolution::VerifiedCloudMasterKey(MasterKey::from_bytes(
                    *local_master_key.as_bytes(),
                )))
            }
            Err(WrapperRepairError::WrongKey) => {
                Ok(MasterKeyResolution::Finished(self.reinitialize_result(
                    "local master key cannot decrypt existing cloud wallet backups",
                )))
            }
            Err(WrapperRepairError::Inconclusive) => Ok(MasterKeyResolution::Finished(
                self.retry_result("could not download any wallet to verify local key"),
            )),
            Err(WrapperRepairError::Operation(error)) => Err(error),
        }
    }

    async fn verify_wallet_backups_and_autosync(
        &mut self,
        master_key: &MasterKey,
    ) -> Option<DeepVerificationResult> {
        let wallet_record_ids = self.wallet_record_ids.clone()?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let (verified, failed, unsupported) = self.verify_wallet_backups(&critical_key).await;
        self.report.wallets_verified = verified;
        self.report.wallets_failed = failed;
        self.report.wallets_unsupported = unsupported;
        let remote_wallet_truth =
            match self.manager.load_remote_wallet_truth(&wallet_record_ids).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => return Some(self.remote_truth_retry_result(&error)),
            };

        let unsynced = match CloudWalletInventory::load_with_remote_truth(
            &wallet_record_ids,
            remote_wallet_truth,
        )
        .await
        {
            Ok(inventory) => {
                let detail = inventory.build_detail();
                self.report.detail = Some(detail.clone());
                if inventory.has_unknown_remote_wallets() {
                    return Some(
                        self.retry_result("failed to refresh remote wallet truth for some wallets"),
                    );
                }

                inventory.upload_candidate_wallets()
            }
            Err(error) => return Some(self.local_inventory_retry_result(&error)),
        };

        if unsynced.is_empty() {
            return None;
        }

        let count = unsynced.len() as u32;
        info!("Deep verify: {count} local wallet(s) not in cloud, auto-syncing");
        if let Err(error) = self.manager.do_backup_wallets(&unsynced).await {
            warn!("Deep verify: auto-sync failed: {error}");
            return Some(
                self.retry_result(format!("failed to auto-sync missing wallet backups: {error}")),
            );
        }
        let mut pending_uploads = Vec::with_capacity(unsynced.len());
        for wallet in &unsynced {
            let prepared = match prepare_wallet_backup(wallet, wallet.wallet_mode).await {
                Ok(prepared) => prepared,
                Err(error) => return Some(self.local_inventory_retry_result(&error)),
            };
            pending_uploads
                .push(PendingVerificationUpload::new(prepared.record_id, prepared.revision_hash));
        }

        let updated_ids = match self.cloud.list_wallet_backups(self.namespace.clone()).await {
            Ok(updated_ids) => updated_ids,
            Err(error) => {
                warn!("Deep verify: failed to re-check wallet backups after auto-sync: {error}");
                return Some(self.retry_result(format!(
                    "failed to re-check wallet backups after auto-sync: {error}"
                )));
            }
        };

        let remote_wallet_truth = match self.manager.load_remote_wallet_truth(&updated_ids).await {
            Ok(remote_wallet_truth) => remote_wallet_truth,
            Err(error) => return Some(self.remote_truth_retry_result(&error)),
        };
        self.manager.cleanup_confirmed_pending_blobs(&remote_wallet_truth);
        let unconfirmed_pending_uploads = pending_uploads
            .iter()
            .filter(|upload| {
                !remote_wallet_revision_matches(
                    &remote_wallet_truth,
                    upload.record_id(),
                    upload.expected_revision(),
                )
            })
            .count();
        let inventory =
            match CloudWalletInventory::load_with_remote_truth(&updated_ids, remote_wallet_truth)
                .await
            {
                Ok(inventory) => inventory,
                Err(error) => return Some(self.local_inventory_retry_result(&error)),
            };
        let listed: std::collections::HashSet<_> = updated_ids.iter().cloned().collect();

        let remaining_unsynced = inventory.upload_candidate_wallets();
        self.report.detail = Some(inventory.build_detail());
        self.wallet_record_ids = Some(updated_ids);
        if inventory.has_unknown_remote_wallets() {
            return Some(
                self.retry_result("failed to refresh remote wallet truth for some wallets"),
            );
        }
        let missing_listed_uploads =
            pending_uploads.iter().any(|upload| !listed.contains(upload.record_id()));

        if remaining_unsynced.is_empty()
            && !missing_listed_uploads
            && unconfirmed_pending_uploads == 0
        {
            return None;
        }

        let remaining_count = remaining_unsynced.len();
        let missing_count =
            pending_uploads.iter().filter(|upload| !listed.contains(upload.record_id())).count();
        let stale_count = unconfirmed_pending_uploads.saturating_sub(missing_count);
        warn!(
            "Deep verify: auto-sync finished but confirmation is still pending missing_listed={missing_count} stale={stale_count} stale_or_unsynced={remaining_count}"
        );
        self.manager.replace_pending_verification_completion(PendingVerificationCompletion::new(
            self.report.clone(),
            self.namespace.clone(),
            pending_uploads,
        ));

        Some(DeepVerificationResult::AwaitingUploadConfirmation(self.report.clone()))
    }

    async fn verify_wallet_backups(&self, critical_key: &[u8; 32]) -> (u32, u32, u32) {
        let Some(wallet_record_ids) = self.wallet_record_ids.as_ref() else {
            return (0, 0, 0);
        };
        let reader = WalletBackupReader::new(
            self.cloud.clone(),
            self.namespace.clone(),
            Zeroizing::new(*critical_key),
        );

        let mut verified = 0u32;
        let mut failed = 0u32;
        let mut unsupported = 0u32;

        for record_id in wallet_record_ids {
            match reader.lookup_entry(record_id).await {
                Ok(WalletBackupLookup::Found(_)) => verified += 1,
                Ok(WalletBackupLookup::UnsupportedVersion(_)) => unsupported += 1,
                Ok(WalletBackupLookup::NotFound) => {
                    warn!("Verify: failed to download wallet {record_id}: not found");
                    failed += 1;
                }
                Err(error) => {
                    warn!("Verify: failed to download wallet {record_id}: {error}");
                    failed += 1;
                }
            }
        }

        (verified, failed, unsupported)
    }

    fn finish_verified(self) -> DeepVerificationResult {
        DeepVerificationResult::Verified(self.report)
    }

    fn detail(&self) -> Option<CloudBackupDetail> {
        self.report.detail.clone()
    }

    fn local_inventory_retry_result(&self, error: &CloudBackupError) -> DeepVerificationResult {
        self.retry_result(format!("failed to load local wallet inventory: {error}"))
    }

    fn remote_truth_retry_result(&self, error: &CloudBackupError) -> DeepVerificationResult {
        self.retry_result(format!("failed to refresh remote wallet truth: {error}"))
    }

    fn retry_result(&self, message: impl Into<String>) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure {
            kind: VerificationFailureKind::Retry,
            message: message.into(),
            detail: self.detail(),
        })
    }

    fn recreate_manifest_result(&self) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure {
            kind: VerificationFailureKind::RecreateManifest { warning: RECREATE_WARNING.into() },
            message: "wallet backups not found in iCloud namespace".into(),
            detail: self.detail(),
        })
    }

    fn reinitialize_result(&self, message: impl Into<String>) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure {
            kind: VerificationFailureKind::ReinitializeBackup {
                warning: REINITIALIZE_WARNING.into(),
            },
            message: message.into(),
            detail: self.detail(),
        })
    }

    /// When the user cancels the discoverable passkey picker, check if the
    /// stored credential still exists. If it does the backup is healthy and
    /// we avoid downgrading persisted state. If the credential is gone the
    /// passkey is durably missing and the user needs repair
    fn resolve_cancellation_outcome(&self) -> DeepVerificationResult {
        if let Some(credential_id) = load_stored_credential_id(&self.keychain) {
            match self.passkey.check_passkey_presence(PASSKEY_RP_ID.to_string(), credential_id) {
                PasskeyCredentialPresence::Present => {
                    info!("Passkey picker cancelled but stored credential still exists");
                    cancellation_outcome(PasskeyCredentialPresence::Present, self.detail())
                }
                PasskeyCredentialPresence::Missing => {
                    info!("Passkey picker cancelled and stored credential is missing");
                    self.keychain.clear_cspp_passkey();
                    cancellation_outcome(PasskeyCredentialPresence::Missing, self.detail())
                }
                PasskeyCredentialPresence::Indeterminate => {
                    info!(
                        "Passkey picker cancelled and stored credential could not be revalidated"
                    );
                    cancellation_outcome(PasskeyCredentialPresence::Indeterminate, self.detail())
                }
            }
        } else {
            info!("Passkey picker cancelled and no stored credential found");
            DeepVerificationResult::PasskeyMissing(self.detail())
        }
    }
}

fn cancellation_outcome(
    presence: PasskeyCredentialPresence,
    detail: Option<CloudBackupDetail>,
) -> DeepVerificationResult {
    match presence {
        PasskeyCredentialPresence::Present => DeepVerificationResult::PasskeyConfirmed(detail),
        PasskeyCredentialPresence::Missing => DeepVerificationResult::PasskeyMissing(detail),
        PasskeyCredentialPresence::Indeterminate => DeepVerificationResult::UserCancelled(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_outcome_confirms_present_passkey() {
        let result = cancellation_outcome(PasskeyCredentialPresence::Present, None);
        assert!(matches!(result, DeepVerificationResult::PasskeyConfirmed(None)));
    }

    #[test]
    fn cancellation_outcome_marks_missing_passkey() {
        let result = cancellation_outcome(PasskeyCredentialPresence::Missing, None);
        assert!(matches!(result, DeepVerificationResult::PasskeyMissing(None)));
    }

    #[test]
    fn cancellation_outcome_treats_indeterminate_as_user_cancelled() {
        let result = cancellation_outcome(PasskeyCredentialPresence::Indeterminate, None);
        assert!(matches!(result, DeepVerificationResult::UserCancelled(None)));
    }
}
