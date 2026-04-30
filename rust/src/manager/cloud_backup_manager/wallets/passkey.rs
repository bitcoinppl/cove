use std::time::Duration;

use cove_cspp::backup_data::{EncryptedMasterKeyBackup, MasterKeyBackupVersion};
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
    Matched(Vec<NamespaceMatch>),
    UserDeclined,
    NoMatch,
    Inconclusive,
    UnsupportedVersions,
}

struct PasskeyMaterialDiscoveryContext {
    fallback_context: &'static str,
    attempt_message: &'static str,
    discovered_message: &'static str,
    cancelled_message: &'static str,
    missing_message: &'static str,
    failed_message: &'static str,
    create_for_enable: bool,
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

    /// Creates a passkey for enabling cloud backup without persisting keychain state
    pub async fn create_for_enable(&self) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("Creating new passkey for cloud backup enable");
        self.create_new_prf_key_with_mapper(map_enable_passkey_error).await
    }

    /// Discovers an existing passkey for wrapper repair or creates a new one
    pub async fn discover_or_create_for_wrapper_repair(
        &self,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        self.discover_or_create(PasskeyMaterialDiscoveryContext {
            fallback_context: "wrapper repair",
            attempt_message: "Attempting passkey discovery before creating new wrapper-repair passkey",
            discovered_message: "Discovered existing passkey for wrapper repair",
            cancelled_message: "User cancelled passkey discovery for wrapper repair",
            missing_message: "No existing passkey found for wrapper repair, creating new",
            failed_message: "Wrapper-repair discovery failed",
            create_for_enable: false,
        })
        .await
    }

    /// Discovers an existing passkey for enabling cloud backup or creates a new one
    pub async fn discover_or_create_for_enable(
        &self,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        self.discover_or_create(PasskeyMaterialDiscoveryContext {
            fallback_context: "cloud backup enable",
            attempt_message:
                "Attempting passkey discovery before creating new cloud backup enable passkey",
            discovered_message: "Discovered existing passkey for cloud backup enable",
            cancelled_message: "User cancelled passkey discovery for cloud backup enable",
            missing_message: "No existing passkey found for cloud backup enable, creating new",
            failed_message: "Cloud backup enable discovery failed",
            create_for_enable: true,
        })
        .await
    }

    async fn discover_or_create(
        &self,
        context: PasskeyMaterialDiscoveryContext,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("{}", context.attempt_message);
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
                info!("{}", context.discovered_message);

                Ok(UnpersistedPrfKey { prf_key, prf_salt, credential_id: discovered.credential_id })
            }
            Err(PasskeyError::UserCancelled) => {
                info!("{}", context.cancelled_message);
                Err(CloudBackupError::PasskeyDiscoveryCancelled)
            }
            Err(PasskeyError::NoCredentialFound) => {
                info!("{}", context.missing_message);
                self.create_for_context(context.create_for_enable).await
            }
            Err(PasskeyError::PrfUnsupportedProvider) => {
                Err(CloudBackupError::UnsupportedPasskeyProvider)
            }
            Err(error) => {
                let failed_message = context.failed_message;
                let fallback_context = context.fallback_context;
                warn!("{failed_message} ({error}), falling back to create for {fallback_context}");
                self.create_for_context(context.create_for_enable).await
            }
        }
    }

    async fn create_for_context(
        &self,
        create_for_enable: bool,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        if create_for_enable {
            return self.create_for_enable().await;
        }

        self.create_for_wrapper_repair().await
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

            match encrypted.backup_version() {
                Ok(MasterKeyBackupVersion::V1) => {}
                Err(_) => {
                    had_unsupported_versions = true;
                    continue;
                }
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
            Err(PasskeyError::PrfUnsupportedProvider) => {
                return Err(CloudBackupError::UnsupportedPasskeyProvider);
            }
            Err(error) => return Err(CloudBackupError::Passkey(error.to_string())),
        };

        let mut matches = Vec::new();
        let prf_key = prf_output_to_key(discovered.prf_output.clone())?;
        let try_first = cove_cspp::master_key_crypto::decrypt_master_key(first_encrypted, &prf_key);
        if let Ok(master_key) = try_first {
            matches.push(NamespaceMatch {
                namespace_id: namespace_id.clone(),
                master_key,
                prf_salt: first_encrypted.prf_salt,
                credential_id: discovered.credential_id.clone(),
            });
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
                Err(PasskeyError::UserCancelled) => {
                    if matches.is_empty() {
                        return Ok(NamespaceMatchOutcome::UserDeclined);
                    }

                    break;
                }
                Err(PasskeyError::PrfUnsupportedProvider) => {
                    return Err(CloudBackupError::UnsupportedPasskeyProvider);
                }
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
                matches.push(NamespaceMatch {
                    namespace_id: namespace_id.clone(),
                    master_key,
                    prf_salt: encrypted.prf_salt,
                    credential_id: discovered.credential_id.clone(),
                });
            }
        }

        if !matches.is_empty() {
            return Ok(NamespaceMatchOutcome::Matched(matches));
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
