use std::collections::HashSet;

use cove_cspp::backup_data::{
    EncryptedMasterKeyBackup, MASTER_KEY_RECORD_ID, MasterKeyBackupVersion,
};
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use sha2::{Digest as _, Sha256};
use tracing::{info, warn};

use super::authorization_retry::PlatformAuthorizationRetrier;
use super::prf_output_to_key;
use crate::manager::cloud_backup_manager::{CloudBackupError, master_key_wrapper_revision_hash};

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

pub(crate) enum NamespaceMatchSnapshotOutcome {
    Matched(Vec<NamespaceMatch>),
    UserDeclined,
    Continue,
}

pub(crate) struct NamespacePasskeyMatcher {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WrapperRevisionDigest(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CandidateRevisionIdentity {
    namespace_id: String,
    wrapper_revision: WrapperRevisionDigest,
}

struct NamespacePasskeyCandidate {
    identity: CandidateRevisionIdentity,
    encrypted: EncryptedMasterKeyBackup,
}

impl NamespacePasskeyCandidate {
    fn registration_timestamp(&self) -> u64 {
        self.encrypted
            .passkey_provider_hint
            .as_ref()
            .map(|hint| hint.registered_at)
            .or(self.encrypted.remote_metadata.updated_at)
            .unwrap_or_default()
    }
}

pub(crate) struct NamespacePasskeyMatchSession {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
    authorization_retrier: PlatformAuthorizationRetrier,
    selected_credential_id: Option<Vec<u8>>,
    discoverable_assertion_completed: bool,
    attempted_candidates: HashSet<CandidateRevisionIdentity>,
    had_inconclusive_cloud_failure: bool,
    had_unsupported_versions: bool,
    saw_supported_candidate: bool,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher from cloud and passkey service handles
    pub(crate) fn new(cloud: &CloudStorageClient, passkey: &PasskeyAccess) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone() }
    }

    pub(crate) fn start_session(&self) -> NamespacePasskeyMatchSession {
        NamespacePasskeyMatchSession {
            cloud: self.cloud.clone(),
            passkey: self.passkey.clone(),
            authorization_retrier: PlatformAuthorizationRetrier::new(),
            selected_credential_id: None,
            discoverable_assertion_completed: false,
            attempted_candidates: HashSet::new(),
            had_inconclusive_cloud_failure: false,
            had_unsupported_versions: false,
            saw_supported_candidate: false,
        }
    }

    /// Downloads candidate wrappers and tries the selected passkey against each PRF salt
    pub(crate) async fn match_namespaces(
        &self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchOutcome, CloudBackupError> {
        let mut session = self.start_session();
        match session.match_snapshot(namespaces).await? {
            NamespaceMatchSnapshotOutcome::Matched(matches) => {
                Ok(NamespaceMatchOutcome::Matched(matches))
            }
            NamespaceMatchSnapshotOutcome::UserDeclined => Ok(NamespaceMatchOutcome::UserDeclined),
            NamespaceMatchSnapshotOutcome::Continue => Ok(session.finish()),
        }
    }
}

impl NamespacePasskeyMatchSession {
    pub(crate) fn note_namespace_discovery_failure(&mut self) {
        self.had_inconclusive_cloud_failure = true;
    }

    pub(crate) fn saw_supported_candidate(&self) -> bool {
        self.saw_supported_candidate
    }

    pub(crate) async fn match_snapshot(
        &mut self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        let mut candidates = self.download_candidates(namespaces).await;
        candidates.sort_by(|left, right| {
            right
                .registration_timestamp()
                .cmp(&left.registration_timestamp())
                .then_with(|| left.identity.namespace_id.cmp(&right.identity.namespace_id))
        });

        let new_candidate_count = candidates
            .iter()
            .filter(|candidate| !self.attempted_candidates.contains(&candidate.identity))
            .count();
        info!(
            "Passkey candidate refresh namespace_count={} usable_count={} new_or_changed_count={} attempted_count={}",
            namespaces.len(),
            candidates.len(),
            new_candidate_count,
            self.attempted_candidates.len()
        );

        let mut matches = Vec::new();
        for candidate in candidates {
            if !self.attempted_candidates.insert(candidate.identity.clone()) {
                continue;
            }

            if self.discoverable_assertion_completed && self.selected_credential_id.is_none() {
                continue;
            }

            let (credential_id, prf_output) = match &self.selected_credential_id {
                Some(credential_id) => {
                    let auth = self
                        .authorization_retrier
                        .authenticate(&self.passkey, credential_id, candidate.encrypted.prf_salt)
                        .await;
                    let prf_output = match auth {
                        Ok(prf_output) => prf_output,
                        Err(PasskeyError::UserCancelled) => {
                            return Ok(NamespaceMatchSnapshotOutcome::UserDeclined);
                        }
                        Err(PasskeyError::PrfUnsupportedProvider) => {
                            return Err(CloudBackupError::UnsupportedPasskeyProvider);
                        }
                        Err(error) => {
                            warn!(
                                "Failed targeted passkey auth for new or changed cloud backup wrapper: {error}"
                            );
                            self.had_inconclusive_cloud_failure = true;
                            continue;
                        }
                    };

                    (credential_id.clone(), prf_output)
                }
                None => {
                    self.discoverable_assertion_completed = true;
                    let discovery = self
                        .authorization_retrier
                        .discover(&self.passkey, candidate.encrypted.prf_salt)
                        .await;
                    let discovered = match discovery {
                        Ok(discovered) => discovered,
                        Err(PasskeyError::UserCancelled) => {
                            return Ok(NamespaceMatchSnapshotOutcome::UserDeclined);
                        }
                        Err(PasskeyError::NoCredentialFound) => {
                            continue;
                        }
                        Err(PasskeyError::PrfUnsupportedProvider) => {
                            return Err(CloudBackupError::UnsupportedPasskeyProvider);
                        }
                        Err(error) => return Err(CloudBackupError::passkey(error)),
                    };

                    info!(
                        "Passkey discovery selected credential fingerprint={}",
                        credential_id_fingerprint(&discovered.credential_id)
                    );
                    self.selected_credential_id = Some(discovered.credential_id.clone());

                    (discovered.credential_id, discovered.prf_output)
                }
            };

            let prf_key = prf_output_to_key(prf_output)?;
            if let Ok(master_key) =
                cove_cspp::master_key_crypto::decrypt_master_key(&candidate.encrypted, &prf_key)
            {
                matches.push(NamespaceMatch {
                    namespace_id: candidate.identity.namespace_id,
                    master_key,
                    prf_salt: candidate.encrypted.prf_salt,
                    credential_id,
                });
            }
        }

        if matches.is_empty() {
            Ok(NamespaceMatchSnapshotOutcome::Continue)
        } else {
            Ok(NamespaceMatchSnapshotOutcome::Matched(matches))
        }
    }

    pub(crate) fn finish(self) -> NamespaceMatchOutcome {
        if self.had_inconclusive_cloud_failure {
            return NamespaceMatchOutcome::Inconclusive;
        }

        if !self.saw_supported_candidate && self.had_unsupported_versions {
            return NamespaceMatchOutcome::UnsupportedVersions;
        }

        NamespaceMatchOutcome::NoMatch
    }

    async fn download_candidates(
        &mut self,
        namespaces: &[String],
    ) -> Vec<NamespacePasskeyCandidate> {
        let mut seen_namespaces = HashSet::new();
        let mut candidates = Vec::with_capacity(namespaces.len());

        for namespace in namespaces {
            if !seen_namespaces.insert(namespace) {
                continue;
            }

            match self
                .cloud
                .is_backup_uploaded(namespace.clone(), MASTER_KEY_RECORD_ID.to_string())
                .await
            {
                Ok(true) => {
                    info!("Passkey candidate wrapper upload_state=uploaded")
                }
                Ok(false) => {
                    info!("Passkey candidate wrapper upload_state=pending");
                    self.had_inconclusive_cloud_failure = true;
                    continue;
                }
                Err(error) => {
                    warn!("Failed to read passkey candidate upload state: {error}");
                    self.had_inconclusive_cloud_failure = true;
                    continue;
                }
            }

            let master_json = match self.cloud.download_master_key_backup(namespace.clone()).await {
                Ok(master_json) => master_json,
                Err(cove_device::cloud_storage::CloudStorageError::NotFound(_)) => {
                    info!("Ignoring stale cloud backup namespace with no master key wrapper");
                    continue;
                }
                Err(error) => {
                    warn!("Failed to download cloud backup master key: {error}");
                    self.had_inconclusive_cloud_failure = true;
                    continue;
                }
            };

            let encrypted = match serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) {
                Ok(encrypted) => encrypted,
                Err(error) => {
                    warn!("Failed to deserialize cloud backup master key: {error}");
                    self.had_inconclusive_cloud_failure = true;
                    continue;
                }
            };

            match encrypted.backup_version() {
                Ok(MasterKeyBackupVersion::V1) => {}
                Err(_) => {
                    self.had_unsupported_versions = true;
                    continue;
                }
            }
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                self.had_inconclusive_cloud_failure = true;
                continue;
            }

            self.saw_supported_candidate = true;
            candidates.push(NamespacePasskeyCandidate {
                identity: CandidateRevisionIdentity {
                    namespace_id: namespace.clone(),
                    wrapper_revision: WrapperRevisionDigest(master_key_wrapper_revision_hash(
                        &master_json,
                    )),
                },
                encrypted,
            });
        }

        candidates
    }
}

fn credential_id_fingerprint(credential_id: &[u8]) -> String {
    let digest = Sha256::digest(credential_id);
    format!("{} len={}", hex::encode(&digest[..6]), credential_id.len())
}
