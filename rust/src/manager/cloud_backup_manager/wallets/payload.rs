use cove_cspp::backup_data::{
    DescriptorPair, WalletEntry, WalletMode, WalletSecret as CloudWalletSecret,
};
use cove_device::keychain::Keychain;
use cove_util::ResultExt as _;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::warn;

use super::super::LocalWalletSecret;
use super::{CloudBackupError, LocalWalletMode, MAX_CLOUD_LABELS_SIZE, PreparedWalletBackup};
use crate::backup::model::DescriptorPair as LocalDescriptorPair;
use crate::label_manager::LabelManager;
use crate::wallet::{
    WalletAddressType,
    metadata::{WalletColor, WalletMetadata, WalletType},
};

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
            origin: metadata.origin.clone(),
            fingerprint: metadata.master_fingerprint.as_ref().map(|fp| fp.as_uppercase()),
            labels_hash,
            secret_hash: stable_sync_hash(secret)?,
            descriptor_hash: stable_sync_hash_option(descriptors)?,
            xpub_hash: stable_sync_hash_option(xpub)?,
        })
    }

    fn content_revision_hash(&self) -> Result<String, CloudBackupError> {
        let bytes = serde_json::to_vec(self)
            .map_err_prefix("serialize revision payload", CloudBackupError::Internal)?;

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

pub async fn build_wallet_entry(
    metadata: &WalletMetadata,
    mode: LocalWalletMode,
) -> Result<WalletEntry, CloudBackupError> {
    let keychain = Keychain::global();
    let id = &metadata.id;
    let name = &metadata.name;

    let secret = match metadata.wallet_type {
        WalletType::Hot => {
            let mnemonic = keychain.get_wallet_key(id).map_err(|error| {
                CloudBackupError::Internal(format!("failed to get mnemonic for '{name}': {error}"))
            })?;
            let Some(mnemonic) = mnemonic else {
                return Err(CloudBackupError::Internal(format!(
                    "hot wallet '{name}' has no mnemonic"
                )));
            };

            CloudWalletSecret::Mnemonic(mnemonic.to_string())
        }
        WalletType::Cold => build_cold_wallet_secret(keychain, metadata, id, name)?,
        WalletType::XpubOnly | WalletType::WatchOnly => CloudWalletSecret::WatchOnly,
    };

    let xpub = match keychain.get_wallet_xpub(id) {
        Ok(Some(xpub)) => Some(xpub.to_string()),
        Ok(None) => None,
        Err(error) => {
            return Err(CloudBackupError::Internal(format!(
                "failed to read xpub for '{name}': {error}"
            )));
        }
    };

    let descriptors = match keychain.get_public_descriptor(id) {
        Ok(Some((external, internal))) => {
            Some(DescriptorPair { external: external.to_string(), internal: internal.to_string() })
        }
        Ok(None) => None,
        Err(error) => {
            return Err(CloudBackupError::Internal(format!(
                "failed to read descriptors for '{name}': {error}"
            )));
        }
    };

    let metadata_value = serde_json::to_value(metadata)
        .map_err_prefix("serialize metadata", CloudBackupError::Internal)?;

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
    let updated_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

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

fn build_cold_wallet_secret(
    keychain: &Keychain,
    metadata: &WalletMetadata,
    id: &crate::wallet::metadata::WalletId,
    name: &str,
) -> Result<CloudWalletSecret, CloudBackupError> {
    let is_tap_signer =
        metadata.hardware_metadata.as_ref().is_some_and(|hardware| hardware.is_tap_signer());

    if !is_tap_signer {
        return Ok(CloudWalletSecret::WatchOnly);
    }

    match keychain.get_tap_signer_backup(id) {
        Ok(Some(backup)) => Ok(CloudWalletSecret::TapSignerBackup(backup)),
        Ok(None) => {
            warn!("Tap signer wallet '{name}' has no backup, exporting without it");
            Ok(CloudWalletSecret::WatchOnly)
        }
        Err(error) => Err(CloudBackupError::Internal(format!(
            "failed to read tap signer backup for '{name}': {error}"
        ))),
    }
}

pub async fn prepare_wallet_backup(
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
        Some(
            crate::backup::crypto::compress(labels_jsonl.as_bytes())
                .map_err_prefix("compress labels", CloudBackupError::Internal)?,
        )
    };

    Ok(PreparedCloudLabels {
        labels_zstd_jsonl,
        labels_count,
        labels_hash,
        labels_uncompressed_size,
    })
}

pub fn wallet_metadata_change_requires_upload(
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
    let bytes = serde_json::to_vec(value)
        .map_err_prefix("serialize sync field", CloudBackupError::Internal)?;
    Ok(hex::encode(Sha256::digest(bytes)))
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
        .map_err(|error| CloudBackupError::Internal(format!("open labels db: {error}")))?;

    manager.export().await.map_err_str(CloudBackupError::Internal)
}

pub fn decode_cloud_labels_jsonl(entry: &WalletEntry) -> Result<Option<String>, CloudBackupError> {
    let Some(compressed_labels) = &entry.labels_zstd_jsonl else {
        return Ok(None);
    };

    let decompressed = crate::backup::crypto::decompress(compressed_labels)
        .map_err_prefix("decompress labels", CloudBackupError::Internal)?;

    ensure_cloud_labels_size(decompressed.len(), "decompressed")?;

    String::from_utf8(decompressed.to_vec())
        .map(Some)
        .map_err(|error| CloudBackupError::Internal(format!("decode labels as utf8: {error}")))
}

fn ensure_cloud_labels_size(size: usize, label_state: &str) -> Result<(), CloudBackupError> {
    if size <= MAX_CLOUD_LABELS_SIZE {
        return Ok(());
    }

    Err(CloudBackupError::Internal(format!(
        "{label_state} labels exceed {MAX_CLOUD_LABELS_SIZE} byte limit",
    )))
}

pub fn convert_cloud_secret(secret: &CloudWalletSecret) -> LocalWalletSecret {
    match secret {
        CloudWalletSecret::Mnemonic(mnemonic) => LocalWalletSecret::Mnemonic(mnemonic.clone()),
        CloudWalletSecret::TapSignerBackup(backup) => {
            LocalWalletSecret::TapSignerBackup(backup.clone())
        }
        CloudWalletSecret::Descriptor(_) | CloudWalletSecret::WatchOnly => LocalWalletSecret::None,
    }
}

pub fn descriptor_pair_from_cloud(
    descriptors: &Option<DescriptorPair>,
) -> Option<LocalDescriptorPair> {
    descriptors.as_ref().map(|descriptors| LocalDescriptorPair {
        external: descriptors.external.clone(),
        internal: descriptors.internal.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
