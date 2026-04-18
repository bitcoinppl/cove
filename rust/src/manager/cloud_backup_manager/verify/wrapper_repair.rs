use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_tokio::unblock;
use cove_util::ResultExt as _;
use rand::RngExt as _;
use tracing::info;
use zeroize::Zeroizing;

use super::super::{
    CloudBackupError, PASSKEY_RP_ID, RustCloudBackupManager, cspp_master_key_record_id,
};
use crate::manager::cloud_backup_manager::wallets::{
    WalletBackupLookup, WalletBackupReader, create_prf_key_without_persisting,
    discover_or_create_prf_key_without_persisting,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalKeyProof {
    Verified,
    WrongKey,
    Inconclusive,
}

#[derive(Debug)]
pub(super) enum WrapperRepairError {
    WrongKey,
    Inconclusive,
    Operation(CloudBackupError),
}

impl WrapperRepairError {
    pub(super) fn into_cloud_backup_error(self) -> CloudBackupError {
        match self {
            Self::WrongKey => CloudBackupError::Crypto(
                "local master key cannot decrypt existing cloud wallet backups".into(),
            ),
            Self::Inconclusive => {
                CloudBackupError::Cloud("could not download any wallet to verify local key".into())
            }
            Self::Operation(error) => error,
        }
    }
}

#[derive(Debug)]
pub(super) enum WrapperRepairStrategy {
    CreateNew,
    DiscoverOrCreate,
    ReuseExisting(Vec<u8>),
}

#[derive(Debug)]
struct WrapperRepairCredentials {
    prf_key: [u8; 32],
    prf_salt: [u8; 32],
    credential_id: Vec<u8>,
}

struct LocalKeyVerifier {
    cloud: CloudStorage,
    namespace: String,
}

impl LocalKeyVerifier {
    fn new(cloud: &CloudStorage, namespace: &str) -> Self {
        Self { cloud: cloud.clone(), namespace: namespace.to_owned() }
    }

    async fn prove(&self, wallet_record_ids: &[String], master_key: &MasterKey) -> LocalKeyProof {
        let reader = WalletBackupReader::new(
            self.cloud.clone(),
            self.namespace.clone(),
            Zeroizing::new(master_key.critical_data_key()),
        );
        let mut had_wrong_key = false;
        let mut verified = false;

        for record_id in wallet_record_ids {
            let encrypted = match reader.download_encrypted(record_id).await {
                Ok(WalletBackupLookup::Found(encrypted)) => encrypted,
                Ok(WalletBackupLookup::NotFound | WalletBackupLookup::UnsupportedVersion(_))
                | Err(_) => continue,
            };

            match reader.decrypt_entry(&encrypted) {
                Ok(_) => {
                    verified = true;
                    break;
                }
                Err(cove_cspp::CsppError::WrongKey) => {
                    had_wrong_key = true;
                }
                Err(_) => {}
            }
        }

        if verified {
            return LocalKeyProof::Verified;
        }

        if had_wrong_key {
            return LocalKeyProof::WrongKey;
        }

        LocalKeyProof::Inconclusive
    }
}

pub(super) struct WrapperRepairOperation {
    manager: RustCloudBackupManager,
    keychain: Keychain,
    cloud: CloudStorage,
    passkey: PasskeyAccess,
    namespace: String,
}

impl WrapperRepairOperation {
    pub(super) fn new(
        manager: &RustCloudBackupManager,
        keychain: &Keychain,
        cloud: &CloudStorage,
        passkey: &PasskeyAccess,
        namespace: &str,
    ) -> Self {
        Self {
            manager: manager.clone(),
            keychain: keychain.clone(),
            cloud: cloud.clone(),
            passkey: passkey.clone(),
            namespace: namespace.to_owned(),
        }
    }

    pub(super) async fn run(
        &self,
        local_master_key: &MasterKey,
        wallet_record_ids: &[String],
        strategy: WrapperRepairStrategy,
    ) -> Result<(), WrapperRepairError> {
        self.verify_local_key(wallet_record_ids, local_master_key).await?;

        let credentials =
            self.credentials(strategy).await.map_err(WrapperRepairError::Operation)?;
        let encrypted_backup = master_key_crypto::encrypt_master_key(
            local_master_key,
            &credentials.prf_key,
            &credentials.prf_salt,
        )
        .map_err_str(CloudBackupError::Crypto)
        .map_err(WrapperRepairError::Operation)?;

        let backup_json = serde_json::to_vec(&encrypted_backup)
            .map_err_str(CloudBackupError::Internal)
            .map_err(WrapperRepairError::Operation)?;

        self.cloud
            .upload_master_key_backup(self.namespace.clone(), backup_json)
            .await
            .map_err_str(CloudBackupError::Cloud)
            .map_err(WrapperRepairError::Operation)?;

        self.keychain
            .save_cspp_passkey(&credentials.credential_id, credentials.prf_salt)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)
            .map_err(WrapperRepairError::Operation)?;
        self.manager
            .mark_blob_uploaded_pending_confirmation(
                self.namespace.as_str(),
                None,
                cspp_master_key_record_id(),
                "master-key-wrapper".into(),
                jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
            )
            .map_err(WrapperRepairError::Operation)?;

        Ok(())
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
                let new_prf = create_prf_key_without_persisting(&self.passkey).await?;

                Ok(WrapperRepairCredentials {
                    prf_key: new_prf.prf_key,
                    prf_salt: new_prf.prf_salt,
                    credential_id: new_prf.credential_id.clone(),
                })
            }
            WrapperRepairStrategy::DiscoverOrCreate => {
                let passkey = discover_or_create_prf_key_without_persisting(&self.passkey).await?;
                info!("Using discovered-or-new passkey for wrapper repair");

                Ok(WrapperRepairCredentials {
                    prf_key: passkey.prf_key,
                    prf_salt: passkey.prf_salt,
                    credential_id: passkey.credential_id.clone(),
                })
            }
            WrapperRepairStrategy::ReuseExisting(credential_id) => {
                let prf_salt: [u8; 32] = rand::rng().random();
                let passkey = self.passkey.clone();
                let auth_credential_id = credential_id.clone();
                let prf_output = unblock::run_blocking(move || {
                    passkey.authenticate_with_prf(
                        PASSKEY_RP_ID.to_owned(),
                        auth_credential_id,
                        prf_salt.to_vec(),
                        rand::rng().random::<[u8; 32]>().to_vec(),
                    )
                })
                .await
                .map_err_str(CloudBackupError::Passkey)?;

                let prf_key: [u8; 32] = prf_output
                    .try_into()
                    .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

                info!("Reusing discovered passkey for wrapper repair");

                Ok(WrapperRepairCredentials { prf_key, prf_salt, credential_id })
            }
        }
    }
}
