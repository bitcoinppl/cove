use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cove_cspp::backup_data::PasskeyProviderHint;
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use tracing::{info, warn};

use super::authority::PasskeyRequestAuthority;
use super::authorization_retry::{
    PlatformAuthorizationRetrier, is_pre_presentation_platform_authorization_failure,
};
use super::prf_output_to_key;
use crate::manager::cloud_backup_manager::cloud_folders::{
    CloudFolder, CloudFolders, DownloadedWrapper, ListedBackups, ReadableFolder, WrapperRead,
};
use crate::manager::cloud_backup_manager::timing::log_elapsed;
use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupPasskeyHint};
use cove_device::cloud_storage::CloudStorageClient;

pub(crate) struct NamespaceMatch {
    pub(crate) namespace_id: String,
    pub(crate) master_key: cove_cspp::master_key::MasterKey,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
    /// Where the matched passkey was created, from the wrapper it opened
    pub(crate) provider_hint: Option<PasskeyProviderHint>,
    /// The wrapper the passkey opened, as downloaded, which a restore writes back to the
    /// namespace it activates
    pub(crate) opened_wrapper: DownloadedWrapper,
}

pub(crate) enum NamespaceMatchOutcome {
    Matched(Vec<NamespaceMatch>),
    UserDeclined,
    /// No passkey opened a listed backup
    Unopened(Unopened),
}

/// Why no passkey opened a listed backup, and whether one may still arrive
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unopened {
    pub(crate) reason: UnopenedReason,
    /// A folder's master key upload isn't confirmed yet, so it may be the backup a passkey opens
    ///
    /// Restore waits for it as it waits for a cloud it couldn't reach, while Enable reports what
    /// the other folders showed
    pub(crate) uploading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnopenedReason {
    /// No listed folder holds a master key a passkey could open: none is listed, or every one
    /// lacks a visible master key or is still uploading it, so no passkey was asked for
    NoBackupFiles,
    /// The passkey opened none of the readable backups
    PasskeyMismatch,
    /// The platform found no passkey for Cloud Backup on this device, such as one saved on another
    /// device that hasn't synced here yet, so no backup was tried
    NoPasskeyOnDevice,
    /// A listing, a folder or a passkey request failed before it could tell
    Inconclusive,
    /// Every backup with a master key uses a newer format than this app reads
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
    authority: PasskeyRequestAuthority,
}

/// One revision of one namespace's wrapper, tried at most once per session
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CandidateRevisionIdentity {
    namespace_id: String,
    wrapper_revision: String,
}

impl CandidateRevisionIdentity {
    fn of(folder: &ReadableFolder) -> Self {
        Self {
            namespace_id: folder.namespace_id.clone(),
            wrapper_revision: folder.wrapper.revision().to_owned(),
        }
    }
}

enum CredentialSelection {
    NotAttempted,
    NoCredentialFound,
    Selected(Vec<u8>),
}

/// What asking for a passkey on a readable folder showed, when it didn't open it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasskeyAttempt {
    Mismatch,
    /// Discovery found no passkey on this device, which says nothing about whether the backup's
    /// passkey exists elsewhere
    NoCredential,
    /// The platform failed before it presented the request
    Inconclusive,
}

/// Where the wrapper's passkey was created; a hint whose wrapper names another namespace is
/// dropped, since the hint only labels the passkey and must not fail the match
fn provider_hint(folder: &ReadableFolder) -> Option<PasskeyProviderHint> {
    folder
        .encrypted
        .passkey_provider_hint_for(&folder.namespace_id)
        .inspect_err(|error| warn!("Ignoring a passkey hint from a mismatched wrapper: {error}"))
        .ok()
        .flatten()
        .cloned()
}

fn registration_timestamp(folder: &ReadableFolder) -> u64 {
    folder
        .encrypted
        .passkey_provider_hint
        .as_ref()
        .map(|hint| hint.registered_at)
        .or(folder.encrypted.remote_metadata.updated_at)
        .unwrap_or_default()
}

pub(crate) struct NamespacePasskeyMatchSession {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
    authorization_retrier: PlatformAuthorizationRetrier,
    credential_selection: CredentialSelection,
    attempted_candidates: HashSet<CandidateRevisionIdentity>,
    /// Every folder the session's listings showed
    folders: Vec<CloudFolder>,
    attempts: Vec<PasskeyAttempt>,
    /// A listing failed, so a folder it would have shown is unknown
    listing_failed: bool,
    /// Never set for sessions that no restore operation can cancel
    cancellation: Arc<AtomicBool>,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher that presents passkey requests under `authority`
    pub(crate) fn new(
        cloud: &CloudStorageClient,
        passkey: &PasskeyAccess,
        authority: &PasskeyRequestAuthority,
    ) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone(), authority: authority.clone() }
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
            authorization_retrier: PlatformAuthorizationRetrier::new(self.authority.clone())
                .with_cancellation(cancellation.clone()),
            credential_selection: CredentialSelection::NotAttempted,
            attempted_candidates: HashSet::new(),
            folders: Vec::new(),
            attempts: Vec::new(),
            listing_failed: false,
            cancellation,
        }
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
        let folders = CloudFolders::read(
            &self.cloud,
            namespaces,
            WrapperRead::Uploaded,
            &AtomicBool::default(),
        )
        .await?;
        let passkey_hint = folders.passkey_hint().cloned();
        let outcome = match session.match_inspection(namespaces.len(), folders).await? {
            NamespaceMatchSnapshotOutcome::Matched(matches) => {
                NamespaceMatchOutcome::Matched(matches)
            }
            NamespaceMatchSnapshotOutcome::UserDeclined => NamespaceMatchOutcome::UserDeclined,
            NamespaceMatchSnapshotOutcome::Cancelled(matches) => {
                NamespaceMatchOutcome::Matched(matches)
            }
            // this session has no cancellation anyone can set
            NamespaceMatchSnapshotOutcome::OperationCancelled => {
                NamespaceMatchOutcome::Unopened(Unopened {
                    reason: UnopenedReason::Inconclusive,
                    uploading: false,
                })
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
}

impl NamespacePasskeyMatchSession {
    pub(crate) fn note_namespace_discovery_failure(&mut self) {
        self.listing_failed = true;
    }

    pub(crate) async fn match_snapshot(
        &mut self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }

        let folders =
            CloudFolders::read(&self.cloud, namespaces, WrapperRead::Uploaded, &self.cancellation)
                .await;
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }
        let folders = folders?;

        self.match_inspection(namespaces.len(), folders).await
    }

    async fn match_inspection(
        &mut self,
        namespace_count: usize,
        folders: CloudFolders,
    ) -> Result<NamespaceMatchSnapshotOutcome, CloudBackupError> {
        if cancellation_requested(&self.cancellation) {
            return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
        }

        // the session keeps every folder, so what it finishes with covers each listing
        let first_new = self.folders.len();
        self.folders.extend(folders.into_folders());
        let mut candidates = self.folders[first_new..]
            .iter()
            .filter_map(|folder| match folder {
                CloudFolder::Readable(folder) => Some(folder.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            registration_timestamp(right)
                .cmp(&registration_timestamp(left))
                .then_with(|| left.namespace_id.cmp(&right.namespace_id))
        });

        let new_candidate_count = candidates
            .iter()
            .filter(|candidate| {
                !self.attempted_candidates.contains(&CandidateRevisionIdentity::of(candidate))
            })
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
            let identity = CandidateRevisionIdentity::of(candidate);
            if self.attempted_candidates.contains(&identity) {
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
                    .await?;

                    if cancellation_requested(&self.cancellation) {
                        return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
                    }

                    let prf_output = match auth {
                        Ok(prf_output) => prf_output,
                        // a sheet that never showed says nothing about this candidate
                        Err(PasskeyError::NotCompleted | PasskeyError::NotReady { .. }) => {
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
                                self.attempted_candidates.insert(identity.clone());
                                return Ok(NamespaceMatchSnapshotOutcome::AuthenticationFailed {
                                    matches,
                                    error,
                                });
                            }
                            self.attempts.push(PasskeyAttempt::Inconclusive);
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
                    .await?;

                    if cancellation_requested(&self.cancellation) {
                        return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
                    }

                    let discovered = match discovery {
                        Ok(discovered) => discovered,
                        Err(PasskeyError::NotCompleted | PasskeyError::NotReady { .. }) => {
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
                            self.attempted_candidates.insert(identity.clone());
                            self.attempts.push(PasskeyAttempt::NoCredential);
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
                    self.attempted_candidates.insert(identity.clone());
                    continue;
                }
            };

            if cancellation_requested(&self.cancellation) {
                return Ok(NamespaceMatchSnapshotOutcome::OperationCancelled);
            }

            self.attempted_candidates.insert(identity);
            let prf_key = prf_output_to_key(prf_output)?;
            if let Ok(master_key) =
                cove_cspp::master_key_crypto::decrypt_master_key(&candidate.encrypted, &prf_key)
            {
                matches.push(NamespaceMatch {
                    provider_hint: provider_hint(candidate),
                    namespace_id: candidate.namespace_id.clone(),
                    master_key,
                    prf_salt: candidate.encrypted.prf_salt,
                    credential_id,
                    opened_wrapper: candidate.wrapper.clone(),
                });
            } else {
                self.attempts.push(PasskeyAttempt::Mismatch);
            }
        }

        if matches.is_empty() {
            Ok(NamespaceMatchSnapshotOutcome::Continue)
        } else {
            Ok(NamespaceMatchSnapshotOutcome::Matched(matches))
        }
    }

    pub(crate) fn finish(self) -> NamespaceMatchOutcome {
        let uploading =
            self.folders.iter().any(|folder| matches!(folder, CloudFolder::UploadPending));
        NamespaceMatchOutcome::Unopened(Unopened {
            reason: self.unopened_reason(uploading),
            uploading,
        })
    }

    fn unopened_reason(&self, uploading: bool) -> UnopenedReason {
        let listed = ListedBackups::of(&self.folders);
        if !self.listing_failed
            && matches!(listed, ListedBackups::None | ListedBackups::FilesUnavailable)
        {
            return UnopenedReason::NoBackupFiles;
        }
        if self.attempts.contains(&PasskeyAttempt::Mismatch) {
            return UnopenedReason::PasskeyMismatch;
        }

        // no passkey to try is definite whatever a read did, so it outranks a failed one
        if self.attempts.contains(&PasskeyAttempt::NoCredential) {
            return UnopenedReason::NoPasskeyOnDevice;
        }

        let unreadable =
            self.folders.iter().any(|folder| matches!(folder, CloudFolder::Unreadable(_)));
        if self.listing_failed
            || unreadable
            || self.attempts.contains(&PasskeyAttempt::Inconclusive)
        {
            return UnopenedReason::Inconclusive;
        }

        let readable = self.folders.iter().any(|folder| matches!(folder, CloudFolder::Readable(_)));
        let newer = self.folders.iter().any(|folder| matches!(folder, CloudFolder::NewerFormat));
        match (readable, newer, uploading) {
            // the backup still uploading may be one this app reads
            (false, true, true) => UnopenedReason::Inconclusive,
            (false, true, false) => UnopenedReason::UnsupportedVersions,
            _ => UnopenedReason::PasskeyMismatch,
        }
    }
}

fn cancellation_requested(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Acquire)
}
