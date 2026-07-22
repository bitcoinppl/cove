use zeroize::Zeroizing;

use crate::manager::cloud_backup_manager::actors::{
    CleanupSourceNamespace, CloudBackupUploadedWallet,
};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, PreparedWalletBackup, StagedPrfKey, UnpersistedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupError, CloudBackupPasskeyHint,
    PendingEnablePasskeyMetadata, PendingEnableSession, PendingVerificationUpload,
};

pub(crate) struct MergeNamespace {
    pub(crate) matched: NamespaceMatch,
    pub(crate) wallet_record_ids: Vec<String>,
}

pub(crate) enum EnablePasskeyAcquisition {
    Ready(StagedPrfKey),
    Cancelled,
}

pub(crate) enum EnablePasskeyRegistrationFlow {
    ForceNew,
    NoDiscovery,
}

pub(crate) struct CloudBackupRegisteredEnablePasskey {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<StagedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
}

pub(crate) enum CloudBackupEnablePasskeyRegistration {
    Registered(CloudBackupRegisteredEnablePasskey),
    Cancelled { context: CloudBackupEnableContext },
}

pub(crate) enum CloudBackupEnablePasskeyPreparation {
    Ready(CloudBackupReadyEnableUpload),
    Registered(CloudBackupRegisteredEnablePasskey),
    Cancelled { context: CloudBackupEnableContext },
}

pub(crate) enum CloudBackupEnablePreparation {
    CreateNew {
        context: CloudBackupEnableContext,
    },
    ExistingBackupFound {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    PasskeyChoice {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    Recover {
        context: CloudBackupEnableContext,
        matches: Vec<NamespaceMatch>,
    },
}

pub(crate) struct CloudBackupEnableRecoveryPreparation {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) merge_namespaces: Vec<MergeNamespace>,
    pub(crate) active_index: usize,
    pub(crate) active_namespace_id: String,
    pub(crate) active_master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) active_critical_key: Zeroizing<[u8; 32]>,
}

pub(crate) struct CloudBackupEnableRecoveryCompletion {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
    pub(crate) active_critical_key: Zeroizing<[u8; 32]>,
    pub(crate) uploaded_wallets: Vec<CloudBackupUploadedWallet>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl CloudBackupEnableRecoveryPreparation {
    pub(crate) fn recovered_passkey_metadata(&self) -> PendingEnablePasskeyMetadata {
        let matched = &self.merge_namespaces[self.active_index].matched;

        PendingEnablePasskeyMetadata {
            credential_id: matched.credential_id.clone(),
            prf_salt: matched.prf_salt,
            provider_hint: None,
        }
    }
}

impl std::fmt::Debug for CloudBackupEnableRecoveryCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupEnableRecoveryCompletion")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("prf_salt", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("uploaded_wallets_count", &self.uploaded_wallets.len())
            .field("pending_uploads_count", &self.pending_uploads.len())
            .field("cleanup_sources_count", &self.cleanup_sources.len())
            .finish()
    }
}

pub(crate) enum CloudBackupNoDiscoveryEnablePreparation {
    RegisterPasskey {
        context: CloudBackupEnableContext,
    },
    ExistingBackupFound {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
}

pub(crate) struct CloudBackupReadyEnableUpload {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
}

pub(crate) struct CloudBackupUploadedEnableBackup {
    pub(crate) master_key: Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup,
    pub(crate) master_key_wrapper_revision: String,
    pub(crate) uploaded_at: u64,
    pub(crate) uploaded_wallets: Vec<PreparedWalletBackup>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
}

pub(crate) enum CloudBackupSavedPasskeyConfirmation {
    Confirmed(CloudBackupReadyEnableUpload),
    Retry { pending: PendingEnableSession, error: CloudBackupError },
    Failed(CloudBackupError),
}

impl EnablePasskeyRegistrationFlow {
    pub(crate) fn log_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new",
            Self::NoDiscovery => "Enable (no discovery)",
        }
    }

    pub(crate) fn cancelled_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new cancelled before passkey setup finished",
            Self::NoDiscovery => "Enable (no discovery) cancelled before passkey setup finished",
        }
    }

    pub(crate) fn failed_context(&self) -> &'static str {
        match self {
            Self::ForceNew => "Enable force new failed before passkey setup finished",
            Self::NoDiscovery => "Enable (no discovery) failed before passkey setup finished",
        }
    }
}
