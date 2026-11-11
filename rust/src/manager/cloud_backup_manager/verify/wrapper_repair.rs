use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::passkey::PasskeyAccess;
use rand::RngExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::manager::cloud_backup_manager::wallets::{
    PasskeyMaterialAcquirer, PasskeyRequestAuthority, PlatformAuthorizationRetrier,
    WalletBackupLookup, WalletBackupReader, map_wrapper_repair_passkey_error,
};
use crate::manager::cloud_backup_manager::{CloudBackupError, master_key_wrapper_revision_hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalKeyProof {
    Verified,
    WrongKey,
    Inconclusive,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WrapperRepairError {
    #[error("local master key cannot decrypt existing cloud wallet backups")]
    WrongKey,

    #[error("could not download any wallet to verify local key")]
    Inconclusive,

    #[error(transparent)]
    Operation(#[from] CloudBackupError),
}

impl From<WrapperRepairError> for CloudBackupError {
    fn from(error: WrapperRepairError) -> Self {
        let msg = error.to_string();

        match error {
            WrapperRepairError::WrongKey => CloudBackupError::Crypto(msg.into()),
            WrapperRepairError::Inconclusive => CloudBackupError::Cloud(msg),
            WrapperRepairError::Operation(error) => error,
        }
    }
}

/// Chooses how wrapper repair should acquire passkey material
pub(crate) enum WrapperRepairStrategy {
    CreateNew,
    DiscoverOrCreate,
    ReuseExisting(Vec<u8>),
}

impl std::fmt::Debug for WrapperRepairStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateNew => f.write_str("CreateNew"),
            Self::DiscoverOrCreate => f.write_str("DiscoverOrCreate"),
            Self::ReuseExisting(credential_id) => f
                .debug_tuple("ReuseExisting")
                .field(&format_args!("<redacted len={}>", credential_id.len()))
                .finish(),
        }
    }
}

struct WrapperRepairCredentials {
    prf_key: Zeroizing<[u8; 32]>,
    prf_salt: [u8; 32],
    credential_id: Vec<u8>,
    provider_hint: Option<cove_cspp::backup_data::PasskeyProviderHint>,
}

impl std::fmt::Debug for WrapperRepairCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrapperRepairCredentials")
            .field("prf_key", &"<redacted>")
            .field("prf_salt", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("provider_hint", &self.provider_hint)
            .finish()
    }
}

pub(crate) struct CloudBackupPreparedPasskeyWrapperRepair {
    pub(crate) namespace_id: String,
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) master_key_wrapper_json: Vec<u8>,
    pub(crate) master_key_wrapper_revision: String,
    pub(crate) uploaded_at: u64,
}

impl std::fmt::Debug for CloudBackupPreparedPasskeyWrapperRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupPreparedPasskeyWrapperRepair")
            .field("namespace_id", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .field(
                "master_key_wrapper_json",
                &format_args!("<redacted len={}>", self.master_key_wrapper_json.len()),
            )
            .field("master_key_wrapper_revision", &self.master_key_wrapper_revision)
            .field("uploaded_at", &self.uploaded_at)
            .finish()
    }
}

pub(crate) struct CloudBackupPasskeyWrapperRepairUpload {
    pub(crate) namespace_id: String,
    pub(crate) master_key_wrapper_json: Vec<u8>,
    pub(crate) master_key_wrapper_revision: String,
    pub(crate) uploaded_at: u64,
}

impl std::fmt::Debug for CloudBackupPasskeyWrapperRepairUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupPasskeyWrapperRepairUpload")
            .field("namespace_id", &"<redacted>")
            .field(
                "master_key_wrapper_json",
                &format_args!("<redacted len={}>", self.master_key_wrapper_json.len()),
            )
            .field("master_key_wrapper_revision", &self.master_key_wrapper_revision)
            .field("uploaded_at", &self.uploaded_at)
            .finish()
    }
}

pub(crate) struct CloudBackupUploadedPasskeyWrapperRepair {
    pub(crate) namespace_id: String,
}

impl std::fmt::Debug for CloudBackupUploadedPasskeyWrapperRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupUploadedPasskeyWrapperRepair")
            .field("namespace_id", &"<redacted>")
            .finish()
    }
}

impl CloudBackupPreparedPasskeyWrapperRepair {
    pub(crate) fn into_upload(self) -> CloudBackupPasskeyWrapperRepairUpload {
        CloudBackupPasskeyWrapperRepairUpload {
            namespace_id: self.namespace_id,
            master_key_wrapper_json: self.master_key_wrapper_json,
            master_key_wrapper_revision: self.master_key_wrapper_revision,
            uploaded_at: self.uploaded_at,
        }
    }
}

struct LocalKeyVerifier {
    cloud: CloudStorageClient,
    namespace: String,
}

impl LocalKeyVerifier {
    fn new(cloud: &CloudStorageClient, namespace: &str) -> Self {
        Self { cloud: cloud.clone(), namespace: namespace.to_owned() }
    }

    async fn prove(&self, wallet_record_ids: &[String], master_key: &MasterKey) -> LocalKeyProof {
        let reader = WalletBackupReader::new(
            self.cloud.clone(),
            self.namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut skipped = SkippedProofRecords::default();

        for record_id in wallet_record_ids {
            let encrypted = match reader.download_encrypted(record_id).await {
                Ok(WalletBackupLookup::Found(encrypted)) => encrypted,
                Ok(WalletBackupLookup::NotFound) => {
                    skipped.not_found += 1;
                    continue;
                }
                Ok(WalletBackupLookup::UnsupportedVersion(_)) => {
                    skipped.unsupported += 1;
                    continue;
                }
                Err(error) => {
                    skipped.download_failed += 1;
                    skipped.first_error.get_or_insert_with(|| error.to_string());
                    continue;
                }
            };

            match reader.decrypt_entry(&encrypted) {
                Ok(_) => return LocalKeyProof::Verified,
                Err(cove_cspp::CsppError::WrongKey) => skipped.wrong_key += 1,
                Err(error) => {
                    skipped.decrypt_failed += 1;
                    skipped.first_error.get_or_insert_with(|| error.to_string());
                }
            }
        }

        // one summary per proof, since the per-record causes decide wrong key versus inconclusive
        let proof = if skipped.wrong_key > 0 {
            LocalKeyProof::WrongKey
        } else {
            LocalKeyProof::Inconclusive
        };

        warn!(
            "Local master key proof {proof:?} over {} wallet backup(s): wrong_key={}, not_found={}, unsupported={}, download_failed={}, decrypt_failed={}, first_error={}",
            wallet_record_ids.len(),
            skipped.wrong_key,
            skipped.not_found,
            skipped.unsupported,
            skipped.download_failed,
            skipped.decrypt_failed,
            skipped.first_error.as_deref().unwrap_or("none")
        );

        proof
    }
}

/// Why each wallet backup failed to prove the local master key, summarized after the proof
#[derive(Default)]
struct SkippedProofRecords {
    wrong_key: usize,
    not_found: usize,
    unsupported: usize,
    download_failed: usize,
    decrypt_failed: usize,
    first_error: Option<String>,
}

/// Repairs the cloud master-key wrapper after proving the local master key is valid
pub(crate) struct WrapperRepairOperation {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
    authority: PasskeyRequestAuthority,
    namespace: String,
}

impl WrapperRepairOperation {
    pub(crate) fn new(
        cloud: &CloudStorageClient,
        passkey: &PasskeyAccess,
        authority: &PasskeyRequestAuthority,
        namespace: &str,
    ) -> Self {
        Self {
            cloud: cloud.clone(),
            passkey: passkey.clone(),
            authority: authority.clone(),
            namespace: namespace.to_owned(),
        }
    }

    /// Prepares a repaired wrapper after proving the local master key is valid
    pub(crate) async fn prepare(
        &self,
        local_master_key: &MasterKey,
        wallet_record_ids: &[String],
        strategy: WrapperRepairStrategy,
    ) -> Result<CloudBackupPreparedPasskeyWrapperRepair, WrapperRepairError> {
        self.verify_local_key(wallet_record_ids, local_master_key).await?;

        let credentials = self.credentials(strategy).await?;

        let uploaded_at = cove_util::time::unix_timestamp_secs_or_zero();
        let encrypted_backup = master_key_crypto::encrypt_master_key_with_remote_metadata(
            local_master_key,
            &credentials.prf_key,
            &credentials.prf_salt,
            credentials.provider_hint.clone(),
            RemotePayloadMetadata::master_key(&self.namespace, uploaded_at),
        )
        .map_err(CloudBackupError::crypto)?;

        let backup_json =
            serde_json::to_vec(&encrypted_backup).map_err(CloudBackupError::internal)?;
        let master_key_wrapper_revision = master_key_wrapper_revision_hash(&backup_json);

        Ok(CloudBackupPreparedPasskeyWrapperRepair {
            namespace_id: self.namespace.clone(),
            credential_id: credentials.credential_id,
            prf_salt: credentials.prf_salt,
            master_key_wrapper_json: backup_json,
            master_key_wrapper_revision,
            uploaded_at,
        })
    }

    async fn verify_local_key(
        &self,
        wallet_record_ids: &[String],
        local_master_key: &MasterKey,
    ) -> Result<(), WrapperRepairError> {
        if wallet_record_ids.is_empty() {
            return Ok(());
        }

        let verifier = LocalKeyVerifier::new(&self.cloud, &self.namespace);
        match verifier.prove(wallet_record_ids, local_master_key).await {
            LocalKeyProof::Verified => Ok(()),
            LocalKeyProof::WrongKey => Err(WrapperRepairError::WrongKey),
            LocalKeyProof::Inconclusive => Err(WrapperRepairError::Inconclusive),
        }
    }

    async fn credentials(
        &self,
        strategy: WrapperRepairStrategy,
    ) -> Result<WrapperRepairCredentials, CloudBackupError> {
        match strategy {
            WrapperRepairStrategy::CreateNew => {
                let new_prf = PasskeyMaterialAcquirer::new(&self.passkey, &self.authority)
                    .create_for_wrapper_repair()
                    .await?;
                let provider_hint = new_prf.provider_hint.clone();
                let (prf_key, prf_salt, credential_id) = new_prf.into_parts();

                Ok(WrapperRepairCredentials {
                    prf_key: Zeroizing::new(prf_key),
                    prf_salt,
                    credential_id,
                    provider_hint,
                })
            }

            WrapperRepairStrategy::DiscoverOrCreate => {
                let passkey = PasskeyMaterialAcquirer::new(&self.passkey, &self.authority)
                    .discover_or_create_for_wrapper_repair()
                    .await?;
                info!("Using discovered-or-new passkey for wrapper repair");
                let provider_hint = passkey.provider_hint.clone();
                let (prf_key, prf_salt, credential_id) = passkey.into_parts();

                Ok(WrapperRepairCredentials {
                    prf_key: Zeroizing::new(prf_key),
                    prf_salt,
                    credential_id,
                    provider_hint,
                })
            }

            WrapperRepairStrategy::ReuseExisting(credential_id) => {
                let prf_salt: [u8; 32] = rand::rng().random();
                let retrier = PlatformAuthorizationRetrier::new(self.authority.clone());
                let prf_output = retrier
                    .authenticate(&self.passkey, &credential_id, prf_salt)
                    .await?
                    .map_err(map_wrapper_repair_passkey_error)?;

                let prf_key: [u8; 32] = prf_output
                    .try_into()
                    .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

                info!("Reusing discovered passkey for wrapper repair");

                Ok(WrapperRepairCredentials {
                    prf_key: Zeroizing::new(prf_key),
                    prf_salt,
                    credential_id,
                    provider_hint: None,
                })
            }
        }
    }
}
