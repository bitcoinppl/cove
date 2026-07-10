use cove_cspp::backup_data::PasskeyProviderHint;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::wallets::{StagedPrfKey, UnpersistedPrfKey};
use super::{CloudBackupEnableContext, CloudBackupError};

mod coordinator;

pub(crate) use coordinator::PendingEnableCoordinator;

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
    pub(crate) fn staged(
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
            phase: PendingEnableJournalPhase::Staged,
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
            PendingEnableJournalPhase::Staged => None,
            PendingEnableJournalPhase::PasskeyRegistered(passkey)
            | PendingEnableJournalPhase::RemoteWritesStarted(passkey)
            | PendingEnableJournalPhase::LocalPromotionStarted(passkey) => Some(passkey),
        }
    }

    pub(crate) fn previous_metadata(&self) -> &PendingEnableLocalMetadataSnapshot {
        &self.previous_metadata
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
            PendingEnableJournalPhase::Staged => false,
        }
    }

    pub(crate) fn mark_local_promotion_started(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::RemoteWritesStarted(passkey) => {
                self.phase = PendingEnableJournalPhase::LocalPromotionStarted(passkey.clone());
                true
            }
            PendingEnableJournalPhase::LocalPromotionStarted(_) => true,
            PendingEnableJournalPhase::Staged | PendingEnableJournalPhase::PasskeyRegistered(_) => {
                false
            }
        }
    }

    pub(crate) fn roll_back_local_promotion(&mut self) -> bool {
        match &self.phase {
            PendingEnableJournalPhase::LocalPromotionStarted(passkey) => {
                self.phase = PendingEnableJournalPhase::RemoteWritesStarted(passkey.clone());
                true
            }
            PendingEnableJournalPhase::RemoteWritesStarted(_) => true,
            PendingEnableJournalPhase::Staged | PendingEnableJournalPhase::PasskeyRegistered(_) => {
                false
            }
        }
    }
}

pub(crate) struct PendingEnableSessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<UnpersistedPrfKey>,
    context: CloudBackupEnableContext,
}

pub(crate) struct PendingSavedPasskeySessionMaterial {
    master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    passkey: Zeroizing<StagedPrfKey>,
    context: CloudBackupEnableContext,
}

/// Tracks passkey material created during enable before the flow fully completes
#[allow(dead_code)]
pub(crate) enum PendingEnableSession {
    /// A new passkey and master key are staged while the user confirms Create New Backup
    AwaitingForceNewConfirmation(PendingEnableSessionMaterial),
    /// Upload already started and should retry with the same staged passkey material
    RetryUpload(PendingEnableSessionMaterial),
    /// A registered passkey is staged until targeted PRF auth confirms it can be used
    AwaitingSavedPasskeyConfirmation(PendingSavedPasskeySessionMaterial),
}

impl std::fmt::Debug for PendingEnableSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingEnableSession").finish_non_exhaustive()
    }
}

impl PendingEnableSessionMaterial {
    pub(crate) fn new(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self { master_key: Zeroizing::new(master_key), passkey: Zeroizing::new(passkey), context }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>) {
        (self.master_key, self.passkey)
    }

    pub(crate) fn namespace_id(&self) -> String {
        self.master_key.namespace_id()
    }

    pub(crate) fn context(&self) -> CloudBackupEnableContext {
        self.context
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
    pub(crate) fn retry_upload(
        master_key: cove_cspp::master_key::MasterKey,
        passkey: UnpersistedPrfKey,
        context: CloudBackupEnableContext,
    ) -> Self {
        Self::RetryUpload(PendingEnableSessionMaterial::new(master_key, passkey, context))
    }

    pub(crate) fn into_ready_parts(
        self,
    ) -> Result<
        (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<UnpersistedPrfKey>),
        CloudBackupError,
    > {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                Ok(material.into_parts())
            }
            Self::AwaitingSavedPasskeyConfirmation(_) => Err(CloudBackupError::Internal(
                "pending enable session did not contain authenticated passkey material".into(),
            )),
        }
    }

    pub(crate) fn into_staged_parts(
        self,
    ) -> Result<
        (Zeroizing<cove_cspp::master_key::MasterKey>, Zeroizing<StagedPrfKey>),
        CloudBackupError,
    > {
        match self {
            Self::AwaitingSavedPasskeyConfirmation(material) => Ok(material.into_parts()),
            Self::AwaitingForceNewConfirmation(_) | Self::RetryUpload(_) => {
                Err(CloudBackupError::Internal(
                    "pending enable session did not contain staged passkey material".into(),
                ))
            }
        }
    }

    pub(crate) fn namespace_id(&self) -> String {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.namespace_id()
            }
            Self::AwaitingSavedPasskeyConfirmation(material) => material.namespace_id(),
        }
    }

    pub(crate) fn context(&self) -> CloudBackupEnableContext {
        match self {
            Self::AwaitingForceNewConfirmation(material) | Self::RetryUpload(material) => {
                material.context()
            }
            Self::AwaitingSavedPasskeyConfirmation(material) => material.context(),
        }
    }

    pub(crate) fn is_retry_upload(&self) -> bool {
        matches!(self, Self::RetryUpload(_))
    }

    pub(crate) fn is_awaiting_force_new_confirmation(&self) -> bool {
        matches!(self, Self::AwaitingForceNewConfirmation(_))
    }

    pub(crate) fn is_awaiting_saved_passkey_confirmation(&self) -> bool {
        matches!(self, Self::AwaitingSavedPasskeyConfirmation(_))
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
