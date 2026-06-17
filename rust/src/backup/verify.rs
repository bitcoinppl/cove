use cove_util::ResultExt as _;
use zeroize::Zeroizing;

use crate::wallet::metadata::WalletMetadata;
use crate::wallet_identity::{
    ExistingWalletIdentitySet, WalletIdentityKey, collect_existing_wallet_identities,
    fallback_identity_key_for_backup, identity_key_for_backup,
};

use super::crypto;
use super::error::BackupError;
use super::import::{self, WalletTypeSecretValidation};
use super::model::{
    BackupPayload, BackupVerifyReport, BackupWalletSummary, WalletBackup, WalletSecretType,
};

pub async fn verify_backup(
    data: Vec<u8>,
    password: String,
) -> Result<BackupVerifyReport, BackupError> {
    let password = Zeroizing::new(password);
    let password = crypto::clean_password(&password)?;
    let decrypted = crypto::decrypt(&data, &password)?;
    let decompressed = crypto::decompress(&decrypted)?;
    let payload = BackupPayload::decode(&decompressed)?;

    // mutable: grows as we process wallets, matching import's progressive dedup
    let existing_identities = collect_existing_wallet_identities()?;

    verify_payload(payload, existing_identities)
}

fn verify_payload(
    mut payload: BackupPayload,
    mut existing_identities: ExistingWalletIdentitySet,
) -> Result<BackupVerifyReport, BackupError> {
    let mut wallets = Vec::with_capacity(payload.wallets.len());
    for wallet_backup in &payload.wallets {
        let metadata: WalletMetadata = serde_json::from_value(wallet_backup.metadata.clone())
            .map_err_prefix("wallet metadata", BackupError::Deserialization)?;

        let identity_preview =
            WalletIdentityPreview::new(&metadata, wallet_backup, &existing_identities);

        let import_preview = WalletImportPreview::new(
            &metadata,
            wallet_backup,
            identity_preview.already_on_device,
            identity_preview.import_blocking_error.as_deref(),
        );

        if import_preview.will_import
            && let Some(duplicate_key) = identity_preview.duplicate_key
        {
            existing_identities.insert(duplicate_key);
        }

        let label_count = wallet_backup
            .labels_jsonl
            .as_ref()
            .map(|jsonl| jsonl.lines().filter(|l| !l.trim().is_empty()).count() as u32)
            .unwrap_or(0);

        wallets.push(BackupWalletSummary {
            name: metadata.name,
            network: metadata.network,
            wallet_type: metadata.wallet_type,
            fingerprint: metadata.master_fingerprint.as_ref().map(|fp| fp.as_uppercase()),
            secret_type: WalletSecretType::from(&wallet_backup.secret),
            has_xpub: wallet_backup.xpub.is_some(),
            has_descriptors: wallet_backup.descriptors.is_some(),
            label_count,
            already_on_device: identity_preview.already_on_device,
            warning: import_preview.warning,
        });
    }

    let report = BackupVerifyReport {
        created_at: payload.created_at,
        wallet_count: wallets.len() as u32,
        wallets,
        fiat_currency: payload.settings.selected_fiat_currency.clone(),
        color_scheme: payload.settings.color_scheme.clone(),
        node_config_count: payload.settings.selected_nodes.len() as u32,
    };

    payload.wallets.clear();
    Ok(report)
}

struct WalletImportPreview {
    will_import: bool,
    warning: Option<String>,
}

struct WalletIdentityPreview {
    duplicate_key: Option<WalletIdentityKey>,
    already_on_device: bool,
    import_blocking_error: Option<String>,
}

impl WalletIdentityPreview {
    fn new(
        metadata: &WalletMetadata,
        wallet_backup: &WalletBackup,
        existing_identities: &ExistingWalletIdentitySet,
    ) -> Self {
        match identity_key_for_backup(metadata, wallet_backup) {
            Ok(duplicate_key) => {
                let already_on_device = existing_identities.contains(&duplicate_key);

                Self {
                    duplicate_key: Some(duplicate_key),
                    already_on_device,
                    import_blocking_error: None,
                }
            }
            Err(error) => {
                let fallback_key = fallback_identity_key_for_backup(metadata);
                let already_on_device = existing_identities.contains(&fallback_key);

                Self {
                    duplicate_key: already_on_device.then_some(fallback_key),
                    already_on_device,
                    import_blocking_error: (!already_on_device).then(|| error.to_string()),
                }
            }
        }
    }
}

impl WalletImportPreview {
    fn new(
        metadata: &WalletMetadata,
        wallet_backup: &WalletBackup,
        already_on_device: bool,
        identity_error: Option<&str>,
    ) -> Self {
        if already_on_device {
            return Self { will_import: false, warning: None };
        }

        if let Some(error) = identity_error {
            return Self {
                will_import: false,
                warning: Some(format!("Import will fail: {error}")),
            };
        }

        match import::validate_wallet_type_secret(
            &metadata.wallet_type,
            &wallet_backup.secret,
            &metadata.name,
        ) {
            Ok(WalletTypeSecretValidation::Valid) => Self { will_import: true, warning: None },
            Ok(WalletTypeSecretValidation::Degraded) => Self {
                will_import: true,
                warning: Some("Will be imported with reduced functionality".to_string()),
            },
            Err(e) => Self { will_import: false, warning: Some(format!("Import will fail: {e}")) },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;
    use std::sync::Arc;

    use crate::backup::model::{
        AppSettings, DescriptorPair, PAYLOAD_VERSION, WalletBackup, WalletSecret,
    };
    use crate::wallet::fingerprint::Fingerprint;
    use crate::wallet::metadata::{WalletMetadata, WalletType};

    use super::*;

    fn hot_metadata(name: &str) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.name = name.to_string();
        metadata.wallet_type = WalletType::Hot;
        metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(
            bdk_wallet::bitcoin::bip32::Fingerprint::from_str("817e7be0").unwrap(),
        )));

        metadata
    }

    fn cold_metadata(name: &str) -> WalletMetadata {
        let mut metadata = hot_metadata(name);
        metadata.wallet_type = WalletType::Cold;
        metadata
    }

    fn payload_with_wallets(wallets: Vec<WalletBackup>) -> BackupPayload {
        BackupPayload {
            version: PAYLOAD_VERSION,
            created_at: 1700000000,
            wallets,
            settings: AppSettings {
                selected_network: None,
                selected_fiat_currency: None,
                color_scheme: None,
                selected_nodes: vec![],
                custom_block_explorers: Default::default(),
            },
        }
    }

    fn payload_with_wallet(wallet: WalletBackup) -> BackupPayload {
        payload_with_wallets(vec![wallet])
    }

    fn invalid_descriptor_wallet(metadata: &WalletMetadata) -> WalletBackup {
        WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret: WalletSecret::None,
            descriptors: Some(DescriptorPair {
                external: "not a descriptor".to_string(),
                internal: "also not a descriptor".to_string(),
            }),
            xpub: None,
            labels_jsonl: None,
        }
    }

    #[test]
    fn verify_duplicate_wallet_skips_secret_validation_warning() {
        let metadata = hot_metadata("Existing hot wallet");
        let wallet = WalletBackup {
            metadata: serde_json::to_value(&metadata).unwrap(),
            secret: WalletSecret::Unknown,
            descriptors: None,
            xpub: None,
            labels_jsonl: None,
        };
        let duplicate_key = identity_key_for_backup(&metadata, &wallet).unwrap();
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(duplicate_key);

        let report = verify_payload(payload_with_wallet(wallet), existing_identities).unwrap();

        let summary = report.wallets.first().expect("wallet summary should exist");
        assert!(summary.already_on_device);
        assert_eq!(summary.warning, None);
    }

    #[test]
    fn verify_duplicate_wallet_skips_invalid_public_identity_warning() {
        let metadata = cold_metadata("Existing malformed public wallet");
        let wallet = invalid_descriptor_wallet(&metadata);
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(fallback_identity_key_for_backup(&metadata));

        let report = verify_payload(payload_with_wallet(wallet), existing_identities).unwrap();

        let summary = report.wallets.first().expect("wallet summary should exist");
        assert!(summary.already_on_device);
        assert_eq!(summary.warning, None);
    }

    #[test]
    fn verify_identity_key_failure_warns_for_wallet_and_continues() {
        let invalid_metadata = cold_metadata("Broken public wallet");
        let invalid_wallet = invalid_descriptor_wallet(&invalid_metadata);
        let valid_metadata = hot_metadata("Importable hot wallet");
        let valid_wallet = WalletBackup {
            metadata: serde_json::to_value(&valid_metadata).unwrap(),
            secret: WalletSecret::Mnemonic(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                    .to_string(),
            ),
            descriptors: None,
            xpub: None,
            labels_jsonl: None,
        };

        let report = verify_payload(
            payload_with_wallets(vec![invalid_wallet, valid_wallet]),
            ExistingWalletIdentitySet::default(),
        )
        .unwrap();

        assert_eq!(report.wallets.len(), 2);
        assert_eq!(report.wallets[0].name, invalid_metadata.name);
        assert!(!report.wallets[0].already_on_device);
        let warning = report.wallets[0]
            .warning
            .as_deref()
            .expect("invalid public identity should become a wallet warning");
        assert!(
            warning
                .contains("Import will fail: public descriptor identity for Broken public wallet")
        );

        assert_eq!(report.wallets[1].name, valid_metadata.name);
        assert!(!report.wallets[1].already_on_device);
        assert_eq!(report.wallets[1].warning, None);
    }
}
