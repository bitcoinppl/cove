//! Deep Cloud Backup verification session
//!
//! A verification session proves that the master key, passkey wrapper, wallet
//! blobs, and pending uploads agree. Some outcomes prepare follow-up work for
//! the supervisor, which keeps the continuation tied to the active operation

use cove_cspp::backup_data::{EncryptedMasterKeyBackup, MasterKeyBackupVersion};
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_device::passkey::{PasskeyAccess, PasskeyCredentialPresence};
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::passkey_auth::PasskeyAuthOutcome;
use super::wrapper_repair::{
    CloudBackupPasskeyWrapperRepairUpload, CloudBackupPreparedPasskeyWrapperRepair,
    WrapperRepairError, WrapperRepairOperation, WrapperRepairStrategy,
};
use crate::manager::cloud_backup_manager::pending::remote_wallet_revision_matches;
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupDetail, CloudBackupError, CloudBackupKeychain,
    CloudBackupRetryAction, CloudBackupRetryContext, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, PASSKEY_RP_ID, PendingVerificationCompletion,
    PendingVerificationUpload, RustCloudBackupManager,
    actors::CloudBackupUploadedWallet,
    blocking_cloud_error,
    cloud_inventory::CloudWalletInventory,
    is_connectivity_related_issue, offline_error_for_step,
    wallets::{WalletBackupLookup, WalletBackupReader},
};
use crate::wallet::metadata::WalletMetadata;

const RECREATE_WARNING: &str = "Recreating from this device will remove references to wallets that only exist in the cloud backup";
const REINITIALIZE_WARNING: &str = "This will replace your entire cloud backup set. Wallets that only exist in the cloud backup will be lost";

enum EncryptedMasterKeyStep {
    Loaded(EncryptedMasterKeyBackup),
    Missing,
    Finished(DeepVerificationResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterKeyAuthorizationSource {
    CloudMasterKeyWrapper,
    RepairedCloudWrapper,
}

struct AuthenticatedMasterKey {
    master_key: MasterKey,
    source: MasterKeyAuthorizationSource,
}

impl AuthenticatedMasterKey {
    fn new(master_key: MasterKey, source: MasterKeyAuthorizationSource) -> Self {
        Self { master_key, source }
    }
}

enum MasterKeyResolution {
    Continue(ContinuableMasterKeyResolution),
    Finished(DeepVerificationResult),
}

enum ContinuableMasterKeyResolution {
    Authenticated(AuthenticatedMasterKey),
    NeedsWrapperRepair { reuse_credential_id: Option<Vec<u8>> },
}

enum RepairedMasterKeyResolution {
    PreparedWrapperRepair(Box<CloudBackupPreparedDeepVerificationWrapperRepair>),
    Finished(DeepVerificationResult),
}

/// Result of a verification pass, including supervisor-owned continuations
pub(crate) enum CloudBackupDeepVerificationStep {
    Complete(DeepVerificationResult),
    PreparedWrapperRepair(Box<CloudBackupPreparedDeepVerificationWrapperRepair>),
    PreparedAutoSync(Box<CloudBackupPreparedDeepVerificationAutoSync>),
}

/// Prepared passkey-wrapper repair that still belongs to the verification run
pub(crate) struct CloudBackupPreparedDeepVerificationWrapperRepair {
    session: VerificationSession,
    preparation: CloudBackupPreparedPasskeyWrapperRepair,
    authenticated_master_key: MasterKey,
}

/// Resume token after wrapper repair upload proves the master key again
pub(crate) struct CloudBackupPendingDeepVerificationResume {
    session: VerificationSession,
    authenticated_master_key: MasterKey,
}

/// Prepared auto-sync upload for missing or stale wallet blobs
pub(crate) struct CloudBackupPreparedDeepVerificationAutoSync {
    session: VerificationSession,
    upload: CloudBackupDeepVerificationAutoSyncUpload,
}

/// Resume token after auto-sync upload and pending confirmation handling
pub(crate) struct CloudBackupPendingDeepVerificationAutoSyncResume {
    session: VerificationSession,
}

/// Wallet upload payload produced by deep verification auto-sync
pub(crate) struct CloudBackupDeepVerificationAutoSyncUpload {
    namespace_id: String,
    wallets: Vec<WalletMetadata>,
    master_key: MasterKey,
}

/// Uploaded wallet set returned after deep verification auto-sync
#[derive(Debug, Clone)]
pub(crate) struct CloudBackupUploadedDeepVerificationAutoSync {
    namespace_id: String,
    uploaded_wallets: Vec<CloudBackupUploadedWallet>,
}

/// Final auto-sync verification result and optional pending upload confirmation
pub(crate) struct CloudBackupDeepVerificationAutoSyncCompletion {
    result: DeepVerificationResult,
    pending_completion: Option<PendingVerificationCompletion>,
}

impl CloudBackupPreparedDeepVerificationWrapperRepair {
    pub(crate) fn namespace_id(&self) -> &str {
        &self.preparation.namespace_id
    }

    pub(crate) fn credential_id(&self) -> &[u8] {
        &self.preparation.credential_id
    }

    pub(crate) fn prf_salt(&self) -> [u8; 32] {
        self.preparation.prf_salt
    }

    pub(crate) fn into_parts(
        self,
    ) -> (CloudBackupPendingDeepVerificationResume, CloudBackupPasskeyWrapperRepairUpload) {
        let resume = CloudBackupPendingDeepVerificationResume {
            session: self.session,
            authenticated_master_key: self.authenticated_master_key,
        };
        let upload = self.preparation.into_upload();
        (resume, upload)
    }
}

impl CloudBackupPendingDeepVerificationResume {
    pub(crate) async fn resume(self) -> Result<CloudBackupDeepVerificationStep, CloudBackupError> {
        self.session.resume_after_wrapper_repair(self.authenticated_master_key).await
    }
}

impl CloudBackupPreparedDeepVerificationAutoSync {
    pub(crate) fn into_parts(
        self,
    ) -> (CloudBackupPendingDeepVerificationAutoSyncResume, CloudBackupDeepVerificationAutoSyncUpload)
    {
        let resume = CloudBackupPendingDeepVerificationAutoSyncResume { session: self.session };
        (resume, self.upload)
    }
}

impl CloudBackupPendingDeepVerificationAutoSyncResume {
    pub(crate) fn upload_error_result(&self, error: &CloudBackupError) -> DeepVerificationResult {
        self.session.cloud_backup_retry_result("failed to auto-sync missing wallet backups", error)
    }

    pub(crate) async fn resume(
        self,
        uploaded: CloudBackupUploadedDeepVerificationAutoSync,
    ) -> CloudBackupDeepVerificationAutoSyncCompletion {
        self.session.resume_after_auto_sync(uploaded).await
    }
}

impl CloudBackupDeepVerificationAutoSyncUpload {
    pub(crate) fn into_parts(self) -> (String, Vec<WalletMetadata>, MasterKey) {
        (self.namespace_id, self.wallets, self.master_key)
    }
}

impl CloudBackupUploadedDeepVerificationAutoSync {
    pub(crate) fn new(
        namespace_id: String,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
    ) -> Self {
        Self { namespace_id, uploaded_wallets }
    }

    pub(crate) fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    pub(crate) fn uploaded_wallets(&self) -> &[CloudBackupUploadedWallet] {
        &self.uploaded_wallets
    }
}

impl CloudBackupDeepVerificationAutoSyncCompletion {
    pub(crate) fn complete(result: DeepVerificationResult) -> Self {
        Self { result, pending_completion: None }
    }

    fn awaiting_upload_confirmation(
        result: DeepVerificationResult,
        pending_completion: PendingVerificationCompletion,
    ) -> Self {
        Self { result, pending_completion: Some(pending_completion) }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (DeepVerificationResult, Option<PendingVerificationCompletion>) {
        (self.result, self.pending_completion)
    }
}

/// One deep verification run with its cloud and keychain dependencies
pub(crate) struct VerificationSession {
    pub(crate) manager: RustCloudBackupManager,
    pub(crate) cloud_keychain: CloudBackupKeychain,
    pub(crate) cspp: cove_cspp::Cspp<Keychain>,
    pub(crate) cloud: CloudStorageClient,
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
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        let local_master_key = cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?;

        Ok(Self {
            manager: manager.clone(),
            cloud_keychain,
            cspp,
            cloud: CloudStorage::global_explicit_client(),
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

    pub(crate) async fn run(mut self) -> Result<CloudBackupDeepVerificationStep, CloudBackupError> {
        let encrypted_master = match self.load_encrypted_master_key().await? {
            EncryptedMasterKeyStep::Loaded(encrypted_master) => Some(encrypted_master),
            EncryptedMasterKeyStep::Missing => None,
            EncryptedMasterKeyStep::Finished(result) => {
                return Ok(CloudBackupDeepVerificationStep::Complete(result));
            }
        };

        let master_key_resolution = self.resolve_master_key_step(encrypted_master.as_ref()).await?;
        let master_key_resolution = match master_key_resolution {
            MasterKeyResolution::Finished(result) => {
                return Ok(CloudBackupDeepVerificationStep::Complete(result));
            }
            MasterKeyResolution::Continue(resolution) => resolution,
        };

        if let ContinuableMasterKeyResolution::Authenticated(authenticated_master) =
            &master_key_resolution
        {
            self.apply_verified_cloud_master_key(&authenticated_master.master_key)?;
        }

        if let Some(result) = self.ensure_wallet_inventory_or_short_circuit().await {
            return Ok(CloudBackupDeepVerificationStep::Complete(result));
        }

        let authenticated_master = match master_key_resolution {
            ContinuableMasterKeyResolution::Authenticated(authenticated_master) => {
                authenticated_master
            }

            ContinuableMasterKeyResolution::NeedsWrapperRepair { reuse_credential_id } => {
                match self.repair_wrapper_from_local_key(reuse_credential_id).await? {
                    RepairedMasterKeyResolution::PreparedWrapperRepair(prepared) => {
                        return Ok(CloudBackupDeepVerificationStep::PreparedWrapperRepair(
                            prepared,
                        ));
                    }
                    RepairedMasterKeyResolution::Finished(result) => {
                        return Ok(CloudBackupDeepVerificationStep::Complete(result));
                    }
                }
            }
        };

        Ok(self.verify_wallet_backups_and_autosync(authenticated_master).await)
    }

    async fn load_wallet_inventory(&mut self) -> Option<DeepVerificationResult> {
        match self.cloud.list_wallet_backups(self.namespace.clone()).await {
            Ok(ids) => {
                let remote_wallet_truth =
                    match self.manager.load_remote_wallet_truth(&ids, self.cloud.clone()).await {
                        Ok(remote_wallet_truth) => remote_wallet_truth,
                        Err(error) => return Some(self.remote_truth_retry_result(&error)),
                    };

                self.manager.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

                let detail_result = self
                    .manager
                    .build_cloud_backup_detail_with_remote_truth(&ids, remote_wallet_truth)
                    .await;

                let detail = match detail_result {
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
                Some(self.cloud_storage_retry_result("failed to list wallet backups", error))
            }
        }
    }

    async fn ensure_wallet_inventory_or_short_circuit(&mut self) -> Option<DeepVerificationResult> {
        if self.wallet_record_ids.is_none()
            && !self.wallets_missing
            && let Some(result) = self.load_wallet_inventory().await
        {
            return Some(result);
        }

        if self.wallets_missing {
            return Some(self.recreate_manifest_result());
        }

        None
    }

    async fn load_encrypted_master_key(&self) -> Result<EncryptedMasterKeyStep, CloudBackupError> {
        match self.cloud.download_master_key_backup(self.namespace.clone()).await {
            Ok(json) => {
                let encrypted: EncryptedMasterKeyBackup =
                    serde_json::from_slice(&json).map_err_str(CloudBackupError::Internal)?;
                encrypted.remote_metadata.normalized_master_key(&self.namespace).map_err(
                    |error| {
                        CloudBackupError::Internal(format!("normalize master key payload: {error}"))
                    },
                )?;

                match encrypted.backup_version() {
                    Ok(MasterKeyBackupVersion::V1) => {}
                    Err(unsupported) => {
                        let version = unsupported.0;
                        return Ok(EncryptedMasterKeyStep::Finished(
                            DeepVerificationResult::Failed(
                                DeepVerificationFailure::UnsupportedVersion {
                                    message: format!(
                                        "master key backup version {version} is not supported",
                                    ),
                                    detail: self.detail(),
                                },
                            ),
                        ));
                    }
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
                self.cloud_storage_retry_result("failed to download master key backup", error),
            )),
        }
    }

    async fn resolve_master_key_step(
        &mut self,
        encrypted_master: Option<&EncryptedMasterKeyBackup>,
    ) -> Result<MasterKeyResolution, CloudBackupError> {
        let Some(encrypted_master) = encrypted_master else {
            return Ok(MasterKeyResolution::Continue(
                ContinuableMasterKeyResolution::NeedsWrapperRepair { reuse_credential_id: None },
            ));
        };

        let prf_salt = encrypted_master.prf_salt;
        let authenticated = match self.authenticate_with_fallback(&prf_salt).await? {
            PasskeyAuthOutcome::Authenticated(result) => result,
            PasskeyAuthOutcome::UserCancelled => {
                return Ok(MasterKeyResolution::Finished(self.resolve_cancellation_outcome()));
            }
            PasskeyAuthOutcome::NoCredentialFound => {
                if self.local_master_key.is_some() {
                    return Ok(MasterKeyResolution::Continue(
                        ContinuableMasterKeyResolution::NeedsWrapperRepair {
                            reuse_credential_id: None,
                        },
                    ));
                }

                return Ok(MasterKeyResolution::Finished(
                    self.reinitialize_result("no passkey found and no local master key"),
                ));
            }
        };

        self.report.credential_recovered = authenticated.credential_recovered;

        match master_key_crypto::decrypt_master_key(encrypted_master, &authenticated.prf_key) {
            // cloud wrapper decrypted, so this is the trusted master key for later wallet checks
            Ok(master_key) => {
                if let Err(error) =
                    self.cloud_keychain.save_passkey(&authenticated.credential_id, prf_salt)
                {
                    return Ok(MasterKeyResolution::Finished(
                        self.retry_result(format!("save cspp credentials: {error}")),
                    ));
                }

                Ok(MasterKeyResolution::Continue(ContinuableMasterKeyResolution::Authenticated(
                    AuthenticatedMasterKey::new(
                        master_key,
                        MasterKeyAuthorizationSource::CloudMasterKeyWrapper,
                    ),
                )))
            }
            // the passkey worked but the wrapper is stale; use the local key to replace it
            Err(_) if self.local_master_key.is_some() => Ok(MasterKeyResolution::Continue(
                ContinuableMasterKeyResolution::NeedsWrapperRepair {
                    reuse_credential_id: Some(authenticated.credential_id),
                },
            )),
            // without a local key there is no trusted source left to rebuild the cloud wrapper
            Err(_) => Ok(MasterKeyResolution::Finished(self.reinitialize_result(
                "could not decrypt cloud master key and no local key available",
            ))),
        }
    }

    fn apply_verified_cloud_master_key(
        &mut self,
        master_key: &MasterKey,
    ) -> Result<(), CloudBackupError> {
        let repaired = match &self.local_master_key {
            // restore the missing local key from the verified cloud backup
            None => {
                self.cspp
                    .save_master_key(master_key)
                    .map_err_prefix("repair local master key", CloudBackupError::Internal)?;
                info!("Repaired local master key from cloud");
                true
            }

            // replace a stale local key after cloud decryption proves the cloud key is valid
            Some(local_key) if local_key.as_bytes() != master_key.as_bytes() => {
                self.cspp
                    .save_master_key(master_key)
                    .map_err_prefix("repair local master key", CloudBackupError::Internal)?;
                info!("Repaired local master key to match cloud");
                true
            }

            // keep the local key when it already matches the verified cloud key
            Some(_) => false,
        };

        if repaired {
            self.local_master_key = Some(MasterKey::from_bytes(*master_key.as_bytes()));
            self.report.local_master_key_repaired = true;
        }

        Ok(())
    }

    async fn repair_wrapper_from_local_key(
        self,
        reuse_credential_id: Option<Vec<u8>>,
    ) -> Result<RepairedMasterKeyResolution, CloudBackupError> {
        let Some(local_master_key) = self.local_master_key.as_ref() else {
            return Ok(RepairedMasterKeyResolution::Finished(
                self.reinitialize_result("no local master key available for wrapper repair"),
            ));
        };
        let local_master_key = MasterKey::from_bytes(*local_master_key.as_bytes());

        let repair = WrapperRepairOperation::new(&self.cloud, &self.passkey, &self.namespace);

        let strategy = match reuse_credential_id {
            Some(credential_id) => WrapperRepairStrategy::ReuseExisting(credential_id),
            None => WrapperRepairStrategy::CreateNew,
        };

        let wallet_record_ids = self.wallet_record_ids.as_deref().unwrap_or(&[]);
        match repair.prepare(&local_master_key, wallet_record_ids, strategy).await {
            Ok(preparation) => Ok(RepairedMasterKeyResolution::PreparedWrapperRepair(Box::new(
                CloudBackupPreparedDeepVerificationWrapperRepair {
                    session: self,
                    preparation,
                    authenticated_master_key: local_master_key,
                },
            ))),

            Err(WrapperRepairError::WrongKey) => {
                Ok(RepairedMasterKeyResolution::Finished(self.reinitialize_result(
                    "local master key cannot decrypt existing cloud wallet backups",
                )))
            }

            Err(WrapperRepairError::Inconclusive) => Ok(RepairedMasterKeyResolution::Finished(
                self.retry_result("could not download any wallet to verify local key"),
            )),

            Err(WrapperRepairError::Operation(error)) => Err(error),
        }
    }

    async fn resume_after_wrapper_repair(
        mut self,
        authenticated_master_key: MasterKey,
    ) -> Result<CloudBackupDeepVerificationStep, CloudBackupError> {
        self.report.master_key_wrapper_repaired = true;
        info!("Repaired cloud master key wrapper");

        let authenticated_master = AuthenticatedMasterKey::new(
            authenticated_master_key,
            MasterKeyAuthorizationSource::RepairedCloudWrapper,
        );

        if let Some(result) = self.ensure_wallet_inventory_or_short_circuit().await {
            return Ok(CloudBackupDeepVerificationStep::Complete(result));
        }

        Ok(self.verify_wallet_backups_and_autosync(authenticated_master).await)
    }

    async fn verify_wallet_backups_and_autosync(
        mut self,
        authenticated_master: AuthenticatedMasterKey,
    ) -> CloudBackupDeepVerificationStep {
        let Some(wallet_record_ids) = self.wallet_record_ids.clone() else {
            return CloudBackupDeepVerificationStep::Complete(self.finish_verified());
        };

        info!(
            "Verification: checking wallet backups with authorization source {:?}",
            authenticated_master.source
        );
        let critical_key = Zeroizing::new(authenticated_master.master_key.critical_data_key());
        let (verified, failed, unsupported) = self.verify_wallet_backups(&critical_key).await;
        self.report.wallets_verified = verified;
        self.report.wallets_failed = failed;
        self.report.wallets_unsupported = unsupported;
        let other_backups = self.manager.other_backup_state(&self.cloud).await;
        let remote_wallet_truth_result =
            self.manager.load_remote_wallet_truth(&wallet_record_ids, self.cloud.clone()).await;

        let remote_wallet_truth = match remote_wallet_truth_result {
            Ok(remote_wallet_truth) => remote_wallet_truth,
            Err(error) => {
                return CloudBackupDeepVerificationStep::Complete(
                    self.remote_truth_retry_result(&error),
                );
            }
        };

        let inventory_result =
            CloudWalletInventory::load_with_remote_truth(&wallet_record_ids, remote_wallet_truth)
                .await;

        let unsynced = match inventory_result {
            Ok(inventory) => {
                let detail = inventory.build_detail(other_backups.clone());
                self.report.detail = Some(detail.clone());
                if inventory.has_unknown_remote_wallets() {
                    return CloudBackupDeepVerificationStep::Complete(
                        self.retry_result("failed to refresh remote wallet truth for some wallets"),
                    );
                }

                inventory.upload_candidate_wallets()
            }

            Err(error) => {
                return CloudBackupDeepVerificationStep::Complete(
                    self.local_inventory_retry_result(&error),
                );
            }
        };

        if unsynced.is_empty() {
            return CloudBackupDeepVerificationStep::Complete(self.finish_verified());
        }

        let count = unsynced.len() as u32;
        info!("Deep verify: {count} local wallet(s) not in cloud, auto-syncing");
        let namespace_id = self.namespace.clone();
        let upload_master_key = authenticated_master.master_key;

        CloudBackupDeepVerificationStep::PreparedAutoSync(Box::new(
            CloudBackupPreparedDeepVerificationAutoSync {
                session: self,
                upload: CloudBackupDeepVerificationAutoSyncUpload {
                    namespace_id,
                    wallets: unsynced,
                    master_key: upload_master_key,
                },
            },
        ))
    }

    async fn resume_after_auto_sync(
        mut self,
        uploaded: CloudBackupUploadedDeepVerificationAutoSync,
    ) -> CloudBackupDeepVerificationAutoSyncCompletion {
        let pending_uploads = uploaded
            .uploaded_wallets()
            .iter()
            .map(|wallet| {
                PendingVerificationUpload::new(
                    wallet.record_id().to_owned(),
                    wallet.revision_hash().to_owned(),
                )
            })
            .collect::<Vec<_>>();

        let other_backups = self.manager.other_backup_state(&self.cloud).await;
        let updated_ids = match self.cloud.list_wallet_backups(self.namespace.clone()).await {
            Ok(updated_ids) => updated_ids,
            Err(error) => {
                warn!("Deep verify: failed to re-check wallet backups after auto-sync: {error}");
                return CloudBackupDeepVerificationAutoSyncCompletion::complete(
                    self.cloud_storage_retry_result(
                        "failed to re-check wallet backups after auto-sync",
                        error,
                    ),
                );
            }
        };

        let remote_wallet_truth =
            match self.manager.load_remote_wallet_truth(&updated_ids, self.cloud.clone()).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => {
                    return CloudBackupDeepVerificationAutoSyncCompletion::complete(
                        self.remote_truth_retry_result(&error),
                    );
                }
            };

        self.manager.cleanup_confirmed_pending_blobs(&remote_wallet_truth);

        let unconfirmed_pending_uploads = pending_uploads
            .iter()
            .filter(|upload| match (upload.wallet_record_id(), upload.wallet_revision()) {
                (Some(record_id), Some(expected_revision)) => !remote_wallet_revision_matches(
                    &remote_wallet_truth,
                    record_id,
                    expected_revision,
                ),
                _ => false,
            })
            .count();

        let inventory =
            match CloudWalletInventory::load_with_remote_truth(&updated_ids, remote_wallet_truth)
                .await
            {
                Ok(inventory) => inventory,
                Err(error) => {
                    return CloudBackupDeepVerificationAutoSyncCompletion::complete(
                        self.local_inventory_retry_result(&error),
                    );
                }
            };

        let listed: std::collections::HashSet<_> = updated_ids.iter().cloned().collect();
        let remaining_unsynced = inventory.upload_candidate_wallets();

        self.report.detail = Some(inventory.build_detail(other_backups));
        self.wallet_record_ids = Some(updated_ids);

        if inventory.has_unknown_remote_wallets() {
            return CloudBackupDeepVerificationAutoSyncCompletion::complete(
                self.retry_result("failed to refresh remote wallet truth for some wallets"),
            );
        }

        let missing_listed_uploads = pending_uploads
            .iter()
            .filter_map(PendingVerificationUpload::wallet_record_id)
            .any(|record_id| !listed.contains(record_id));

        if remaining_unsynced.is_empty()
            && !missing_listed_uploads
            && unconfirmed_pending_uploads == 0
        {
            return CloudBackupDeepVerificationAutoSyncCompletion::complete(self.finish_verified());
        }

        let remaining_count = remaining_unsynced.len();
        let missing_count = pending_uploads
            .iter()
            .filter_map(PendingVerificationUpload::wallet_record_id)
            .filter(|record_id| !listed.contains(*record_id))
            .count();
        let stale_count = unconfirmed_pending_uploads.saturating_sub(missing_count);

        warn!(
            "Deep verify: auto-sync finished but confirmation is still pending missing_listed={missing_count} stale={stale_count} stale_or_unsynced={remaining_count}"
        );

        let result = DeepVerificationResult::AwaitingUploadConfirmation(self.report.clone());
        CloudBackupDeepVerificationAutoSyncCompletion::awaiting_upload_confirmation(
            result,
            PendingVerificationCompletion::new(
                self.report.clone(),
                self.namespace.clone(),
                pending_uploads,
            ),
        )
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
        self.cloud_backup_retry_result("failed to load local wallet inventory", error)
    }

    fn remote_truth_retry_result(&self, error: &CloudBackupError) -> DeepVerificationResult {
        self.cloud_backup_retry_result("failed to refresh remote wallet truth", error)
    }

    /// Builds a retryable verification failure while preserving the latest backup detail for UI recovery prompts
    fn retry_result(&self, message: impl Into<String>) -> DeepVerificationResult {
        self.retry_result_with_context(message, None)
    }

    fn retry_result_with_context(
        &self,
        message: impl Into<String>,
        retry_context: Option<CloudBackupRetryContext>,
    ) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure::retry(
            message,
            self.detail(),
            retry_context,
        ))
    }

    fn connectivity_retry_context(&self) -> CloudBackupRetryContext {
        let action = if self.force_discoverable {
            CloudBackupRetryAction::VerifyDiscoverable
        } else {
            CloudBackupRetryAction::Verify
        };

        CloudBackupRetryContext::connectivity(action)
    }

    fn cloud_storage_retry_result(
        &self,
        context: &'static str,
        error: CloudStorageError,
    ) -> DeepVerificationResult {
        let error = CloudBackupError::cloud_storage_context(context, error);
        let error = blocking_cloud_error(BlockingCloudStep::Verify, error);

        let retry_context =
            is_connectivity_related_issue(&error).then(|| self.connectivity_retry_context());

        self.retry_result_with_context(error.to_string(), retry_context)
    }

    fn cloud_backup_retry_result(
        &self,
        context: &'static str,
        error: &CloudBackupError,
    ) -> DeepVerificationResult {
        if is_connectivity_related_issue(error) {
            return self.retry_result_with_context(
                offline_error_for_step(BlockingCloudStep::Verify).to_string(),
                Some(self.connectivity_retry_context()),
            );
        }

        self.retry_result(format!("{context}: {error}"))
    }

    /// Builds the failure shown when wallet blobs are missing but local data can recreate the manifest
    fn recreate_manifest_result(&self) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure::RecreateManifest {
            message: "wallet backups not found in iCloud namespace".into(),
            warning: RECREATE_WARNING.into(),
            detail: self.detail(),
        })
    }

    /// Builds the failure shown when the backup cannot be trusted and should be recreated from scratch
    fn reinitialize_result(&self, message: impl Into<String>) -> DeepVerificationResult {
        DeepVerificationResult::Failed(DeepVerificationFailure::ReinitializeBackup {
            message: message.into(),
            warning: REINITIALIZE_WARNING.into(),
            detail: self.detail(),
        })
    }

    /// When the user cancels the discoverable passkey picker, check if the
    /// stored credential still exists. If it does the backup is healthy and
    /// we avoid downgrading persisted state. If the credential is gone the
    /// passkey is durably missing and the user needs repair
    fn resolve_cancellation_outcome(&self) -> DeepVerificationResult {
        match self.cloud_keychain.load_credential_id() {
            Some(credential_id) => match self
                .passkey
                .check_passkey_presence(PASSKEY_RP_ID.to_string(), credential_id)
            {
                PasskeyCredentialPresence::Present => {
                    info!("Passkey picker cancelled but stored credential still exists");
                    Self::cancellation_outcome(PasskeyCredentialPresence::Present, self.detail())
                }
                PasskeyCredentialPresence::Missing => {
                    info!("Passkey picker cancelled and stored credential is missing");
                    self.cloud_keychain.clear_passkey();
                    Self::cancellation_outcome(PasskeyCredentialPresence::Missing, self.detail())
                }
                PasskeyCredentialPresence::Indeterminate => {
                    info!(
                        "Passkey picker cancelled and stored credential could not be revalidated"
                    );
                    Self::cancellation_outcome(
                        PasskeyCredentialPresence::Indeterminate,
                        self.detail(),
                    )
                }
            },
            None => {
                info!("Passkey picker cancelled and no stored credential found");
                DeepVerificationResult::PasskeyMissing(self.detail())
            }
        }
    }

    /// Maps passkey presence after cancellation to the verification result the UI should show
    fn cancellation_outcome(
        presence: PasskeyCredentialPresence,
        detail: Option<CloudBackupDetail>,
    ) -> DeepVerificationResult {
        match presence {
            PasskeyCredentialPresence::Present => DeepVerificationResult::PasskeyConfirmed(detail),
            PasskeyCredentialPresence::Missing => DeepVerificationResult::PasskeyMissing(detail),
            PasskeyCredentialPresence::Indeterminate => {
                DeepVerificationResult::UserCancelled(detail)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_outcome_confirms_present_passkey() {
        let result =
            VerificationSession::cancellation_outcome(PasskeyCredentialPresence::Present, None);
        assert!(matches!(result, DeepVerificationResult::PasskeyConfirmed(None)));
    }

    #[test]
    fn cancellation_outcome_marks_missing_passkey() {
        let result =
            VerificationSession::cancellation_outcome(PasskeyCredentialPresence::Missing, None);
        assert!(matches!(result, DeepVerificationResult::PasskeyMissing(None)));
    }

    #[test]
    fn cancellation_outcome_treats_indeterminate_as_user_cancelled() {
        let result = VerificationSession::cancellation_outcome(
            PasskeyCredentialPresence::Indeterminate,
            None,
        );
        assert!(matches!(result, DeepVerificationResult::UserCancelled(None)));
    }
}
