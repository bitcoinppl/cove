use zeroize::Zeroizing;

use super::wallets::{StagedPrfKey, UnpersistedPrfKey};
use super::{CloudBackupEnableContext, CloudBackupError};

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
