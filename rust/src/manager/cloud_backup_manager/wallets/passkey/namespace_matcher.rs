use std::collections::HashSet;

use cove_cspp::backup_data::{
    EncryptedMasterKeyBackup, MASTER_KEY_RECORD_ID, MasterKeyBackupVersion,
};
use cove_device::cloud_storage::{CloudBackupUploadStatus, CloudStorageClient, CloudStorageError};
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

enum CredentialSelection {
    NotAttempted,
    NoCredentialFound,
    Selected(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamespaceCandidateOutcome {
    Missing,
    PendingUpload,
    PasskeyMismatch,
    UnsupportedVersion,
    Inconclusive,
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
    credential_selection: CredentialSelection,
    attempted_candidates: HashSet<CandidateRevisionIdentity>,
    supported_candidates: HashSet<CandidateRevisionIdentity>,
    candidate_outcomes: Vec<NamespaceCandidateOutcome>,
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
            credential_selection: CredentialSelection::NotAttempted,
            attempted_candidates: HashSet::new(),
            supported_candidates: HashSet::new(),
            candidate_outcomes: Vec::new(),
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
        self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
    }

    pub(crate) fn saw_supported_candidate(&self) -> bool {
        !self.supported_candidates.is_empty()
    }

    pub(crate) async fn match_snapshot(
        &mut self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        let mut candidates = self.download_candidates(namespaces).await?;
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

            let (credential_id, prf_output) = match &self.credential_selection {
                CredentialSelection::Selected(credential_id) => {
                    let auth = self
                        .authorization_retrier
                        .authenticate(&self.passkey, credential_id, candidate.encrypted.prf_salt)
                        .await;
                    let prf_output = match auth {
                        Ok(prf_output) => prf_output,
                        Err(PasskeyError::UserCancelled) => {
                            return Ok(if matches.is_empty() {
                                NamespaceMatchSnapshotOutcome::UserDeclined
                            } else {
                                NamespaceMatchSnapshotOutcome::Matched(matches)
                            });
                        }
                        Err(PasskeyError::PrfUnsupportedProvider) => {
                            return Err(CloudBackupError::UnsupportedPasskeyProvider);
                        }
                        Err(error) => {
                            warn!(
                                "Failed targeted passkey auth for new or changed cloud backup wrapper: {error}"
                            );
                            self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                            continue;
                        }
                    };

                    (credential_id.clone(), prf_output)
                }
                CredentialSelection::NotAttempted => {
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
                            self.credential_selection = CredentialSelection::NoCredentialFound;
                            self.candidate_outcomes
                                .push(NamespaceCandidateOutcome::PasskeyMismatch);
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
                    self.credential_selection =
                        CredentialSelection::Selected(discovered.credential_id.clone());

                    (discovered.credential_id, discovered.prf_output)
                }
                CredentialSelection::NoCredentialFound => continue,
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
            } else {
                self.candidate_outcomes.push(NamespaceCandidateOutcome::PasskeyMismatch);
            }
        }

        if matches.is_empty() {
            Ok(NamespaceMatchSnapshotOutcome::Continue)
        } else {
            Ok(NamespaceMatchSnapshotOutcome::Matched(matches))
        }
    }

    pub(crate) fn finish(self) -> NamespaceMatchOutcome {
        if self.candidate_outcomes.contains(&NamespaceCandidateOutcome::PasskeyMismatch) {
            return NamespaceMatchOutcome::NoMatch;
        }
        if self.candidate_outcomes.iter().any(|outcome| {
            matches!(
                outcome,
                NamespaceCandidateOutcome::PendingUpload | NamespaceCandidateOutcome::Inconclusive
            )
        }) {
            return NamespaceMatchOutcome::Inconclusive;
        }
        if self.supported_candidates.is_empty()
            && self.candidate_outcomes.contains(&NamespaceCandidateOutcome::UnsupportedVersion)
        {
            return NamespaceMatchOutcome::UnsupportedVersions;
        }

        NamespaceMatchOutcome::NoMatch
    }

    async fn download_candidates(
        &mut self,
        namespaces: &[String],
    ) -> Result<Vec<NamespacePasskeyCandidate>, CloudBackupError> {
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
                Ok(CloudBackupUploadStatus::Uploaded) => {
                    info!("Passkey candidate wrapper upload_state=uploaded")
                }
                Ok(CloudBackupUploadStatus::Pending) => {
                    info!("Passkey candidate wrapper upload_state=pending");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::PendingUpload);
                    continue;
                }
                Ok(CloudBackupUploadStatus::NotFound) => {
                    info!("Ignoring stale cloud backup namespace with no master key wrapper");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::Missing);
                    continue;
                }
                Err(error @ CloudStorageError::AuthorizationRequired(_)) => {
                    return Err(error.into());
                }
                Err(error) => {
                    warn!("Failed to read passkey candidate upload state: {error}");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                    continue;
                }
            }

            let master_json = match self.cloud.download_master_key_backup(namespace.clone()).await {
                Ok(master_json) => master_json,
                Err(CloudStorageError::NotFound(_)) => {
                    info!("Ignoring stale cloud backup namespace with no master key wrapper");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::Missing);
                    continue;
                }
                Err(error @ CloudStorageError::AuthorizationRequired(_)) => {
                    return Err(error.into());
                }
                Err(error) => {
                    warn!("Failed to download cloud backup master key: {error}");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                    continue;
                }
            };

            let encrypted = match serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) {
                Ok(encrypted) => encrypted,
                Err(error) => {
                    warn!("Failed to deserialize cloud backup master key: {error}");
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                    continue;
                }
            };

            match encrypted.backup_version() {
                Ok(MasterKeyBackupVersion::V1) => {}
                Err(_) => {
                    self.candidate_outcomes.push(NamespaceCandidateOutcome::UnsupportedVersion);
                    continue;
                }
            }
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                continue;
            }

            let identity = CandidateRevisionIdentity {
                namespace_id: namespace.clone(),
                wrapper_revision: WrapperRevisionDigest(master_key_wrapper_revision_hash(
                    &master_json,
                )),
            };
            self.supported_candidates.insert(identity.clone());
            candidates.push(NamespacePasskeyCandidate { identity, encrypted });
        }

        Ok(candidates)
    }
}

fn credential_id_fingerprint(credential_id: &[u8]) -> String {
    let digest = Sha256::digest(credential_id);
    format!("{} len={}", hex::encode(&digest[..6]), credential_id.len())
}
