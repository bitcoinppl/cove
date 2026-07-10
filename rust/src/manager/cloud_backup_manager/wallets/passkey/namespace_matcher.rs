use cove_cspp::backup_data::{EncryptedMasterKeyBackup, MasterKeyBackupVersion};
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use tracing::warn;

use super::authorization_retry::PlatformAuthorizationRetrier;
use super::prf_output_to_key;
use crate::manager::cloud_backup_manager::CloudBackupError;

pub(crate) struct NamespaceMatch {
    pub(crate) namespace_id: String,
    pub(crate) master_key: cove_cspp::master_key::MasterKey,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
}

pub(crate) enum NamespaceMatchOutcome {
    Matched(Vec<NamespaceMatch>),
    UserDeclined,
    NoMatch,
    Inconclusive,
    UnsupportedVersions,
}

/// Matches a discoverable passkey against candidate cloud backup namespaces
pub(crate) struct NamespacePasskeyMatcher {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher from cloud and passkey service handles
    pub(crate) fn new(cloud: &CloudStorageClient, passkey: &PasskeyAccess) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone() }
    }

    /// Downloads candidate wrappers and tries the selected passkey against each PRF salt
    pub(crate) async fn match_namespaces(
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
                        warn!("Failed to download cloud backup master key: {error}");
                        had_download_failures = true;
                    },
                )
            else {
                continue;
            };

            let Ok(encrypted) = serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json)
                .inspect_err(|error| {
                    warn!("Failed to deserialize cloud backup master key: {error}");
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
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                had_download_failures = true;
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
        let retrier = PlatformAuthorizationRetrier::new();
        let discovery = retrier.discover(&self.passkey, first_encrypted.prf_salt).await;
        let discovered = match discovery {
            Ok(discovered) => discovered,
            Err(PasskeyError::UserCancelled) => return Ok(NamespaceMatchOutcome::UserDeclined),
            Err(PasskeyError::NoCredentialFound) => return Ok(NamespaceMatchOutcome::NoMatch),
            Err(PasskeyError::PrfUnsupportedProvider) => {
                return Err(CloudBackupError::UnsupportedPasskeyProvider);
            }
            Err(error) => return Err(CloudBackupError::passkey(error)),
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
            let prf_output_result = retrier
                .authenticate(&self.passkey, &discovered.credential_id, encrypted.prf_salt)
                .await;

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
                    warn!("Failed targeted passkey auth for cloud backup namespace: {error}");
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
