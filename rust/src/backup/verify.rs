use cove_util::ResultExt as _;
use zeroize::Zeroizing;

use crate::wallet::metadata::WalletMetadata;
use crate::wallet_identity::{collect_existing_wallet_identities, identity_key_for_backup};

use super::crypto;
use super::error::BackupError;
use super::import::{self, WalletTypeSecretValidation};
use super::model::{BackupPayload, BackupVerifyReport, BackupWalletSummary, WalletSecretType};

pub async fn verify_backup(
    data: Vec<u8>,
    password: String,
) -> Result<BackupVerifyReport, BackupError> {
    let password = Zeroizing::new(password);
    let password = crypto::clean_password(&password)?;
    let decrypted = crypto::decrypt(&data, &password)?;
    let decompressed = crypto::decompress(&decrypted)?;
    let mut payload = BackupPayload::decode(&decompressed)?;

    // mutable: grows as we process wallets, matching import's progressive dedup
    let mut existing_identities = collect_existing_wallet_identities()?;

    let mut wallets = Vec::with_capacity(payload.wallets.len());
    for wallet_backup in &payload.wallets {
        let metadata: WalletMetadata = serde_json::from_value(wallet_backup.metadata.clone())
            .map_err_prefix("wallet metadata", BackupError::Deserialization)?;

        let duplicate_key = identity_key_for_backup(&metadata, wallet_backup)?;
        let already_on_device = existing_identities.contains(&duplicate_key);

        let validation = import::validate_wallet_type_secret(
            &metadata.wallet_type,
            &wallet_backup.secret,
            &metadata.name,
        );
        let is_importable = validation.is_ok();
        let warning = match validation {
            Ok(WalletTypeSecretValidation::Valid) => None,
            Ok(WalletTypeSecretValidation::Degraded) => {
                Some("Will be imported with reduced functionality".to_string())
            }
            Err(e) => Some(format!("Import will fail: {e}")),
        };

        if !already_on_device && is_importable {
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
            warning,
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
