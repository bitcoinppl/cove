use std::str::FromStr as _;

use cove_cspp::backup_data::{EncryptedWalletBackup, WalletEntry};
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::payload::{convert_cloud_secret, descriptor_pair_from_cloud};
use super::{DownloadedWalletBackup, RemoteWalletBackupSummary, decode_cloud_labels_jsonl};
use crate::backup::import::{
    ExistingWalletIdentitySet, LabelRestoreBehavior, LabelRestoreWarning, duplicate_key_for_backup,
    restore_wallet_labels,
};
use crate::backup::model::{WalletBackup, WalletSecret};
use crate::manager::cloud_backup_manager::{CloudBackupError, LocalWalletSecret};
use crate::wallet::metadata::WalletMetadata;

pub(crate) enum WalletBackupLookup<T> {
    Found(T),
    NotFound,
    UnsupportedVersion(u32),
}

#[derive(Clone)]
pub(crate) struct WalletBackupReader {
    cloud: Option<CloudStorageClient>,
    namespace: String,
    critical_key: Zeroizing<[u8; 32]>,
}

impl WalletBackupReader {
    pub(crate) fn new(
        cloud: CloudStorageClient,
        namespace: String,
        critical_key: Zeroizing<[u8; 32]>,
    ) -> Self {
        Self { cloud: Some(cloud), namespace, critical_key }
    }

    pub(crate) async fn download(
        &self,
        record_id: &str,
    ) -> Result<DownloadedWalletBackup, CloudBackupError> {
        match self.lookup(record_id).await? {
            WalletBackupLookup::Found(wallet) => Ok(wallet),
            WalletBackupLookup::NotFound => Err(CloudBackupError::Cloud(format!(
                "download {record_id}: not found in cloud backup"
            ))),
            WalletBackupLookup::UnsupportedVersion(version) => Err(CloudBackupError::Internal(
                format!("download {record_id}: unsupported wallet backup version {version}"),
            )),
        }
    }

    pub(crate) async fn summary(
        &self,
        record_id: &str,
    ) -> Result<WalletBackupLookup<RemoteWalletBackupSummary>, CloudBackupError> {
        match self.lookup_entry(record_id).await? {
            WalletBackupLookup::Found(entry) => {
                Ok(WalletBackupLookup::Found(RemoteWalletBackupSummary::from_entry(&entry)))
            }
            WalletBackupLookup::NotFound => Ok(WalletBackupLookup::NotFound),
            WalletBackupLookup::UnsupportedVersion(version) => {
                Ok(WalletBackupLookup::UnsupportedVersion(version))
            }
        }
    }

    pub(crate) async fn lookup(
        &self,
        record_id: &str,
    ) -> Result<WalletBackupLookup<DownloadedWalletBackup>, CloudBackupError> {
        match self.lookup_entry(record_id).await? {
            WalletBackupLookup::Found(entry) => {
                let metadata = serde_json::from_value(entry.metadata.clone())
                    .map_err_prefix("parse wallet metadata", CloudBackupError::Internal)?;
                Ok(WalletBackupLookup::Found(DownloadedWalletBackup { metadata, entry }))
            }
            WalletBackupLookup::NotFound => Ok(WalletBackupLookup::NotFound),
            WalletBackupLookup::UnsupportedVersion(version) => {
                Ok(WalletBackupLookup::UnsupportedVersion(version))
            }
        }
    }

    pub(crate) async fn lookup_entry(
        &self,
        record_id: &str,
    ) -> Result<WalletBackupLookup<WalletEntry>, CloudBackupError> {
        match self.download_encrypted(record_id).await? {
            WalletBackupLookup::Found(encrypted) => {
                let entry = self
                    .decrypt_entry(&encrypted)
                    .map_err_prefix("decrypt wallet", CloudBackupError::Crypto)?;
                encrypted
                    .remote_metadata
                    .normalized_wallet(&self.namespace, record_id, Some(entry.wallet_id.as_str()))
                    .map_err(|error| {
                        CloudBackupError::Internal(format!("normalize wallet payload: {error}"))
                    })?;

                Ok(WalletBackupLookup::Found(entry))
            }
            WalletBackupLookup::NotFound => Ok(WalletBackupLookup::NotFound),
            WalletBackupLookup::UnsupportedVersion(version) => {
                Ok(WalletBackupLookup::UnsupportedVersion(version))
            }
        }
    }

    pub(crate) async fn download_encrypted(
        &self,
        record_id: &str,
    ) -> Result<WalletBackupLookup<EncryptedWalletBackup>, CloudBackupError> {
        let wallet_json = match self.download_wallet_json(record_id).await {
            Ok(wallet_json) => wallet_json,
            Err(CloudStorageError::NotFound(_)) => return Ok(WalletBackupLookup::NotFound),
            Err(error) => {
                return Err(CloudBackupError::cloud_storage_context(
                    format!("download {record_id}"),
                    error,
                ));
            }
        };

        let encrypted: EncryptedWalletBackup = serde_json::from_slice(&wallet_json)
            .map_err_prefix("deserialize wallet", CloudBackupError::Internal)?;

        if encrypted.version != 1 {
            let version = encrypted.version;
            warn!(
                "Skipping wallet backup {record_id}: unsupported wallet backup version {version}"
            );
            return Ok(WalletBackupLookup::UnsupportedVersion(version));
        }

        Ok(WalletBackupLookup::Found(encrypted))
    }

    async fn download_wallet_json(&self, record_id: &str) -> Result<Vec<u8>, CloudStorageError> {
        let Some(cloud) = &self.cloud else {
            return Err(CloudStorageError::NotAvailable(
                "test cloud storage cannot download wallet backups".into(),
            ));
        };
        cloud.download_wallet_backup(self.namespace.clone(), record_id.to_string()).await
    }

    pub(crate) fn decrypt_entry(
        &self,
        encrypted: &EncryptedWalletBackup,
    ) -> Result<WalletEntry, cove_cspp::CsppError> {
        wallet_crypto::decrypt_wallet_backup(encrypted, &self.critical_key)
    }
}

pub(crate) struct WalletRestoreSession(ExistingWalletIdentitySet);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct WalletRestoreOutcome {
    pub(crate) labels_warning: Option<LabelRestoreWarning>,
}

impl WalletRestoreSession {
    pub(crate) fn new(existing_identities: ExistingWalletIdentitySet) -> Self {
        Self(existing_identities)
    }

    pub(crate) async fn restore_record(
        &mut self,
        reader: &WalletBackupReader,
        record_id: &str,
    ) -> Result<WalletRestoreOutcome, CloudBackupError> {
        let wallet = reader.download(record_id).await?;
        self.restore_downloaded(&wallet)
    }

    pub(crate) fn restore_downloaded(
        &mut self,
        wallet: &DownloadedWalletBackup,
    ) -> Result<WalletRestoreOutcome, CloudBackupError> {
        let duplicate_key = wallet.duplicate_key()?;

        if self.should_skip_duplicate_wallet(&wallet.metadata, &duplicate_key) {
            return Ok(WalletRestoreOutcome::default());
        }

        let outcome = wallet.restore()?;
        self.0.insert(duplicate_key);

        Ok(outcome)
    }

    fn should_skip_duplicate_wallet(
        &self,
        metadata: &WalletMetadata,
        duplicate_key: &crate::backup::import::RestoredWalletDuplicateKey,
    ) -> bool {
        if self.0.contains(duplicate_key) {
            info!("Skipping duplicate wallet {}", metadata.name);
            true
        } else {
            false
        }
    }
}

impl DownloadedWalletBackup {
    fn duplicate_key(
        &self,
    ) -> Result<crate::backup::import::RestoredWalletDuplicateKey, CloudBackupError> {
        let backup_model = WalletBackup {
            metadata: self.entry.metadata.clone(),
            secret: WalletSecret::None,
            descriptors: descriptor_pair_from_cloud(&self.entry.descriptors),
            xpub: self.entry.xpub.clone(),
            labels_jsonl: None,
        };

        duplicate_key_for_backup(&self.metadata, &backup_model)
            .map_err_prefix("cloud wallet duplicate identity", CloudBackupError::Internal)
    }

    fn restore(&self) -> Result<WalletRestoreOutcome, CloudBackupError> {
        let backup_model = WalletBackup {
            metadata: self.entry.metadata.clone(),
            secret: convert_cloud_secret(&self.entry.secret),
            descriptors: descriptor_pair_from_cloud(&self.entry.descriptors),
            xpub: self.entry.xpub.clone(),
            labels_jsonl: decode_cloud_labels_jsonl(&self.entry)?,
        };

        match &backup_model.secret {
            LocalWalletSecret::Mnemonic(words) => {
                let mnemonic = bip39::Mnemonic::from_str(words)
                    .map_err_prefix("invalid mnemonic", CloudBackupError::Internal)?;

                crate::backup::import::restore_cloud_mnemonic_wallet(&self.metadata, mnemonic)
                    .map_err(|(error, _)| {
                        CloudBackupError::Internal(format!("restore mnemonic wallet: {error}"))
                    })?;
            }
            _ => {
                crate::backup::import::restore_cloud_descriptor_wallet(
                    &self.metadata,
                    &backup_model,
                )
                .map_err(|(error, _)| {
                    CloudBackupError::Internal(format!("restore descriptor wallet: {error}"))
                })?;
            }
        }

        let labels_outcome = restore_wallet_labels(
            &self.metadata.id,
            &self.metadata.name,
            backup_model.labels_jsonl.as_deref(),
            LabelRestoreBehavior::PreserveCloudBackupClean,
        );
        if let Some(warning) = &labels_outcome.warning {
            warn!("Failed to restore labels for wallet {}: {}", self.metadata.name, warning.error);
        }

        Ok(WalletRestoreOutcome { labels_warning: labels_outcome.warning })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_cspp::backup_data::WalletSecret;
    use std::sync::Arc;

    fn public_descriptors(account: u32) -> cove_cspp::backup_data::DescriptorPair {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";

        cove_cspp::backup_data::DescriptorPair {
            external: format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/0/*)"),
            internal: format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/1/*)"),
        }
    }

    fn public_metadata(name: &str) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.name = name.to_string();
        metadata.wallet_type = crate::wallet::metadata::WalletType::Cold;
        metadata.master_fingerprint =
            Some(Arc::new(crate::wallet::fingerprint::Fingerprint::from(
                bdk_wallet::bitcoin::bip32::Fingerprint::from_str("817e7be0").unwrap(),
            )));
        metadata
    }

    fn test_wallet_entry(metadata: &WalletMetadata) -> WalletEntry {
        WalletEntry {
            wallet_id: metadata.id.to_string(),
            secret: WalletSecret::WatchOnly,
            metadata: serde_json::to_value(metadata).unwrap(),
            descriptors: None,
            xpub: None,
            wallet_mode: cove_cspp::backup_data::WalletMode::Main,
            labels_zstd_jsonl: None,
            labels_count: 0,
            labels_hash: None,
            labels_uncompressed_size: None,
            content_revision_hash: "test-revision".into(),
            updated_at: 42,
        }
    }

    fn test_public_wallet(metadata: &WalletMetadata, account: u32) -> DownloadedWalletBackup {
        let mut entry = test_wallet_entry(metadata);
        entry.descriptors = Some(public_descriptors(account));

        DownloadedWalletBackup { metadata: metadata.clone(), entry }
    }

    #[test]
    fn decrypt_entry_round_trips_encrypted_wallet_entry() {
        let metadata = WalletMetadata::preview_new();
        let entry = test_wallet_entry(&metadata);
        let critical_key = [7; 32];
        let encrypted = wallet_crypto::encrypt_wallet_entry(&entry, &critical_key).unwrap();
        let reader = WalletBackupReader {
            cloud: None,
            namespace: "test-namespace".into(),
            critical_key: Zeroizing::new(critical_key),
        };

        let decrypted = reader.decrypt_entry(&encrypted).unwrap();

        assert_eq!(decrypted.wallet_id, entry.wallet_id);
        assert_eq!(decrypted.content_revision_hash, entry.content_revision_hash);
    }

    #[test]
    fn restore_session_skips_duplicate_wallet() {
        let metadata = public_metadata("Existing account 0");
        let wallet = test_public_wallet(&metadata, 0);
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(wallet.duplicate_key().unwrap());
        let mut session = WalletRestoreSession::new(existing_identities);

        session.restore_downloaded(&wallet).unwrap();

        assert_eq!(session.0.len(), 1);
    }

    #[test]
    fn restore_session_does_not_skip_same_fingerprint_different_cloud_identity() {
        let existing_metadata = public_metadata("Existing account 0");
        let existing_wallet = test_public_wallet(&existing_metadata, 0);
        let incoming_metadata = public_metadata("Incoming account 1");
        let incoming_wallet = test_public_wallet(&incoming_metadata, 1);
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(existing_wallet.duplicate_key().unwrap());
        let session = WalletRestoreSession::new(existing_identities);
        let incoming_key = incoming_wallet.duplicate_key().unwrap();

        assert!(!session.should_skip_duplicate_wallet(&incoming_wallet.metadata, &incoming_key));
    }

    #[test]
    fn restore_session_tracks_restored_wallet_identity() {
        let metadata = WalletMetadata::preview_new();
        let wallet = DownloadedWalletBackup {
            metadata: metadata.clone(),
            entry: test_wallet_entry(&metadata),
        };
        let mut session = WalletRestoreSession::new(ExistingWalletIdentitySet::default());

        session.0.insert(wallet.duplicate_key().unwrap());

        assert_eq!(session.0.len(), 1);
    }

    #[test]
    fn legacy_wallet_summary_omits_missing_updated_at() {
        let metadata = WalletMetadata::preview_new();
        let mut entry = test_wallet_entry(&metadata);
        entry.updated_at = 0;

        let summary = RemoteWalletBackupSummary::from_entry(&entry);

        assert_eq!(summary.updated_at, None);
    }
}
