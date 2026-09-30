use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cove_cspp::backup_data::{
    EncryptedMasterKeyBackup, MASTER_KEY_RECORD_ID, MasterKeyBackupVersion,
};
use cove_device::cloud_storage::{CloudBackupUploadStatus, CloudStorageClient, CloudStorageError};
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use futures::stream::{self, TryStreamExt as _};
use tracing::{info, warn};

use super::authorization_retry::{
    PlatformAuthorizationRetrier, is_pre_presentation_platform_authorization_failure,
};
use super::prf_output_to_key;
use crate::manager::cloud_backup_manager::timing::log_elapsed;
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupPasskeyHint,
    master_key_wrapper_revision_hash,
};

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
    /// The user cancelled after matching one or more namespaces
    Cancelled(Vec<NamespaceMatch>),
    /// The restore operation was cancelled while this snapshot was in flight
    OperationCancelled,
    /// Targeted authentication failed after the system prompt was shown
    ///
    /// Matching stops so the user is not prompted again, and the namespaces matched
    /// earlier in this snapshot stay restorable. The caller owns the matches from
    /// earlier snapshots, so it decides whether the failure is fatal
    AuthenticationFailed {
        matches: Vec<NamespaceMatch>,
        error: PasskeyError,
    },
    Continue,
}

impl NamespaceMatchSnapshotOutcome {
    /// Ends matching on a terminal authentication failure, which is only an error
    /// when no namespace matched before it
    pub(crate) fn authentication_failure_result(
        matches: Vec<NamespaceMatch>,
        error: PasskeyError,
    ) -> Result<Vec<NamespaceMatch>, CloudBackupError> {
        if matches.is_empty() {
            return Err(CloudBackupError::passkey(error));
        }

        warn!(
            "Passkey authentication failed after {} namespace match(es), keeping them: {error}",
            matches.len()
        );
        Ok(matches)
    }
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

struct LoadedNamespaceWrapper {
    encrypted: EncryptedMasterKeyBackup,
    revision: WrapperRevisionDigest,
}

enum NamespaceWrapperLoad {
    Loaded(Box<LoadedNamespaceWrapper>),
    Missing,
    /// The wrapper could not be downloaded or decoded, already logged where it failed
    Failed,
}

enum NamespaceCandidateLoad {
    Uploaded { namespace_id: String, wrapper: NamespaceWrapperLoad },
    Pending,
    Missing,
    UploadStateFailed(CloudStorageError),
}

/// Cloud data loaded once for one namespace inspection
pub(crate) struct NamespacePasskeyCandidateSnapshot {
    candidates: Vec<NamespacePasskeyCandidate>,
    has_supported_candidate: bool,
    candidate_outcomes: Vec<NamespaceCandidateOutcome>,
    best_passkey_hint: Option<CloudBackupPasskeyHint>,
}

impl NamespacePasskeyCandidateSnapshot {
    pub(crate) fn best_passkey_hint(&self) -> Option<CloudBackupPasskeyHint> {
        self.best_passkey_hint.clone()
    }
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
    saw_supported_candidate: bool,
    candidate_outcomes: Vec<NamespaceCandidateOutcome>,
    /// Never set for sessions that no restore operation can cancel
    cancellation: Arc<AtomicBool>,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher from cloud and passkey service handles
    pub(crate) fn new(cloud: &CloudStorageClient, passkey: &PasskeyAccess) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone() }
    }

    pub(crate) fn start_session(&self) -> NamespacePasskeyMatchSession {
        self.start_session_with_cancellation(Arc::default())
    }

    pub(crate) fn start_session_with_cancellation(
        &self,
        cancellation: Arc<AtomicBool>,
    ) -> NamespacePasskeyMatchSession {
        NamespacePasskeyMatchSession {
            cloud: self.cloud.clone(),
            passkey: self.passkey.clone(),
            authorization_retrier: PlatformAuthorizationRetrier::with_cancellation(
                cancellation.clone(),
            ),
            credential_selection: CredentialSelection::NotAttempted,
            attempted_candidates: HashSet::new(),
            saw_supported_candidate: false,
            candidate_outcomes: Vec::new(),
            cancellation,
        }
    }

    /// Loads cloud wrappers once and derives both display hints and candidates
    pub(crate) async fn inspect_namespaces(
        &self,
        namespaces: &[String],
    ) -> Result<NamespacePasskeyCandidateSnapshot, CloudBackupError> {
        let never_cancelled = AtomicBool::new(false);
        load_candidate_snapshot(&self.cloud, namespaces, &never_cancelled).await
    }

    /// Downloads candidate wrappers and tries the selected passkey against each PRF salt
    pub(crate) async fn match_namespaces(
        &self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchOutcome, CloudBackupError> {
        self.match_namespaces_with_hint(namespaces).await.map(|(outcome, _)| outcome)
    }

    pub(crate) async fn match_namespaces_with_hint(
        &self,
        namespaces: &[String],
    ) -> Result<(NamespaceMatchOutcome, Option<CloudBackupPasskeyHint>), CloudBackupError> {
        let mut session = self.start_session();
        let snapshot = self.inspect_namespaces(namespaces).await?;
        let passkey_hint = snapshot.best_passkey_hint();
        let outcome = match session.match_inspection(namespaces.len(), snapshot).await? {
            NamespaceMatchSnapshotOutcome::Matched(matches) => {
                NamespaceMatchOutcome::Matched(matches)
            }
            NamespaceMatchSnapshotOutcome::UserDeclined => NamespaceMatchOutcome::UserDeclined,
            NamespaceMatchSnapshotOutcome::Cancelled(matches) => {
                NamespaceMatchOutcome::Matched(matches)
            }
            NamespaceMatchSnapshotOutcome::OperationCancelled => {
                NamespaceMatchOutcome::Inconclusive
            }
            NamespaceMatchSnapshotOutcome::AuthenticationFailed { matches, error } => {
                NamespaceMatchOutcome::Matched(
                    NamespaceMatchSnapshotOutcome::authentication_failure_result(matches, error)?,
                )
            }
            NamespaceMatchSnapshotOutcome::Continue => session.finish(),
        };

        Ok((outcome, passkey_hint))
    }

    pub(crate) async fn passkey_hint_for_namespaces(
        &self,
        namespaces: &[String],
    ) -> Option<CloudBackupPasskeyHint> {
        self.inspect_namespaces(namespaces).await.ok()?.best_passkey_hint()
    }
}

impl NamespacePasskeyMatchSession {
    pub(crate) fn note_namespace_discovery_failure(&mut self) {
        self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
    }

    pub(crate) fn saw_supported_candidate(&self) -> bool {
        self.saw_supported_candidate
    }

    pub(crate) async fn match_snapshot(
        &mut self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }

        let snapshot_result =
            load_candidate_snapshot(&self.cloud, namespaces, &self.cancellation).await;
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }
        let snapshot = snapshot_result?;

        self.match_inspection(namespaces.len(), snapshot).await
    }

    async fn match_inspection(
        &mut self,
        namespace_count: usize,
        snapshot: NamespacePasskeyCandidateSnapshot,
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }
        self.candidate_outcomes.extend(snapshot.candidate_outcomes);
        self.saw_supported_candidate |= snapshot.has_supported_candidate;

        let mut candidates = snapshot.candidates;
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
            namespace_count,
            candidates.len(),
            new_candidate_count,
            self.attempted_candidates.len()
        );

        let mut matches = Vec::new();
        for candidate in candidates {
            if cancellation_requested(&self.cancellation) {
                return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
            }
            if self.attempted_candidates.contains(&candidate.identity) {
                continue;
            }

            let (credential_id, prf_output) = match &self.credential_selection {
                CredentialSelection::Selected(credential_id) => {
                    let auth = log_elapsed(
                        "Passkey targeted authentication",
                        self.authorization_retrier.authenticate(
                            &self.passkey,
                            credential_id,
                            candidate.encrypted.prf_salt,
                        ),
                    )
                    .await;

                    if cancellation_requested(&self.cancellation) {
                        return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
                    }

                    let prf_output = match auth {
                        Ok(prf_output) => prf_output,
                        Err(PasskeyError::UserCancelled) => {
                            return Ok(if matches.is_empty() {
                                NamespaceMatchSnapshotOutcome::UserDeclined
                            } else {
                                NamespaceMatchSnapshotOutcome::Cancelled(matches)
                            });
                        }
                        Err(PasskeyError::PrfUnsupportedProvider) => {
                            return Err(CloudBackupError::UnsupportedPasskeyProvider);
                        }
                        Err(error) => {
                            warn!(
                                "Failed targeted passkey auth for new or changed cloud backup wrapper: {error}"
                            );
                            if !is_pre_presentation_platform_authorization_failure(&error) {
                                self.attempted_candidates.insert(candidate.identity.clone());
                                return Ok(NamespaceMatchSnapshotOutcome::AuthenticationFailed {
                                    matches,
                                    error,
                                });
                            }
                            self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                            continue;
                        }
                    };

                    (credential_id.clone(), prf_output)
                }
                CredentialSelection::NotAttempted => {
                    let started_at = Instant::now();
                    let discovery = log_elapsed(
                        "Passkey discovery authentication",
                        self.authorization_retrier
                            .discover(&self.passkey, candidate.encrypted.prf_salt),
                    )
                    .await;

                    if cancellation_requested(&self.cancellation) {
                        return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
                    }

                    let discovered = match discovery {
                        Ok(discovered) => discovered,
                        Err(PasskeyError::UserCancelled) => {
                            // the platform reports a cancellation and an authenticator failure
                            // the same way, so record what the attempt looked like
                            info!(
                                "Passkey discovery cancelled elapsed_ms={} underlying_error=none",
                                started_at.elapsed().as_millis()
                            );
                            return Ok(NamespaceMatchSnapshotOutcome::UserDeclined);
                        }
                        Err(PasskeyError::NoCredentialFound) => {
                            self.credential_selection = CredentialSelection::NoCredentialFound;
                            self.attempted_candidates.insert(candidate.identity.clone());
                            self.candidate_outcomes
                                .push(NamespaceCandidateOutcome::PasskeyMismatch);
                            continue;
                        }
                        Err(PasskeyError::PrfUnsupportedProvider) => {
                            return Err(CloudBackupError::UnsupportedPasskeyProvider);
                        }
                        Err(error) => return Err(CloudBackupError::passkey(error)),
                    };

                    info!("Passkey discovery selected a credential");
                    self.credential_selection =
                        CredentialSelection::Selected(discovered.credential_id.clone());

                    (discovered.credential_id, discovered.prf_output)
                }
                CredentialSelection::NoCredentialFound => {
                    self.attempted_candidates.insert(candidate.identity.clone());
                    continue;
                }
            };

            if cancellation_requested(&self.cancellation) {
                return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
            }

            self.attempted_candidates.insert(candidate.identity.clone());
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
        if self.candidate_outcomes.contains(&NamespaceCandidateOutcome::PendingUpload) {
            return NamespaceMatchOutcome::Inconclusive;
        }
        if self.candidate_outcomes.contains(&NamespaceCandidateOutcome::PasskeyMismatch) {
            return NamespaceMatchOutcome::NoMatch;
        }
        if self.candidate_outcomes.contains(&NamespaceCandidateOutcome::Inconclusive) {
            return NamespaceMatchOutcome::Inconclusive;
        }
        if !self.saw_supported_candidate
            && self.candidate_outcomes.contains(&NamespaceCandidateOutcome::UnsupportedVersion)
        {
            return NamespaceMatchOutcome::UnsupportedVersions;
        }

        NamespaceMatchOutcome::NoMatch
    }
}

async fn load_candidate_snapshot(
    cloud: &CloudStorageClient,
    namespaces: &[String],
    cancellation: &AtomicBool,
) -> Result<NamespacePasskeyCandidateSnapshot, CloudBackupError> {
    let mut seen_namespaces = HashSet::new();
    let unique_namespaces = namespaces
        .iter()
        .filter(|namespace| seen_namespaces.insert((*namespace).clone()))
        .cloned()
        .collect::<Vec<_>>();
    let mut loads =
        stream::iter(unique_namespaces.into_iter().enumerate().map(|(index, namespace_id)| {
            let cloud = cloud.clone();

            Ok(async move {
                Ok::<_, CloudBackupError>((
                    index,
                    load_namespace_candidate(&cloud, namespace_id, cancellation).await?,
                ))
            })
        }))
        .try_buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    loads.sort_by_key(|(index, _)| *index);

    let mut snapshot = NamespacePasskeyCandidateSnapshot {
        candidates: Vec::with_capacity(loads.len()),
        has_supported_candidate: false,
        candidate_outcomes: Vec::new(),
        best_passkey_hint: None,
    };

    for (_, load) in loads {
        snapshot.add_load(load);
    }

    Ok(snapshot)
}

async fn load_namespace_candidate(
    cloud: &CloudStorageClient,
    namespace_id: String,
    cancellation: &AtomicBool,
) -> Result<NamespaceCandidateLoad, CloudBackupError> {
    if cancellation_requested(cancellation) {
        return Err(CloudBackupError::Cancelled);
    }

    let upload_state = log_elapsed(
        "Passkey candidate upload-state read",
        cloud.is_backup_uploaded(namespace_id.clone(), MASTER_KEY_RECORD_ID.to_string()),
    )
    .await;
    if cancellation_requested(cancellation) {
        return Err(CloudBackupError::Cancelled);
    }

    let upload_state = match upload_state {
        Ok(upload_state) => upload_state,
        Err(error @ CloudStorageError::AuthorizationRequired(_)) => return Err(error.into()),
        Err(error) => return Ok(NamespaceCandidateLoad::UploadStateFailed(error)),
    };

    match upload_state {
        CloudBackupUploadStatus::Pending => Ok(NamespaceCandidateLoad::Pending),
        CloudBackupUploadStatus::NotFound => Ok(NamespaceCandidateLoad::Missing),
        CloudBackupUploadStatus::Uploaded => {
            if cancellation_requested(cancellation) {
                return Err(CloudBackupError::Cancelled);
            }

            let wrapper = log_elapsed(
                "Passkey candidate wrapper read",
                cloud.download_master_key_backup(namespace_id.clone()),
            )
            .await;
            if cancellation_requested(cancellation) {
                return Err(CloudBackupError::Cancelled);
            }

            let wrapper = match wrapper {
                Ok(master_json) => {
                    match serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) {
                        Ok(encrypted) => {
                            NamespaceWrapperLoad::Loaded(Box::new(LoadedNamespaceWrapper {
                                encrypted,
                                revision: WrapperRevisionDigest(master_key_wrapper_revision_hash(
                                    &master_json,
                                )),
                            }))
                        }
                        Err(error) => {
                            warn!("Failed to deserialize cloud backup master key: {error}");
                            NamespaceWrapperLoad::Failed
                        }
                    }
                }
                Err(CloudStorageError::NotFound(_)) => NamespaceWrapperLoad::Missing,
                Err(error @ CloudStorageError::AuthorizationRequired(_)) => {
                    return Err(error.into());
                }
                Err(error) => {
                    warn!("Failed to download cloud backup master key: {error}");
                    NamespaceWrapperLoad::Failed
                }
            };

            Ok(NamespaceCandidateLoad::Uploaded { namespace_id, wrapper })
        }
    }
}

fn cancellation_requested(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Acquire)
}

impl NamespacePasskeyCandidateSnapshot {
    fn add_load(&mut self, load: NamespaceCandidateLoad) {
        match load {
            NamespaceCandidateLoad::Uploaded { namespace_id, wrapper } => {
                info!("Passkey candidate wrapper upload_state=uploaded");
                self.add_uploaded_candidate(namespace_id, wrapper);
            }
            NamespaceCandidateLoad::Pending => {
                info!("Passkey candidate wrapper upload_state=pending");
                self.candidate_outcomes.push(NamespaceCandidateOutcome::PendingUpload);
            }
            NamespaceCandidateLoad::Missing => {
                info!("Ignoring stale cloud backup namespace with no master key wrapper");
                self.candidate_outcomes.push(NamespaceCandidateOutcome::Missing);
            }
            NamespaceCandidateLoad::UploadStateFailed(error) => {
                warn!("Failed to read passkey candidate upload state: {error}");
                self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
            }
        }
    }

    fn add_passkey_hint(&mut self, encrypted: &EncryptedMasterKeyBackup, namespace_id: &str) {
        let Ok(Some(hint)) =
            CloudBackupPasskeyHint::from_master_key_wrapper(encrypted, namespace_id)
        else {
            return;
        };

        if self.best_passkey_hint.as_ref().is_none_or(|current| hint.is_newer_than(current)) {
            self.best_passkey_hint = Some(hint);
        }
    }

    fn add_uploaded_candidate(&mut self, namespace_id: String, wrapper: NamespaceWrapperLoad) {
        let loaded_wrapper = match wrapper {
            NamespaceWrapperLoad::Loaded(wrapper) => {
                // unsupported wrapper versions still name the passkey that created them
                self.add_passkey_hint(&wrapper.encrypted, &namespace_id);
                wrapper
            }
            NamespaceWrapperLoad::Missing => {
                info!("Ignoring stale cloud backup namespace with no master key wrapper");
                self.candidate_outcomes.push(NamespaceCandidateOutcome::Missing);
                return;
            }
            NamespaceWrapperLoad::Failed => {
                self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
                return;
            }
        };

        let LoadedNamespaceWrapper { encrypted, revision } = *loaded_wrapper;
        if !matches!(encrypted.backup_version(), Ok(MasterKeyBackupVersion::V1)) {
            self.candidate_outcomes.push(NamespaceCandidateOutcome::UnsupportedVersion);
            return;
        }
        if encrypted.remote_metadata.normalized_master_key(&namespace_id).is_err() {
            self.candidate_outcomes.push(NamespaceCandidateOutcome::Inconclusive);
            return;
        }

        let identity = CandidateRevisionIdentity { namespace_id, wrapper_revision: revision };
        self.has_supported_candidate = true;
        self.candidates.push(NamespacePasskeyCandidate { identity, encrypted });
    }
}
