mod passkey;
mod payload;
mod restore;
mod upload;

use cove_cspp::backup_data::{WalletEntry, WalletSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::wallet::metadata::WalletMetadata;

const UPLOAD_WALLET_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before wallets can be uploaded";
const MAX_CLOUD_LABELS_SIZE: usize = 10 * 1024 * 1024;
#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct UnpersistedPrfKey {
    pub(crate) prf_key: [u8; 32],
    pub(crate) prf_salt: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
    #[zeroize(skip)]
    pub(crate) provider_hint: Option<cove_cspp::backup_data::PasskeyProviderHint>,
}

impl UnpersistedPrfKey {
    pub(crate) fn into_parts(mut self) -> ([u8; 32], [u8; 32], Vec<u8>) {
        let credential_id = std::mem::take(&mut self.credential_id);

        (self.prf_key, self.prf_salt, credential_id)
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct StagedPrfKey {
    pub(crate) prf_salt: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
    #[zeroize(skip)]
    pub(crate) provider_hint: Option<cove_cspp::backup_data::PasskeyProviderHint>,
}

impl StagedPrfKey {
    pub(crate) fn copy_for_retry(&self) -> Self {
        Self {
            prf_salt: self.prf_salt,
            credential_id: self.credential_id.clone(),
            provider_hint: self.provider_hint.clone(),
        }
    }
}

pub(crate) struct DownloadedWalletBackup {
    pub(crate) metadata: WalletMetadata,
    pub(crate) entry: WalletEntry,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteWalletBackupSummary {
    pub(crate) revision_hash: String,
    pub(crate) label_count: u32,
    pub(crate) updated_at: Option<u64>,
    pub(crate) secret: BackupSecret,
}

impl RemoteWalletBackupSummary {
    pub(crate) fn from_entry(entry: &WalletEntry) -> Self {
        Self {
            revision_hash: entry.content_revision_hash.clone(),
            label_count: entry.labels_count,
            updated_at: (entry.updated_at != 0).then_some(entry.updated_at),
            secret: BackupSecret::from(&entry.secret),
        }
    }
}

/// Whether a wallet backup carries key material that can sign, or only public keys
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupSecret {
    Held,
    WatchOnly,
}

impl BackupSecret {
    /// Whether a record carrying `self` may replace a cloud record carrying `replaced`
    ///
    /// The cloud record may be the last copy of a key or TAPSIGNER backup, for example after
    /// a hot wallet lost its key here and opened as watch-only, so a record without key
    /// material never replaces one with it. The write lane enforces this before every wallet
    /// upload, and the inventory reports a wallet in that state the same way
    pub(crate) fn may_replace(self, replaced: Self) -> bool {
        match (self, replaced) {
            (Self::WatchOnly, Self::Held) => false,
            (Self::Held, _) | (Self::WatchOnly, Self::WatchOnly) => true,
        }
    }
}

impl From<&WalletSecret> for BackupSecret {
    fn from(secret: &WalletSecret) -> Self {
        match secret {
            WalletSecret::WatchOnly => Self::WatchOnly,
            WalletSecret::Mnemonic(_)
            | WalletSecret::Xprv(_)
            | WalletSecret::TapSignerBackup(_)
            | WalletSecret::Descriptor(_) => Self::Held,
        }
    }
}

pub(crate) struct PreparedWalletBackup {
    pub(crate) metadata: WalletMetadata,
    pub(crate) record_id: String,
    pub(crate) revision_hash: String,
    pub(crate) entry: WalletEntry,
}

#[cfg(test)]
pub(crate) use passkey::authority_test_support;
#[cfg(test)]
pub(crate) use passkey::passkey_request_may_present;
pub(crate) use passkey::{
    NamespaceMatch, NamespaceMatchOutcome, NamespaceMatchSnapshotOutcome, NamespacePasskeyMatcher,
    PasskeyMaterialAcquirer, PasskeyMaterialOutcome, PasskeyRequestAuthority, PermitGrant,
    PermitIntent, PlatformAuthorizationRetrier, delay_before_new_passkey_auth,
    map_wrapper_repair_passkey_error,
};
#[cfg(test)]
pub(crate) use payload::test_support as payload_test_support;
pub(crate) use payload::{
    LocalSecret, decode_cloud_labels_jsonl, prepare_wallet_backup, restored_content_revision_hash,
    wallet_metadata_change_requires_upload,
};
pub(crate) use restore::{
    CloudOnlyRestoreOutcome, WalletBackupLookup, WalletBackupReadError, WalletBackupReader,
    WalletRestoreOutcome, WalletRestoreSession,
};
pub(crate) use upload::WalletUpload;
