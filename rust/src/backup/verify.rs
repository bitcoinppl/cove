use std::collections::HashSet;

use zeroize::Zeroizing;

use cove_types::WalletId;

use crate::wallet::metadata::WalletMetadata;

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

    // mutable — grows as we process wallets, matching import's progressive dedup
    let mut existing_fingerprints = import::collect_existing_fingerprints()?;

    // track no-fingerprint wallet IDs within this backup to detect intra-backup duplicates
    let mut seen_no_fp_ids: HashSet<WalletId> = HashSet::new();

    let mut wallets = Vec::with_capacity(payload.wallets.len());
    for wallet_backup in &payload.wallets {
        let metadata: WalletMetadata = serde_json::from_value(wallet_backup.metadata.clone())
            .map_err(|e| BackupError::Deserialization(format!("wallet metadata: {e}")))?;

        let mut already_on_device = import::is_wallet_duplicate(&metadata, &existing_fingerprints)?;

        // intra-backup dedup for no-fingerprint wallets
        if !already_on_device
            && metadata.master_fingerprint.is_none()
            && !seen_no_fp_ids.insert(metadata.id.clone())
        {
            already_on_device = true;
        }

        // track this wallet's fingerprint for intra-backup dedup (mirrors import.rs)
        if let Some(fp) = metadata.master_fingerprint.as_deref().copied() {
            existing_fingerprints.push((fp, metadata.network, metadata.wallet_mode));
        }

        // validate wallet_type/secret compatibility
        let warning = match import::validate_wallet_type_secret(
            &metadata.wallet_type,
            &wallet_backup.secret,
            &metadata.name,
        ) {
            Ok(WalletTypeSecretValidation::Valid) => None,
            Ok(WalletTypeSecretValidation::Degraded) => {
                Some("Will be imported with reduced functionality".to_string())
            }
            Err(e) => Some(format!("Import will fail: {e}")),
        };

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
