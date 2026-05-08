mod passkey;
mod payload;
mod restore;
mod upload;

use cove_cspp::backup_data::WalletEntry;
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
    pub(crate) fn copy_for_retry(&self) -> Self {
        Self {
            prf_key: self.prf_key,
            prf_salt: self.prf_salt,
            credential_id: self.credential_id.clone(),
            provider_hint: self.provider_hint.clone(),
        }
    }

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
    pub(crate) updated_at: u64,
}

pub(crate) struct PreparedWalletBackup {
    pub(crate) metadata: WalletMetadata,
    pub(crate) record_id: String,
    pub(crate) revision_hash: String,
    pub(crate) entry: WalletEntry,
}

pub(crate) use passkey::{
    NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer,
    PasskeyMaterialOutcome,
};
pub(crate) use payload::{
    decode_cloud_labels_jsonl, prepare_wallet_backup, wallet_metadata_change_requires_upload,
};
pub(crate) use restore::{
    WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use super::payload::convert_cloud_secret;
}
