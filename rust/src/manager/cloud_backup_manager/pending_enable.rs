use cove_cspp::backup_data::PasskeyProviderHint;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::CloudBackupEnableContext;
use super::wallets::{StagedPrfKey, UnpersistedPrfKey};

mod coordinator;
pub(crate) mod evidence;

pub(crate) use coordinator::{PendingEnableCoordinator, PromotionStartError};
pub(crate) use evidence::PendingWork;

pub(crate) const PENDING_ENABLE_JOURNAL_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingEnablePasskeyMetadata {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) provider_hint: Option<PasskeyProviderHint>,
}

impl From<&StagedPrfKey> for PendingEnablePasskeyMetadata {
    fn from(passkey: &StagedPrfKey) -> Self {
        Self {
            credential_id: passkey.credential_id.clone(),
            prf_salt: passkey.prf_salt,
            provider_hint: passkey.provider_hint.clone(),
        }
    }
}

impl From<&UnpersistedPrfKey> for PendingEnablePasskeyMetadata {
    fn from(passkey: &UnpersistedPrfKey) -> Self {
        Self {
            credential_id: passkey.credential_id.clone(),
            prf_salt: passkey.prf_salt,
            provider_hint: passkey.provider_hint.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PendingEnableJournalPhase {
    Staging,
    Staged,
    PasskeyRegistered(PendingEnablePasskeyMetadata),
    RemoteWritesStarted(PendingEnablePasskeyMetadata),
    LocalPromotionStarted(PendingEnablePasskeyMetadata),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PendingEnableNamespaceOwnership {
    FreshOwned,
    RecoveredExisting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingEnableLocalMetadataSnapshot {
    pub(crate) credential_id: Option<String>,
    pub(crate) prf_salt: Option<String>,
    pub(crate) namespace_id: Option<String>,
    /// The stored provider hint, as stored; journals written before hints were stored have none
    #[serde(default)]
    pub(crate) provider_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingEnableJournal {
    version: u8,
    context: CloudBackupEnableContext,
    namespace_id: String,
    namespace_ownership: PendingEnableNamespaceOwnership,
    phase: PendingEnableJournalPhase,
    previous_metadata: PendingEnableLocalMetadataSnapshot,
}

impl PendingEnableJournal {
    pub(crate) fn staging(
        context: CloudBackupEnableContext,
        namespace_id: String,
        namespace_ownership: PendingEnableNamespaceOwnership,
        previous_metadata: PendingEnableLocalMetadataSnapshot,
    ) -> Self {
        Self {
            version: PENDING_ENABLE_JOURNAL_VERSION,
            context,
            namespace_id,
            namespace_ownership,
            phase: PendingEnableJournalPhase::Staging,
            previous_metadata,
        }
    }

    pub(crate) fn context(&self) -> CloudBackupEnableContext {
        self.context
    }

    pub(crate) fn version(&self) -> u8 {
        self.version
    }

    pub(crate) fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    pub(crate) fn namespace_ownership(&self) -> PendingEnableNamespaceOwnership {
        self.namespace_ownership
    }

    pub(crate) fn phase(&self) -> &PendingEnableJournalPhase {
        &self.phase
    }

    pub(crate) fn passkey(&self) -> Option<&PendingEnablePasskeyMetadata> {
        match &self.phase {
            PendingEnableJournalPhase::Staging | PendingEnableJournalPhase::Staged => None,
            PendingEnableJournalPhase::PasskeyRegistered(passkey)
            | PendingEnableJournalPhase::RemoteWritesStarted(passkey)
            | PendingEnableJournalPhase::LocalPromotionStarted(passkey) => Some(passkey),
        }
    }

    /// Whether this records a fresh Enable whose registered passkey waits for the user to confirm
    /// it was saved, before any local promotion began
    pub(crate) fn awaits_saved_passkey(&self) -> bool {
        self.namespace_ownership == PendingEnableNamespaceOwnership::FreshOwned
            && matches!(
                self.phase,
                PendingEnableJournalPhase::PasskeyRegistered(_)
                    | PendingEnableJournalPhase::RemoteWritesStarted(_)
            )
    }

    pub(crate) fn previous_metadata(&self) -> &PendingEnableLocalMetadataSnapshot {
        &self.previous_metadata
    }

    pub(crate) fn mark_staged(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::Staging => {
                self.phase = PendingEnableJournalPhase::Staged;
                true
            }
            PendingEnableJournalPhase::Staged => true,
            PendingEnableJournalPhase::PasskeyRegistered(_)
            | PendingEnableJournalPhase::RemoteWritesStarted(_)
            | PendingEnableJournalPhase::LocalPromotionStarted(_) => false,
        }
    }

    pub(crate) fn register_passkey(&mut self, passkey: PendingEnablePasskeyMetadata) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::Staged => {
                self.phase = PendingEnableJournalPhase::PasskeyRegistered(passkey);
                true
            }
            PendingEnableJournalPhase::PasskeyRegistered(current)
            | PendingEnableJournalPhase::RemoteWritesStarted(current)
            | PendingEnableJournalPhase::LocalPromotionStarted(current) => *current == passkey,
            PendingEnableJournalPhase::Staging => false,
        }
    }

    pub(crate) fn mark_remote_writes_started(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::PasskeyRegistered(passkey) => {
                self.phase = PendingEnableJournalPhase::RemoteWritesStarted(passkey.clone());
                true
            }
            PendingEnableJournalPhase::RemoteWritesStarted(_)
            | PendingEnableJournalPhase::LocalPromotionStarted(_) => true,
            PendingEnableJournalPhase::Staging | PendingEnableJournalPhase::Staged => false,
        }
    }

    pub(crate) fn mark_local_promotion_started(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::RemoteWritesStarted(passkey) => {
                self.phase = PendingEnableJournalPhase::LocalPromotionStarted(passkey.clone());
                true
            }
            PendingEnableJournalPhase::LocalPromotionStarted(_) => true,
            PendingEnableJournalPhase::Staging
            | PendingEnableJournalPhase::Staged
            | PendingEnableJournalPhase::PasskeyRegistered(_) => false,
        }
    }

    pub(crate) fn roll_back_local_promotion(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::LocalPromotionStarted(passkey) => {
                self.phase = PendingEnableJournalPhase::RemoteWritesStarted(passkey.clone());
                true
            }
            PendingEnableJournalPhase::RemoteWritesStarted(_) => true,
            PendingEnableJournalPhase::Staging
            | PendingEnableJournalPhase::Staged
            | PendingEnableJournalPhase::PasskeyRegistered(_) => false,
        }
    }
}

pub(crate) struct PendingSavedPasskeySessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<StagedPrfKey>,
    context: CloudBackupEnableContext,
}

/// Tracks passkey material created during enable before the flow fully completes
///
/// Only a registered passkey waiting for the user's confirmation is kept: an upload that fails
/// after it goes back to that pause, which the journal records
pub(crate) enum PendingEnableSession {
    /// A registered passkey is staged until targeted PRF auth confirms it can be used
    AwaitingSavedPasskeyConfirmation(PendingSavedPasskeySessionMaterial),
}

impl std::fmt::Debug for PendingEnableSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingEnableSession").finish_non_exhaustive()
    }
}

impl PendingSavedPasskeySessionMaterial {
    pub(crate) fn new(
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<StagedPrfKey>,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self { master_key, passkey, context }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<StagedPrfKey>) {
        (self.master_key, self.passkey)
    }

    pub(crate) fn namespace_id(&self) -> String {
        self.master_key.namespace_id()
    }

    pub(crate) fn context(&self) -> CloudBackupEnableContext {
        self.context
    }
}

impl PendingEnableSession {
    pub(crate) fn into_staged_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<StagedPrfKey>) {
        match self {
            Self::AwaitingSavedPasskeyConfirmation(material) => material.into_parts(),
        }
    }

    pub(crate) fn namespace_id(&self) -> String {
        match self {
            Self::AwaitingSavedPasskeyConfirmation(material) => material.namespace_id(),
        }
    }

    pub(crate) fn context(&self) -> CloudBackupEnableContext {
        match self {
            Self::AwaitingSavedPasskeyConfirmation(material) => material.context(),
        }
    }

    pub(crate) fn awaiting_saved_passkey_confirmation(
        master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
        passkey: Zeroizing<StagedPrfKey>,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self::AwaitingSavedPasskeyConfirmation(PendingSavedPasskeySessionMaterial::new(
            master_key, passkey, context,
        ))
    }
}
