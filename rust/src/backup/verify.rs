use cove_util::ResultExt as _;
use zeroize::Zeroizing;

use crate::wallet::metadata::WalletMetadata;
use crate::wallet_identity::{
    ExistingWalletIdentitySet, collect_existing_wallet_identities, identity_key_for_backup,
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

        let duplicate_key = identity_key_for_backup(&metadata, wallet_backup)?;
        let already_on_device = existing_identities.contains(&duplicate_key);
        let import_preview = WalletImportPreview::new(&metadata, wallet_backup, already_on_device);

        if import_preview.will_import {
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
            already_on_device,
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

impl WalletImportPreview {
    fn new(
        metadata: &WalletMetadata,
        wallet_backup: &WalletBackup,
        already_on_device: bool,
    ) -> Self {
        if already_on_device {
            return Self { will_import: false, warning: None };
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

    use crate::backup::model::{AppSettings, PAYLOAD_VERSION, WalletBackup, WalletSecret};
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

    fn payload_with_wallet(wallet: WalletBackup) -> BackupPayload {
        BackupPayload {
            version: PAYLOAD_VERSION,
            created_at: 1700000000,
            wallets: vec![wallet],
            settings: AppSettings {
                selected_network: None,
                selected_fiat_currency: None,
                color_scheme: None,
                selected_nodes: vec![],
            },
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
}
