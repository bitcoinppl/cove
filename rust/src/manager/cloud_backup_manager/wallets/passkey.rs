use std::time::Duration;

use cove_cspp::backup_data::EncryptedMasterKeyBackup;
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use cove_tokio::unblock;
use rand::RngExt as _;
use tracing::{info, warn};

use super::super::{CloudBackupError, PASSKEY_RP_ID};
use super::UnpersistedPrfKey;

async fn delay_before_new_passkey_auth() {
    let delay = Duration::from_secs(3);
    info!("Waiting {delay:?} before authenticating new passkey");
    tokio::time::sleep(delay).await;
}

pub struct NamespaceMatch {
    pub namespace_id: String,
    pub master_key: cove_cspp::master_key::MasterKey,
    pub prf_salt: [u8; 32],
    pub credential_id: Vec<u8>,
}

pub enum NamespaceMatchOutcome {
    Matched(NamespaceMatch),
    UserDeclined,
    NoMatch,
    Inconclusive,
    UnsupportedVersions,
}

/// Acquires passkey PRF material without persisting it to the keychain
pub struct PasskeyMaterialAcquirer {
    passkey: PasskeyAccess,
}

impl PasskeyMaterialAcquirer {
    /// Builds an acquirer from the passkey service handle
    pub fn new(passkey: &PasskeyAccess) -> Self {
        Self { passkey: passkey.clone() }
    }

    /// Creates a passkey for wrapper repair without persisting keychain state
    pub async fn create_for_wrapper_repair(&self) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("Creating new passkey for wrapper repair");
        self.create_new_prf_key_with_mapper(map_wrapper_repair_passkey_error).await
    }

    /// Discovers an existing passkey for wrapper repair or creates a new one
    pub async fn discover_or_create_for_wrapper_repair(
        &self,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("Attempting passkey discovery before creating new wrapper-repair passkey");
        let prf_salt: [u8; 32] = rand::rng().random();

        let discovery = {
            let passkey = self.passkey.clone();
            unblock::run_blocking(move || {
                passkey.discover_and_authenticate_with_prf(
                    PASSKEY_RP_ID.to_string(),
                    prf_salt.to_vec(),
                    random_challenge(),
                )
            })
            .await
        };

        match discovery {
            Ok(discovered) => {
                let prf_key = prf_output_to_key(discovered.prf_output)?;
                info!("Discovered existing passkey for wrapper repair");

                Ok(UnpersistedPrfKey { prf_key, prf_salt, credential_id: discovered.credential_id })
            }
            Err(PasskeyError::UserCancelled) => {
                info!("User cancelled passkey discovery for wrapper repair");
                Err(CloudBackupError::PasskeyDiscoveryCancelled)
            }
            Err(PasskeyError::NoCredentialFound) => {
                info!("No existing passkey found for wrapper repair, creating new");
                self.create_for_wrapper_repair().await
            }
            Err(PasskeyError::PrfUnsupportedProvider) => {
                Err(CloudBackupError::UnsupportedPasskeyProvider)
            }
            Err(error) => {
                warn!("Wrapper-repair discovery failed ({error}), falling back to create");
                self.create_for_wrapper_repair().await
            }
        }
    }

    /// Creates passkey material for enabling cloud backup
    pub async fn create_new(
        &self,
        log_message: &str,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("{log_message}");
        self.create_new_prf_key_with_mapper(map_enable_passkey_error).await
    }

    async fn create_new_prf_key_with_mapper(
        &self,
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        let prf_salt: [u8; 32] = rand::rng().random();
        let credential_id = {
            let passkey = self.passkey.clone();
            unblock::run_blocking(move || {
                passkey.create_passkey(
                    PASSKEY_RP_ID.to_string(),
                    rand::rng().random::<[u8; 16]>().to_vec(),
                    random_challenge(),
                )
            })
            .await
            .map_err(map_passkey_error)?
        };

        // wait briefly before targeted auth so iOS can settle after registration
        // without probing for presence and flashing another native passkey sheet
        delay_before_new_passkey_auth().await;

        let prf_output = {
            let passkey = self.passkey.clone();
            let credential_id = credential_id.clone();
            unblock::run_blocking(move || {
                passkey.authenticate_with_prf(
                    PASSKEY_RP_ID.to_string(),
                    credential_id,
                    prf_salt.to_vec(),
                    random_challenge(),
                )
            })
            .await
            .map_err(map_passkey_error)?
        };

        Ok(UnpersistedPrfKey { prf_key: prf_output_to_key(prf_output)?, prf_salt, credential_id })
    }
}

/// Matches a discoverable passkey against candidate cloud backup namespaces
pub struct NamespacePasskeyMatcher {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher from cloud and passkey service handles
    pub fn new(cloud: &CloudStorageClient, passkey: &PasskeyAccess) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone() }
    }

    /// Downloads candidate wrappers and tries the selected passkey against each PRF salt
    pub async fn match_namespaces(
        &self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchOutcome, CloudBackupError> {
        if namespaces.is_empty() {
            return Ok(NamespaceMatchOutcome::NoMatch);
        }

        let mut downloaded: Vec<(String, EncryptedMasterKeyBackup)> =
            Vec::with_capacity(namespaces.len());
        let mut had_download_failures = false;
        let mut had_unsupported_versions = false;

        for namespace in namespaces {
            let Ok(master_json) =
                self.cloud.download_master_key_backup(namespace.clone()).await.inspect_err(
                    |error| {
                        warn!("Failed to download master key for namespace {namespace}: {error}");
                        had_download_failures = true;
                    },
                )
            else {
                continue;
            };

            let Ok(encrypted) = serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json)
                .inspect_err(|error| {
                    warn!("Failed to deserialize master key for namespace {namespace}: {error}");
                    had_download_failures = true;
                })
            else {
                continue;
            };

            if encrypted.version != 1 {
                had_unsupported_versions = true;
                continue;
            }

            downloaded.push((namespace.clone(), encrypted));
        }

        if downloaded.is_empty() && had_download_failures {
            return Ok(NamespaceMatchOutcome::Inconclusive);
        }

        if downloaded.is_empty() && had_unsupported_versions {
            return Ok(NamespaceMatchOutcome::UnsupportedVersions);
        }

        let (namespace_id, first_encrypted) = &downloaded[0];
        let discovery = {
            let passkey = self.passkey.clone();
            let prf_salt = first_encrypted.prf_salt;
            unblock::run_blocking(move || {
                passkey.discover_and_authenticate_with_prf(
                    PASSKEY_RP_ID.to_string(),
                    prf_salt.to_vec(),
                    random_challenge(),
                )
            })
            .await
        };
        let discovered = match discovery {
            Ok(discovered) => discovered,
            Err(PasskeyError::UserCancelled) => return Ok(NamespaceMatchOutcome::UserDeclined),
            Err(PasskeyError::NoCredentialFound) => return Ok(NamespaceMatchOutcome::NoMatch),
            Err(error) => return Err(CloudBackupError::Passkey(error.to_string())),
        };

        let prf_key = prf_output_to_key(discovered.prf_output.clone())?;
        let try_first = cove_cspp::master_key_crypto::decrypt_master_key(first_encrypted, &prf_key);
        if let Ok(master_key) = try_first {
            return Ok(NamespaceMatchOutcome::Matched(NamespaceMatch {
                namespace_id: namespace_id.clone(),
                master_key,
                prf_salt: first_encrypted.prf_salt,
                credential_id: discovered.credential_id,
            }));
        }

        for (namespace_id, encrypted) in downloaded.iter().skip(1) {
            let prf_output_result = {
                let passkey = self.passkey.clone();
                let credential_id = discovered.credential_id.clone();
                let prf_salt = encrypted.prf_salt;
                unblock::run_blocking(move || {
                    passkey.authenticate_with_prf(
                        PASSKEY_RP_ID.to_string(),
                        credential_id,
                        prf_salt.to_vec(),
                        random_challenge(),
                    )
                })
                .await
            };

            let prf_output = match prf_output_result {
                Ok(prf_output) => prf_output,
                Err(PasskeyError::UserCancelled) => return Ok(NamespaceMatchOutcome::UserDeclined),
                Err(error) => {
                    warn!("Failed targeted passkey auth for namespace {namespace_id}: {error}");
                    had_download_failures = true;
                    continue;
                }
            };

            let prf_key = prf_output_to_key(prf_output)?;

            if let Ok(master_key) =
                cove_cspp::master_key_crypto::decrypt_master_key(encrypted, &prf_key)
            {
                let matched = NamespaceMatch {
                    namespace_id: namespace_id.clone(),
                    master_key,
                    prf_salt: encrypted.prf_salt,
                    credential_id: discovered.credential_id.clone(),
                };

                return Ok(NamespaceMatchOutcome::Matched(matched));
            }
        }

        if had_download_failures {
            return Ok(NamespaceMatchOutcome::Inconclusive);
        }

        if downloaded.is_empty() && had_unsupported_versions {
            return Ok(NamespaceMatchOutcome::UnsupportedVersions);
        }

        Ok(NamespaceMatchOutcome::NoMatch)
    }
}

fn map_wrapper_repair_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for wrapper repair");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::Passkey(other.to_string()),
    }
}

fn map_enable_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for cloud backup enable");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::Passkey(other.to_string()),
    }
}

fn prf_output_to_key(prf_output: Vec<u8>) -> Result<[u8; 32], CloudBackupError> {
    prf_output
        .try_into()
        .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))
}

fn random_challenge() -> Vec<u8> {
    rand::rng().random::<[u8; 32]>().to_vec()
}
