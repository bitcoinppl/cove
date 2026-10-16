use std::str::FromStr as _;

use cove_cspp::backup_data::{EncryptedWalletBackup, WalletBackupVersion, WalletEntry};
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use cove_device::keychain::{Keychain, WalletSecret as KeychainWalletSecret, WalletXprv};
use cove_util::ResultExt as _;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::payload::{convert_cloud_secret, descriptor_pair_from_cloud};
use super::{DownloadedWalletBackup, RemoteWalletBackupSummary, decode_cloud_labels_jsonl};
use crate::backup::import::{
    CloudRestoreError, LabelRestoreBehavior, LabelRestoreWarning, LocalWalletConflict,
    restore_wallet_labels,
};
use crate::backup::model::{WalletBackup, WalletSecret};
use crate::database::Database;
use crate::manager::cloud_backup_manager::error::CloudBackupInternalError;
use crate::manager::cloud_backup_manager::{CloudBackupError, LocalWalletSecret};
use crate::wallet::metadata::WalletId;
use crate::wallet::secret_upgrade::{
    ExistingWalletSecretUpgrade, SecretUpgradeError, SecretUpgradeSource,
};
use crate::wallet_identity::{
    BackupRecoveryMaterial, ExistingWalletIdentitySet, LocalWalletCoverage, WalletIdentityKey,
    fallback_identity_key_for_backup, identity_key_for_backup,
};

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
                format!("download {record_id}: unsupported wallet backup version {version}").into(),
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
                let metadata =
                    serde_json::from_value(entry.metadata.clone()).map_err(|source| {
                        CloudBackupError::internal_context("parse wallet metadata", source)
                    })?;
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
                    .map_err(|source| CloudBackupError::crypto_context("decrypt wallet", source))?;
                encrypted
                    .remote_metadata
                    .normalized_wallet(&self.namespace, record_id, Some(entry.wallet_id.as_str()))
                    .map_err(|error| {
                        CloudBackupError::Internal(
                            format!("normalize wallet payload: {error}").into(),
                        )
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
            .map_err(|source| CloudBackupError::internal_context("deserialize wallet", source))?;

        match encrypted.backup_version() {
            Ok(WalletBackupVersion::V1 | WalletBackupVersion::V2) => {}
            Err(unsupported) => {
                let version = unsupported.0;
                warn!("Skipping wallet backup with unsupported wallet backup version {version}");
                return Ok(WalletBackupLookup::UnsupportedVersion(version));
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WalletRestoreOutcome {
    Restored { labels_warning: Option<LabelRestoreWarning> },
    SkippedDuplicate,
}

/// What restoring one cloud-only record did to the local wallets
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudOnlyRestoreOutcome {
    /// The backup was restored as a new local wallet
    Restored { labels_warning: Option<LabelRestoreWarning> },
    /// A local wallet with the backup's identity now holds the backup's secret as well
    SecretAdded { wallet_id: WalletId },
    /// A local wallet already holds everything the backup can recover
    AlreadyOnDevice,
}

impl WalletRestoreSession {
    pub(crate) fn new(existing_identities: ExistingWalletIdentitySet) -> Self {
        Self(existing_identities)
    }

    pub(crate) fn restore_downloaded(
        &mut self,
        wallet: &DownloadedWalletBackup,
    ) -> Result<WalletRestoreOutcome, CloudBackupError> {
        let duplicate_key = match wallet.duplicate_key() {
            Ok(duplicate_key) => duplicate_key,
            Err(error) => {
                let fallback_key = wallet.fallback_duplicate_key();
                if self.should_skip_duplicate_wallet(&fallback_key) {
                    return Ok(WalletRestoreOutcome::SkippedDuplicate);
                }

                return Err(error);
            }
        };

        if self.should_skip_duplicate_wallet(&duplicate_key) {
            return Ok(WalletRestoreOutcome::SkippedDuplicate);
        }

        let labels_warning = wallet.restore()?;
        self.0.insert(duplicate_key);

        Ok(WalletRestoreOutcome::Restored { labels_warning })
    }

    fn should_skip_duplicate_wallet(&self, duplicate_key: &WalletIdentityKey) -> bool {
        if self.0.contains(duplicate_key) {
            info!("Skipping duplicate wallet during cloud restore");
            true
        } else {
            false
        }
    }
}

/// Keep a local-data conflict typed so the reader learns their data was kept unchanged
fn cloud_restore_error(context: &str, error: CloudRestoreError) -> CloudBackupError {
    match error {
        CloudRestoreError::LocalConflict(conflict) => {
            CloudBackupError::LocalWalletConflict(conflict)
        }
        CloudRestoreError::Backup(error) => {
            CloudBackupError::Internal(CloudBackupInternalError::context(context, error))
        }
    }
}

impl DownloadedWalletBackup {
    /// Restores a cloud-only record against the current local wallets, adding its secret to a
    /// matching local wallet that lacks it
    ///
    /// A local watch-only or cold wallet shares a secret backup's identity without holding its
    /// secret, so skipping the backup as a duplicate would leave the only copy of that secret
    /// in the cloud
    ///
    /// `local_wallets` must be read after the download, since a local wallet can be deleted or
    /// added while it waits
    pub(crate) fn restore_cloud_only(
        &self,
        local_wallets: &ExistingWalletIdentitySet,
    ) -> Result<CloudOnlyRestoreOutcome, CloudBackupError> {
        let (identity_key, identity_error) = self.identity_key();

        match local_wallets.coverage(&identity_key, self.recovery_material()) {
            LocalWalletCoverage::Covered => {
                info!("Cloud-only wallet is already on this device");
                return Ok(CloudOnlyRestoreOutcome::AlreadyOnDevice);
            }
            LocalWalletCoverage::SecretMissing => {
                if let Some(outcome) = self.add_secret_to_local_wallet()? {
                    return Ok(outcome);
                }
            }
            LocalWalletCoverage::Absent => {}
        }

        if let Some(error) = identity_error {
            return Err(error);
        }

        let labels_warning = self.restore()?;
        Ok(CloudOnlyRestoreOutcome::Restored { labels_warning })
    }

    /// How much of this backup the local wallets in `identities` already hold
    pub(crate) fn local_coverage(
        &self,
        identities: &ExistingWalletIdentitySet,
    ) -> LocalWalletCoverage {
        let (identity_key, _) = self.identity_key();

        identities.coverage(&identity_key, self.recovery_material())
    }

    /// The key a restore matches against local wallets, plus the error that forced the fallback
    /// key when the backup's own public identity could not be read
    fn identity_key(&self) -> (WalletIdentityKey, Option<CloudBackupError>) {
        match self.duplicate_key() {
            Ok(identity_key) => (identity_key, None),
            Err(error) => (self.fallback_duplicate_key(), Some(error)),
        }
    }

    fn recovery_material(&self) -> BackupRecoveryMaterial {
        match convert_cloud_secret(&self.entry.secret) {
            LocalWalletSecret::Mnemonic(_) | LocalWalletSecret::Xprv(_) => {
                BackupRecoveryMaterial::Secret
            }
            _ => BackupRecoveryMaterial::PublicOnly,
        }
    }

    /// The backup's signing secret, or `None` for a public-only backup
    fn signing_secret(&self) -> Result<Option<KeychainWalletSecret>, CloudBackupError> {
        let secret = match &convert_cloud_secret(&self.entry.secret) {
            LocalWalletSecret::Mnemonic(words) => {
                let mnemonic = bip39::Mnemonic::from_str(words).map_err(|source| {
                    CloudBackupError::internal_context("invalid mnemonic", source)
                })?;

                KeychainWalletSecret::Mnemonic(mnemonic)
            }
            LocalWalletSecret::Xprv(value) => {
                let xpriv = WalletXprv::parse(value.as_str()).map_err_prefix(
                    "invalid extended private key",
                    CloudBackupInternalError::from,
                )?;

                KeychainWalletSecret::Xpriv(xpriv)
            }
            _ => return Ok(None),
        };

        Ok(Some(secret))
    }

    /// Adds this backup's secret to the local wallet with its exact public identity
    ///
    /// Returns `None` when no single local wallet has that identity, such as another account
    /// under the same fingerprint, so the backup restores as a new wallet
    ///
    /// The upgrade writes only to the existing wallet and rolls itself back on failure, so it
    /// needs no restore journal for the backup's own wallet ID
    fn add_secret_to_local_wallet(
        &self,
    ) -> Result<Option<CloudOnlyRestoreOutcome>, CloudBackupError> {
        let Some(secret) = self.signing_secret()? else {
            return Ok(None);
        };

        let database = Database::global();
        let keychain = Keychain::global();
        let metadata = &self.metadata;
        let upgrade = match ExistingWalletSecretUpgrade::find(
            &database,
            keychain,
            &secret,
            metadata.network,
            metadata.wallet_mode,
        ) {
            Ok(Some(upgrade)) => upgrade,
            // several local wallets share this identity, such as one seed watched under two
            // address types, so the seed restores as its own hot wallet instead of being stuck
            Ok(None) | Err(SecretUpgradeError::IdentityCollision) => return Ok(None),
            Err(error) => {
                return Err(CloudBackupError::internal_context("find wallet for secret", error));
            }
        };

        let source = SecretUpgradeSource::CloudBackup { verified: metadata.verified };
        match upgrade.apply(&database, keychain, source) {
            Ok(upgraded) => {
                Ok(Some(CloudOnlyRestoreOutcome::SecretAdded { wallet_id: upgraded.id }))
            }
            // the local wallet gained a readable secret after coverage was read
            Err(SecretUpgradeError::AlreadyHoldsSecret(_)) => {
                Ok(Some(CloudOnlyRestoreOutcome::AlreadyOnDevice))
            }
            // the backup may hold the only readable copy of this secret, so the row stays
            // restorable instead of becoming a deletable copy
            Err(SecretUpgradeError::UnreadableSecret(_)) => {
                Err(CloudBackupError::LocalWalletConflict(LocalWalletConflict::Unreadable))
            }
            Err(error) => {
                Err(CloudBackupError::internal_context("add secret to local wallet", error))
            }
        }
    }

    fn duplicate_key(&self) -> Result<WalletIdentityKey, CloudBackupError> {
        let backup_model = WalletBackup {
            metadata: self.entry.metadata.clone(),
            secret: WalletSecret::None,
            descriptors: descriptor_pair_from_cloud(&self.entry.descriptors),
            xpub: self.entry.xpub.clone(),
            labels_jsonl: None,
        };

        identity_key_for_backup(&self.metadata, &backup_model).map_err(|source| {
            CloudBackupError::internal_context("cloud wallet duplicate identity", source)
        })
    }

    fn fallback_duplicate_key(&self) -> WalletIdentityKey {
        fallback_identity_key_for_backup(&self.metadata)
    }

    /// Restores the backup as a new local wallet, returning any label import warning
    fn restore(&self) -> Result<Option<LabelRestoreWarning>, CloudBackupError> {
        let backup_model = WalletBackup {
            metadata: self.entry.metadata.clone(),
            secret: convert_cloud_secret(&self.entry.secret),
            descriptors: descriptor_pair_from_cloud(&self.entry.descriptors),
            xpub: self.entry.xpub.clone(),
            labels_jsonl: decode_cloud_labels_jsonl(&self.entry)?,
        };

        match self.signing_secret()? {
            Some(KeychainWalletSecret::Mnemonic(mnemonic)) => {
                crate::backup::import::restore_cloud_mnemonic_wallet(&self.metadata, mnemonic)
                    .map_err(|error| cloud_restore_error("restore mnemonic wallet", error))?;
            }
            Some(KeychainWalletSecret::Xpriv(xpriv)) => {
                crate::backup::import::restore_cloud_xpriv_wallet(&self.metadata, xpriv).map_err(
                    |error| cloud_restore_error("restore extended-private-key wallet", error),
                )?;
            }
            None => {
                crate::backup::import::restore_cloud_descriptor_wallet(
                    &self.metadata,
                    &backup_model,
                )
                .map_err(|error| cloud_restore_error("restore descriptor wallet", error))?;
            }
        }

        let labels_outcome = restore_wallet_labels(
            &self.metadata.id,
            &self.metadata.name,
            backup_model.labels_jsonl.as_deref(),
            LabelRestoreBehavior::PreserveCloudBackupClean,
        );
        if let Some(warning) = &labels_outcome.warning {
            warn!("Failed to restore labels for restored wallet: {}", warning.error);
        }

        Ok(labels_outcome.warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_cspp::backup_data::WalletSecret;
    use cove_device::keychain::Keychain;
    use std::sync::Arc;

    use crate::wallet::metadata::WalletMetadata;
    use crate::wallet_identity::test_support::ExistingWalletIdentitySetTestExt as _;

    fn test_keychain() -> &'static Keychain {
        crate::test_support::init_test_keychain();
        Keychain::global()
    }

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

    fn test_invalid_public_wallet(metadata: &WalletMetadata) -> DownloadedWalletBackup {
        let mut entry = test_wallet_entry(metadata);
        entry.descriptors = Some(cove_cspp::backup_data::DescriptorPair {
            external: "not a descriptor".to_string(),
            internal: "also not a descriptor".to_string(),
        });

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

        let outcome = session.restore_downloaded(&wallet).unwrap();

        assert_eq!(outcome, WalletRestoreOutcome::SkippedDuplicate);
        assert_eq!(session.0.len(), 1);
    }

    #[test]
    fn restore_session_skips_duplicate_wallet_with_invalid_public_identity() {
        let metadata = public_metadata("Existing malformed public wallet");
        let wallet = test_invalid_public_wallet(&metadata);
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(wallet.fallback_duplicate_key());
        let mut session = WalletRestoreSession::new(existing_identities);

        let outcome = session.restore_downloaded(&wallet).unwrap();

        assert_eq!(outcome, WalletRestoreOutcome::SkippedDuplicate);
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

        assert!(!session.should_skip_duplicate_wallet(&incoming_key));
    }

    #[test]
    fn restore_downloaded_tracks_restored_wallet_identity() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::init_test_database();
        test_keychain();

        let metadata = WalletMetadata::preview_new();
        let mut entry = test_wallet_entry(&metadata);
        entry.xpub = Some(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM"
                .into(),
        );
        let wallet = DownloadedWalletBackup { metadata, entry };
        let mut session = WalletRestoreSession::new(ExistingWalletIdentitySet::default());
        let duplicate_key = wallet.duplicate_key().unwrap();

        assert!(!session.0.contains(&duplicate_key));

        let first_outcome = session.restore_downloaded(&wallet).unwrap();

        assert_eq!(first_outcome, WalletRestoreOutcome::Restored { labels_warning: None });
        assert!(session.0.contains(&duplicate_key));
        assert_eq!(session.0.len(), 1);

        let second_outcome = session.restore_downloaded(&wallet).unwrap();

        assert_eq!(second_outcome, WalletRestoreOutcome::SkippedDuplicate);
        assert!(session.0.contains(&duplicate_key));
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
