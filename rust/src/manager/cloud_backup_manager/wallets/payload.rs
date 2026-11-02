use std::str::FromStr as _;

use cove_cspp::backup_data::{
    DescriptorPair, WalletEntry, WalletMode, WalletSecret as CloudWalletSecret,
};
use cove_device::keychain::{Keychain, WalletSecret as KeychainWalletSecret, WalletXprv};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::warn;

use super::{MAX_CLOUD_LABELS_SIZE, PreparedWalletBackup};
use crate::backup::model::DescriptorPair as LocalDescriptorPair;
use crate::label_manager::LabelManager;
use crate::manager::cloud_backup_manager::{CloudBackupError, LocalWalletMode, LocalWalletSecret};
use crate::mnemonic::MnemonicExt as _;
use crate::wallet::{
    WalletAddressType,
    addressing::authoritative_public_descriptor_mirror,
    metadata::{WalletBirthday, WalletColor, WalletMetadata, WalletType},
};
use crate::wallet_secret::WalletSecretExt as _;

#[derive(Debug)]
struct PreparedCloudLabels {
    labels_zstd_jsonl: Option<Vec<u8>>,
    labels_count: u32,
    labels_hash: Option<String>,
    labels_uncompressed_size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WalletBackupRevisionPayload {
    name: String,
    color: WalletColor,
    address_type: WalletAddressType,
    wallet_type: WalletType,
    #[serde(skip_serializing_if = "Option::is_none")]
    birthday: Option<WalletBirthday>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels_hash: Option<String>,
    secret_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptor_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    xpub_hash: Option<String>,
}

impl WalletBackupRevisionPayload {
    fn for_metadata(metadata: &WalletMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            color: metadata.color,
            address_type: metadata.address_type,
            wallet_type: metadata.wallet_type,
            birthday: metadata.birthday,
            origin: metadata.origin.clone(),
            fingerprint: metadata.master_fingerprint.as_ref().map(|fp| fp.as_uppercase()),
            labels_hash: None,
            secret_hash: String::new(),
            descriptor_hash: None,
            xpub_hash: None,
        }
    }

    fn for_backup(
        metadata: &WalletMetadata,
        labels_hash: Option<String>,
        secret: &CloudWalletSecret,
        descriptors: &Option<DescriptorPair>,
        xpub: &Option<String>,
    ) -> Result<Self, CloudBackupError> {
        Ok(Self {
            name: metadata.name.clone(),
            color: metadata.color,
            address_type: metadata.address_type,
            wallet_type: metadata.wallet_type,
            birthday: metadata.birthday,
            origin: metadata.origin.clone(),
            fingerprint: metadata.master_fingerprint.as_ref().map(|fp| fp.as_uppercase()),
            labels_hash,
            secret_hash: stable_sync_hash(secret)?,
            descriptor_hash: stable_sync_hash_option(descriptors)?,
            xpub_hash: stable_sync_hash_option(xpub)?,
        })
    }

    fn content_revision_hash(&self) -> Result<String, CloudBackupError> {
        let bytes = serde_json::to_vec(self).map_err(|source| {
            CloudBackupError::internal_context("serialize revision payload", source)
        })?;

        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

impl From<LocalWalletMode> for WalletMode {
    fn from(mode: LocalWalletMode) -> Self {
        match mode {
            LocalWalletMode::Main => Self::Main,
            LocalWalletMode::Decoy => Self::Decoy,
        }
    }
}

async fn build_wallet_entry(
    metadata: &WalletMetadata,
    mode: LocalWalletMode,
) -> Result<WalletEntry, CloudBackupError> {
    let keychain = Keychain::global();
    let id = &metadata.id;

    let secret = match metadata.wallet_type {
        WalletType::Hot => {
            let secret = keychain.get_wallet_secret(id).map_err(|error| {
                CloudBackupError::Internal(
                    format!("failed to get wallet private key: {error}").into(),
                )
            })?;

            let Some(secret) = secret else {
                return Err(CloudBackupError::Internal("hot wallet has no private key".into()));
            };

            match secret {
                KeychainWalletSecret::Mnemonic(mnemonic) => {
                    CloudWalletSecret::Mnemonic(mnemonic.to_string())
                }

                // a wallet restored before #988 can hold the other network's form, and devices
                // holding either form must agree on the revision hash of the same key
                KeychainWalletSecret::Xpriv(xpriv) => CloudWalletSecret::Xprv(
                    xpriv.into_network_kind(metadata.network.into()).expose().to_string(),
                ),
            }
        }
        WalletType::Cold => build_cold_wallet_secret(keychain, metadata, id)?,
        WalletType::XpubOnly | WalletType::WatchOnly => CloudWalletSecret::WatchOnly,
    };

    let xpub = match keychain.get_wallet_xpub(id) {
        Ok(Some(xpub)) => Some(xpub.to_string()),
        Ok(None) => None,
        Err(error) => {
            return Err(CloudBackupError::Internal(format!("failed to read xpub: {error}").into()));
        }
    };

    let descriptors = backup_descriptors(metadata, &secret)?;

    let metadata_value = serde_json::to_value(metadata.clone_without_local_scan_state())
        .map_err(|source| CloudBackupError::internal_context("serialize metadata", source))?;

    let wallet_mode = mode.into();

    let labels_jsonl = export_wallet_labels_jsonl(id).await?;
    let prepared_labels = prepare_cloud_labels(&labels_jsonl)?;
    let revision_payload = WalletBackupRevisionPayload::for_backup(
        metadata,
        prepared_labels.labels_hash.clone(),
        &secret,
        &descriptors,
        &xpub,
    )?;

    let content_revision_hash = revision_payload.content_revision_hash()?;
    let updated_at = cove_util::time::unix_timestamp_secs_or_zero();

    Ok(WalletEntry {
        wallet_id: id.to_string(),
        secret,
        metadata: metadata_value,
        descriptors,
        xpub,
        wallet_mode,
        labels_zstd_jsonl: prepared_labels.labels_zstd_jsonl,
        labels_count: prepared_labels.labels_count,
        labels_hash: prepared_labels.labels_hash,
        labels_uncompressed_size: prepared_labels.labels_uncompressed_size,
        content_revision_hash,
        updated_at,
    })
}

fn backup_descriptors(
    metadata: &WalletMetadata,
    secret: &CloudWalletSecret,
) -> Result<Option<DescriptorPair>, CloudBackupError> {
    let authoritative = authoritative_public_descriptor_mirror(metadata).map_err(|error| {
        CloudBackupError::Internal(format!("failed to load wallet descriptors: {error}").into())
    })?;

    if let Some((external, internal)) = authoritative {
        return Ok(Some(DescriptorPair {
            external: external.to_string(),
            internal: internal.to_string(),
        }));
    }

    match secret {
        CloudWalletSecret::Mnemonic(mnemonic) => {
            let mnemonic = bip39::Mnemonic::from_str(mnemonic).map_err(|error| {
                CloudBackupError::Internal(format!("failed to parse mnemonic: {error}").into())
            })?;

            let descriptors =
                mnemonic.into_descriptors(None, metadata.network, metadata.address_type);

            Ok(Some(DescriptorPair {
                external: descriptors.external.extended_descriptor.to_string(),
                internal: descriptors.internal.extended_descriptor.to_string(),
            }))
        }

        CloudWalletSecret::Xprv(value) => {
            let xpriv = WalletXprv::parse(value.as_str()).map_err(|error| {
                CloudBackupError::Internal(format!("failed to parse xprv: {error}").into())
            })?;

            let descriptors = KeychainWalletSecret::Xpriv(xpriv)
                .into_descriptors(metadata.network, metadata.address_type);

            Ok(Some(DescriptorPair {
                external: descriptors.external.extended_descriptor.to_string(),
                internal: descriptors.internal.extended_descriptor.to_string(),
            }))
        }

        CloudWalletSecret::TapSignerBackup(_) => {
            Err(CloudBackupError::Internal("tap signer wallet has no public descriptors".into()))
        }
        CloudWalletSecret::Descriptor(_) | CloudWalletSecret::WatchOnly => Ok(None),
    }
}

fn build_cold_wallet_secret(
    keychain: &Keychain,
    metadata: &WalletMetadata,
    id: &crate::wallet::metadata::WalletId,
) -> Result<CloudWalletSecret, CloudBackupError> {
    let is_tap_signer = metadata
        .hardware_metadata
        .as_ref()
        .is_some_and(crate::wallet::metadata::HardwareWalletMetadata::is_tap_signer);

    if !is_tap_signer {
        return Ok(CloudWalletSecret::WatchOnly);
    }

    match keychain.get_tap_signer_backup(id) {
        Ok(Some(backup)) => Ok(CloudWalletSecret::TapSignerBackup(backup.to_vec())),
        Ok(None) => {
            warn!("Tap signer wallet has no backup, exporting without it");
            Ok(CloudWalletSecret::WatchOnly)
        }
        Err(error) => Err(CloudBackupError::Internal(
            format!("failed to read tap signer backup: {error}").into(),
        )),
    }
}

/// Returns the revision hash a downloaded backup has once it is restored and uploaded again
///
/// A restore stores a master key in its wallet's network form, and an upload sends that form. A
/// backup written before #988 can carry the other form, so its copy in another namespace gets a
/// different hash although it holds the same key. The recomputed hash is used only when the
/// entry's own contents reproduce its stored hash, so any other mismatch still blocks cleanup
pub(crate) fn restored_content_revision_hash(
    metadata: &WalletMetadata,
    entry: &WalletEntry,
) -> String {
    let stored = &entry.content_revision_hash;
    let CloudWalletSecret::Xprv(value) = &entry.secret else {
        return stored.clone();
    };

    let Ok(xprv) = WalletXprv::parse(value.as_str()) else {
        return stored.clone();
    };

    let network_form = xprv.into_network_kind(metadata.network.into());
    if network_form.expose() == value.as_str() {
        return stored.clone();
    }

    let revision_hash = |secret: &CloudWalletSecret| {
        WalletBackupRevisionPayload::for_backup(
            metadata,
            entry.labels_hash.clone(),
            secret,
            &entry.descriptors,
            &entry.xpub,
        )
        .and_then(|payload| payload.content_revision_hash())
    };

    if revision_hash(&entry.secret).ok().as_ref() != Some(stored) {
        return stored.clone();
    }

    let restored_secret = CloudWalletSecret::Xprv(network_form.expose().to_string());
    revision_hash(&restored_secret).unwrap_or_else(|_| stored.clone())
}

pub(crate) async fn prepare_wallet_backup(
    metadata: &WalletMetadata,
    mode: LocalWalletMode,
) -> Result<PreparedWalletBackup, CloudBackupError> {
    let entry = build_wallet_entry(metadata, mode).await?;
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let revision_hash = entry.content_revision_hash.clone();

    Ok(PreparedWalletBackup { metadata: metadata.clone(), record_id, revision_hash, entry })
}

fn prepare_cloud_labels(labels_jsonl: &str) -> Result<PreparedCloudLabels, CloudBackupError> {
    ensure_cloud_labels_size(labels_jsonl.len(), "uncompressed")?;

    let labels_count = labels_jsonl.lines().filter(|line| !line.trim().is_empty()).count() as u32;
    let labels_hash =
        (!labels_jsonl.is_empty()).then(|| hex::encode(Sha256::digest(labels_jsonl.as_bytes())));
    let labels_uncompressed_size =
        (!labels_jsonl.is_empty()).then(|| labels_jsonl.len().try_into().unwrap_or(u32::MAX));
    let labels_zstd_jsonl = if labels_jsonl.is_empty() {
        None
    } else {
        Some(crate::backup::crypto::compress(labels_jsonl.as_bytes()))
    };

    Ok(PreparedCloudLabels {
        labels_zstd_jsonl,
        labels_count,
        labels_hash,
        labels_uncompressed_size,
    })
}

pub(crate) fn wallet_metadata_change_requires_upload(
    before: &WalletMetadata,
    after: &WalletMetadata,
) -> bool {
    WalletBackupRevisionPayload::for_metadata(before)
        != WalletBackupRevisionPayload::for_metadata(after)
}

fn stable_sync_hash<T>(value: &T) -> Result<String, CloudBackupError>
where
    T: Serialize,
{
    // stream the JSON into the hasher, so a serialized secret never sits in an unwiped buffer
    let mut hasher = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hasher, value)
        .map_err(|source| CloudBackupError::internal_context("serialize sync field", source))?;

    Ok(hex::encode(hasher.0.finalize()))
}

/// Feeds written bytes straight into a SHA-256 hasher
struct HashWriter(Sha256);

impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn stable_sync_hash_option<T>(value: &Option<T>) -> Result<Option<String>, CloudBackupError>
where
    T: Serialize,
{
    value.as_ref().map(stable_sync_hash).transpose()
}

async fn export_wallet_labels_jsonl(
    wallet_id: &crate::wallet::metadata::WalletId,
) -> Result<String, CloudBackupError> {
    let manager = LabelManager::try_new(wallet_id.clone())
        .map_err(|error| CloudBackupError::Internal(format!("open labels db: {error}").into()))?;

    manager.export().await.map_err(CloudBackupError::internal)
}

pub(crate) fn decode_cloud_labels_jsonl(
    entry: &WalletEntry,
) -> Result<Option<String>, CloudBackupError> {
    let Some(compressed_labels) = &entry.labels_zstd_jsonl else {
        return Ok(None);
    };

    let decompressed = crate::backup::crypto::decompress(compressed_labels)
        .map_err(|source| CloudBackupError::internal_context("decompress labels", source))?;

    ensure_cloud_labels_size(decompressed.len(), "decompressed")?;

    String::from_utf8(decompressed.to_vec()).map(Some).map_err(|error| {
        CloudBackupError::Internal(format!("decode labels as utf8: {error}").into())
    })
}

fn ensure_cloud_labels_size(size: usize, label_state: &str) -> Result<(), CloudBackupError> {
    if size <= MAX_CLOUD_LABELS_SIZE {
        return Ok(());
    }

    Err(CloudBackupError::Internal(
        format!("{label_state} labels exceed {MAX_CLOUD_LABELS_SIZE} byte limit",).into(),
    ))
}

pub(crate) fn convert_cloud_secret(secret: &CloudWalletSecret) -> LocalWalletSecret {
    match secret {
        CloudWalletSecret::Mnemonic(mnemonic) => LocalWalletSecret::Mnemonic(mnemonic.clone()),
        CloudWalletSecret::Xprv(xprv) => LocalWalletSecret::Xprv(xprv.clone()),
        CloudWalletSecret::TapSignerBackup(backup) => {
            LocalWalletSecret::TapSignerBackup(backup.clone())
        }
        CloudWalletSecret::Descriptor(_) | CloudWalletSecret::WatchOnly => LocalWalletSecret::None,
    }
}

pub(crate) fn descriptor_pair_from_cloud(
    descriptors: &Option<DescriptorPair>,
) -> Option<LocalDescriptorPair> {
    descriptors.as_ref().map(|descriptors| LocalDescriptorPair {
        external: descriptors.external.clone(),
        internal: descriptors.internal.clone(),
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use cove_cspp::backup_data::WalletEntry;

    use super::WalletBackupRevisionPayload;
    use crate::wallet::metadata::WalletMetadata;

    /// Computes the revision hash of `entry` from its contents, as the build that uploaded it did
    pub(crate) fn content_revision_hash(metadata: &WalletMetadata, entry: &WalletEntry) -> String {
        WalletBackupRevisionPayload::for_backup(
            metadata,
            entry.labels_hash.clone(),
            &entry.secret,
            &entry.descriptors,
            &entry.xpub,
        )
        .and_then(|payload| payload.content_revision_hash())
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_cloud_backup_derives_descriptors_when_both_mirrors_are_missing() {
        crate::test_support::init_test_keychain();
        let metadata = WalletMetadata::preview_new();
        let secret = CloudWalletSecret::Mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                .to_string(),
        );

        let descriptors = backup_descriptors(&metadata, &secret)
            .expect("hot-wallet descriptors derive from the loaded mnemonic")
            .expect("hot-wallet backup contains descriptors");

        assert!(descriptors.external.starts_with("wpkh("));
        assert!(descriptors.internal.starts_with("wpkh("));
    }

    #[test]
    fn hot_cloud_backup_derives_descriptors_from_loaded_xprv() {
        crate::test_support::init_test_keychain();
        let metadata = WalletMetadata::preview_new();
        let secret = CloudWalletSecret::Xprv(
            "xprv9s21ZrQH143K4BwRCYKSEPwcAMYweWkfKLURabnnv2GLNhJN1LSCgDQyGWyNcat72najQKwyshCBXWfHHVbcdxPAZPqByMyWDbWp5SjCfEa"
                .to_string(),
        );

        let descriptors = backup_descriptors(&metadata, &secret)
            .expect("hot-wallet descriptors derive from the loaded xprv")
            .expect("hot-wallet backup contains descriptors");

        assert!(descriptors.external.starts_with("wpkh("));
        assert!(descriptors.internal.starts_with("wpkh("));
    }

    // regression for #988: a device that restored a master key in its wallet's form and one that
    // still holds the backup's form must not mark each other's backups stale
    #[tokio::test(flavor = "current_thread")]
    async fn either_stored_form_of_a_master_key_backs_up_identically() {
        use bdk_wallet::bitcoin::{NetworkKind, bip32::Xpriv};
        use cove_types::Network;

        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = WalletMetadata::preview_new();
        metadata.network = Network::Signet;
        metadata.wallet_type = WalletType::Hot;

        let tprv = Xpriv::new_master(NetworkKind::Test, &[91; 32]).unwrap();
        let mut xprv = tprv;
        xprv.network = NetworkKind::Main;

        let backup_holding = async |stored: Xpriv| {
            let secret = KeychainWalletSecret::Xpriv(WalletXprv::try_from(stored).unwrap());
            Keychain::global().delete_wallet_secret(&metadata.id);
            Keychain::global().save_wallet_secret(&metadata.id, secret).unwrap();

            prepare_wallet_backup(&metadata, LocalWalletMode::Main).await.unwrap()
        };

        let legacy = backup_holding(xprv).await;
        let current = backup_holding(tprv).await;
        Keychain::global().delete_wallet_items(&metadata.id);

        assert_eq!(legacy.revision_hash, current.revision_hash);

        let CloudWalletSecret::Xprv(uploaded) = &legacy.entry.secret else {
            panic!("a hot xprv wallet uploads its master key");
        };
        assert_eq!(uploaded.as_str(), tprv.to_string());
    }

    // revision hashes are stored in Cloud Backup, so streaming the serialization must hash exactly
    // the bytes that serializing into a buffer did
    #[test]
    fn streamed_sync_hash_matches_existing_revision_hashes() {
        let secret = CloudWalletSecret::Xprv(
            "xprv9s21ZrQH143K4BwRCYKSEPwcAMYweWkfKLURabnnv2GLNhJN1LSCgDQyGWyNcat72najQKwyshCBXWfHHVbcdxPAZPqByMyWDbWp5SjCfEa"
                .to_string(),
        );
        let descriptors = Some(DescriptorPair {
            external: "wpkh([73c5da0a/84h/0h/0h]xpub/0/*)".to_string(),
            internal: "wpkh([73c5da0a/84h/0h/0h]xpub/1/*)".to_string(),
        });
        let buffered = |bytes: Vec<u8>| hex::encode(Sha256::digest(bytes));

        assert_eq!(
            stable_sync_hash(&secret).unwrap(),
            buffered(serde_json::to_vec(&secret).unwrap())
        );
        assert_eq!(
            stable_sync_hash_option(&descriptors).unwrap(),
            Some(buffered(serde_json::to_vec(descriptors.as_ref().unwrap()).unwrap()))
        );
    }

    fn content_revision_hash_for_metadata(metadata: &WalletMetadata) -> String {
        WalletBackupRevisionPayload::for_metadata(metadata).content_revision_hash().unwrap()
    }

    #[test]
    fn prepare_cloud_labels_rejects_uncompressed_labels_over_limit() {
        let labels_jsonl = "a".repeat(MAX_CLOUD_LABELS_SIZE + 1);

        let error = prepare_cloud_labels(&labels_jsonl).unwrap_err();

        assert!(matches!(
            error,
            CloudBackupError::Internal(message)
                if message == format!("uncompressed labels exceed {MAX_CLOUD_LABELS_SIZE} byte limit")
        ));
    }

    #[test]
    fn content_revision_hash_changes_when_wallet_name_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.name = "Renamed wallet".into();

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_wallet_color_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.color = alternate_wallet_color(metadata.color);

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_labels_hash_changes() {
        let metadata = WalletMetadata::preview_new();
        let original_hash = WalletBackupRevisionPayload::for_backup(
            &metadata,
            None,
            &CloudWalletSecret::WatchOnly,
            &None,
            &None,
        )
        .unwrap()
        .content_revision_hash()
        .unwrap();

        let updated_hash = WalletBackupRevisionPayload::for_backup(
            &metadata,
            Some("labels-hash".into()),
            &CloudWalletSecret::WatchOnly,
            &None,
            &None,
        )
        .unwrap()
        .content_revision_hash()
        .unwrap();

        assert_ne!(updated_hash, original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_address_type_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.address_type = crate::wallet::WalletAddressType::Legacy;

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_wallet_type_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.wallet_type = WalletType::WatchOnly;

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_birthday_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.birthday = Some(WalletBirthday::BlockHeight(800_000));

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_origin_changes() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.origin = Some("wpkh([abcd1234/84h/0h/0h])".into());

        assert_ne!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn content_revision_hash_changes_when_secret_changes() {
        let metadata = WalletMetadata::preview_new();
        let original_hash = WalletBackupRevisionPayload::for_backup(
            &metadata,
            None,
            &CloudWalletSecret::WatchOnly,
            &None,
            &None,
        )
        .unwrap()
        .content_revision_hash()
        .unwrap();

        let updated_hash = WalletBackupRevisionPayload::for_backup(
            &metadata,
            None,
            &CloudWalletSecret::Mnemonic("abandon abandon abandon".into()),
            &None,
            &None,
        )
        .unwrap()
        .content_revision_hash()
        .unwrap();

        assert_ne!(updated_hash, original_hash);
    }

    #[test]
    fn content_revision_hash_ignores_non_sync_metadata_fields() {
        let mut metadata = WalletMetadata::preview_new();
        let original_hash = content_revision_hash_for_metadata(&metadata);

        metadata.selected_unit = crate::transaction::Unit::Sat;
        metadata.sensitive_visible = !metadata.sensitive_visible;
        metadata.details_expanded = !metadata.details_expanded;
        metadata.show_labels = !metadata.show_labels;
        metadata.discovery_state = crate::wallet::metadata::DiscoveryState::ChoseAdressType;
        metadata.verified = !metadata.verified;

        assert_eq!(content_revision_hash_for_metadata(&metadata), original_hash);
    }

    #[test]
    fn wallet_metadata_change_requires_upload_only_for_sync_fields() {
        let original = WalletMetadata::preview_new();

        let mut renamed = original.clone();
        renamed.name = "Renamed wallet".into();
        assert!(wallet_metadata_change_requires_upload(&original, &renamed));

        let mut recolored = original.clone();
        recolored.color = alternate_wallet_color(recolored.color);
        assert!(wallet_metadata_change_requires_upload(&original, &recolored));

        let mut address_type_changed = original.clone();
        address_type_changed.address_type = crate::wallet::WalletAddressType::Legacy;
        assert!(wallet_metadata_change_requires_upload(&original, &address_type_changed));

        let mut wallet_type_changed = original.clone();
        wallet_type_changed.wallet_type = WalletType::WatchOnly;
        assert!(wallet_metadata_change_requires_upload(&original, &wallet_type_changed));

        let mut birthday_changed = original.clone();
        birthday_changed.birthday = Some(WalletBirthday::Timestamp(1_700_000_000));
        assert!(wallet_metadata_change_requires_upload(&original, &birthday_changed));

        let mut origin_changed = original.clone();
        origin_changed.origin = Some("wpkh([abcd1234/84h/0h/0h])".into());
        assert!(wallet_metadata_change_requires_upload(&original, &origin_changed));

        let mut view_only = original.clone();
        view_only.selected_unit = crate::transaction::Unit::Sat;
        view_only.details_expanded = !view_only.details_expanded;
        assert!(!wallet_metadata_change_requires_upload(&original, &view_only));
    }

    fn alternate_wallet_color(color: WalletColor) -> WalletColor {
        match color {
            WalletColor::Blue => WalletColor::Green,
            _ => WalletColor::Blue,
        }
    }
}
