use std::str::FromStr as _;

use bip39::Mnemonic;
use strum::IntoEnumIterator as _;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use cove_device::keychain::Keychain;
use cove_types::network::Network;

use crate::database::Database;
use crate::database::global_config::GlobalConfigKey;
use crate::label_manager::LabelManager;
use crate::mnemonic::MnemonicExt as _;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletMetadata, WalletMode, WalletType};

use super::crypto;
use super::error::BackupError;
use super::model::{BackupImportReport, BackupPayload, WalletBackup, WalletSecret};

pub async fn import_all(
    data: Vec<u8>,
    password: String,
) -> Result<BackupImportReport, BackupError> {
    let password = Zeroizing::new(password);
    let password = crypto::clean_password(&password)?;

    let decrypted = crypto::decrypt(&data, &password)?;

    let decompressed = crypto::decompress(&decrypted)?;

    let mut payload = BackupPayload::decode(&decompressed)?;

    let mut report = BackupImportReport::default();

    // collect existing (fingerprint, network, mode) tuples to detect duplicates (including within this import)
    let mut existing_fingerprints = collect_existing_fingerprints()?;

    for wallet_backup in &payload.wallets {
        match restore_wallet(wallet_backup, &existing_fingerprints).await {
            Ok(RestoreResult::Imported {
                name,
                labels_imported,
                labels_failure,
                fingerprint,
                degraded,
            }) => {
                report.imported_wallet_names.push(name);
                if labels_imported {
                    report.wallets_with_labels_imported += 1;
                }
                if let Some((name, error)) = labels_failure {
                    report.labels_failed_wallet_names.push(name);
                    report.labels_failed_errors.push(error);
                }
                if let Some(entry) = fingerprint {
                    existing_fingerprints.push(entry);
                }
                if degraded {
                    let name = wallet_name_from_backup(wallet_backup);
                    report.degraded_wallet_names.push(name);
                }
            }
            Ok(RestoreResult::Skipped { name }) => {
                report.skipped_wallet_names.push(name);
            }
            Err(RestoreError { error: e, cleanup_warnings }) => {
                let name = wallet_name_from_backup(wallet_backup);
                error!("Failed to restore wallet {name}: {e}");
                for warning in &cleanup_warnings {
                    error!("Cleanup failure for {name}: {warning}");
                }
                report.failed_wallet_names.push(name);
                report.failed_wallet_errors.push(e.to_string());
                report.cleanup_warnings.extend(cleanup_warnings);
            }
        }
    }

    if report.imported_wallet_names.is_empty()
        && report.skipped_wallet_names.is_empty()
        && !report.failed_wallet_names.is_empty()
    {
        return Err(BackupError::Restore("All wallets failed to import".to_string()));
    }

    // restore settings only if at least one wallet imported, or backup was settings-only
    if !report.imported_wallet_names.is_empty()
        || payload.wallets.is_empty()
        || !report.skipped_wallet_names.is_empty()
    {
        match restore_settings(&payload.settings) {
            Ok(()) => report.settings_restored = true,
            Err(e) => {
                warn!("Failed to restore some settings: {e}");
                report.settings_error = Some(e.to_string());
            }
        }
    }

    // trigger zeroization of wallet secrets via WalletBackup::Drop
    payload.wallets.clear();

    Ok(report.finalize())
}

struct RestoreError {
    error: BackupError,
    cleanup_warnings: Vec<String>,
}

impl From<BackupError> for RestoreError {
    fn from(error: BackupError) -> Self {
        Self { error, cleanup_warnings: Vec::new() }
    }
}

enum RestoreResult {
    Imported {
        name: String,
        labels_imported: bool,
        labels_failure: Option<(String, String)>,
        fingerprint: Option<(Fingerprint, Network, WalletMode)>,
        degraded: bool,
    },
    Skipped {
        name: String,
    },
}

/// Check if a wallet already exists on the device
///
/// Matches import's exact behavior: fingerprint+network+mode first,
/// falls back to wallet ID for wallets without fingerprints
pub(crate) fn is_wallet_duplicate(
    metadata: &WalletMetadata,
    existing_fingerprints: &[(Fingerprint, Network, WalletMode)],
) -> Result<bool, BackupError> {
    if let Some(fp) = &metadata.master_fingerprint
        && existing_fingerprints.contains(&(**fp, metadata.network, metadata.wallet_mode))
    {
        return Ok(true);
    }

    if metadata.master_fingerprint.is_none() {
        let db = Database::global();
        let already_exists = db
            .wallets
            .get_all(metadata.network, metadata.wallet_mode)
            .map_err(|e| {
                BackupError::Database(format!("duplicate check for {}: {e}", metadata.name))
            })?
            .iter()
            .any(|w| w.id == metadata.id);
        return Ok(already_exists);
    }

    Ok(false)
}

pub(crate) fn collect_existing_fingerprints()
-> Result<Vec<(Fingerprint, Network, WalletMode)>, BackupError> {
    let db = Database::global();
    let mut fingerprints = Vec::new();

    for network in Network::iter() {
        for mode in [WalletMode::Main, WalletMode::Decoy] {
            let wallets = db
                .wallets
                .get_all(network, mode)
                .map_err(|e| BackupError::Database(format!("failed to read wallets: {e}")))?;

            for wallet in wallets {
                if let Some(fp) = &wallet.master_fingerprint {
                    fingerprints.push((**fp, network, mode));
                }
            }
        }
    }

    Ok(fingerprints)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WalletTypeSecretValidation {
    Valid,
    Degraded,
}

/// Validate that wallet_type and secret are compatible
///
/// Returns Ok(Valid) for correct combos, Ok(Degraded) for importable-but-degraded,
/// or Err for hard failures that would prevent import
pub(crate) fn validate_wallet_type_secret(
    wallet_type: &WalletType,
    secret: &WalletSecret,
    name: &str,
) -> Result<WalletTypeSecretValidation, BackupError> {
    match (wallet_type, secret) {
        (WalletType::Hot, WalletSecret::Mnemonic(_))
        | (WalletType::Cold, WalletSecret::TapSignerBackup(_))
        | (WalletType::XpubOnly | WalletType::WatchOnly, WalletSecret::None) => {
            Ok(WalletTypeSecretValidation::Valid)
        }

        // cold wallet without tap signer backup — xpub-only is normal for hardware wallets
        (WalletType::Cold, WalletSecret::None) => Ok(WalletTypeSecretValidation::Valid),

        // hot wallet with unknown secret — newer backup format, hard error
        (WalletType::Hot, WalletSecret::Unknown) => Err(BackupError::Restore(format!(
            "wallet {name} is a hot wallet with an unrecognized secret type, update the app to import this wallet"
        ))),

        // non-hot with unknown secret — degraded
        (_, WalletSecret::Unknown) => Ok(WalletTypeSecretValidation::Degraded),

        // genuine type/secret mismatch
        (wt, s) => Err(BackupError::Restore(format!(
            "wallet {name} has mismatched type ({wt:?}) and secret ({s:?})"
        ))),
    }
}

async fn restore_wallet(
    backup: &WalletBackup,
    existing_fingerprints: &[(Fingerprint, Network, WalletMode)],
) -> Result<RestoreResult, RestoreError> {
    let metadata: WalletMetadata = serde_json::from_value(backup.metadata.clone())
        .map_err(|e| BackupError::Deserialization(format!("wallet metadata: {e}")))?;

    let name = metadata.name.clone();
    let wallet_id = metadata.id.clone();
    let network = metadata.network;

    let validation = validate_wallet_type_secret(&metadata.wallet_type, &backup.secret, &name)?;
    let cold_missing_backup = validation == WalletTypeSecretValidation::Degraded;

    if is_wallet_duplicate(&metadata, existing_fingerprints)? {
        info!("Skipping wallet {name} — already exists on device");
        return Ok(RestoreResult::Skipped { name });
    }

    let mut labels_imported = false;
    let mut labels_failure: Option<(String, String)> = None;
    let mut degraded = cold_missing_backup;
    let fingerprint = metadata
        .master_fingerprint
        .as_deref()
        .copied()
        .map(|fp| (fp, network, metadata.wallet_mode));

    match &backup.secret {
        // NOTE: Mnemonic doesn't implement Zeroize (upstream), so the parsed
        // mnemonic lives as a plain heap allocation until save_wallet_key encrypts
        // and consumes it. WalletSecret::Mnemonic(String) is zeroized on drop
        WalletSecret::Mnemonic(words) => {
            let mnemonic = Mnemonic::from_str(words)
                .map_err(|e| BackupError::Restore(format!("invalid mnemonic for {name}: {e}")))?;

            restore_mnemonic_wallet(&metadata, mnemonic)
                .map_err(|(e, warnings)| RestoreError { error: e, cleanup_warnings: warnings })?;
        }

        secret => {
            if matches!(secret, WalletSecret::Unknown) {
                warn!("wallet {name} has unknown secret type, importing as descriptor-only");
                degraded = true;
            }

            restore_descriptor_wallet(&metadata, backup)
                .map_err(|(e, warnings)| RestoreError { error: e, cleanup_warnings: warnings })?;
        }
    }

    if let Some(jsonl) = &backup.labels_jsonl
        && !jsonl.is_empty()
    {
        match import_labels(&wallet_id, jsonl) {
            Ok(()) => labels_imported = true,
            Err(e) => {
                warn!("Failed to import labels for wallet {name}: {e}");
                labels_failure = Some((name.clone(), e.to_string()));
            }
        }
    }

    Ok(RestoreResult::Imported { name, labels_imported, labels_failure, fingerprint, degraded })
}

/// Run a restore operation, cleaning up on failure
///
/// Returns cleanup failure details on error so callers can surface them
fn with_cleanup<F>(metadata: &WalletMetadata, f: F) -> Result<(), (BackupError, Vec<String>)>
where
    F: FnOnce() -> Result<(), BackupError>,
{
    f().map_err(|e| {
        let cleanup_failures = cleanup_failed_wallet(metadata);
        (e, cleanup_failures)
    })
}

pub(crate) fn restore_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), (BackupError, Vec<String>)> {
    with_cleanup(metadata, || restore_mnemonic_wallet_inner(metadata, mnemonic))
}

fn restore_mnemonic_wallet_inner(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;
    let network = metadata.network;

    let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, network)
        .map_err(|e| BackupError::Restore(format!("BDK store for {name}: {e}")))?;

    // extract xpub before consuming mnemonic
    let xpub = mnemonic.xpub(network.into());
    let descriptors = mnemonic.clone().into_descriptors(None, network, metadata.address_type);

    let ext_descriptor = descriptors.external.extended_descriptor.clone();
    let int_descriptor = descriptors.internal.extended_descriptor.clone();

    // create BDK wallet first — if this fails we haven't touched the keychain yet
    bdk_wallet::Wallet::create(
        descriptors.external.into_tuple(),
        descriptors.internal.into_tuple(),
    )
    .network(network.into())
    .create_wallet(&mut store.conn)
    .map_err(|e| BackupError::Restore(format!("BDK wallet for {name}: {e}")))?;

    keychain
        .save_wallet_key(&metadata.id, mnemonic)
        .map_err(|e| BackupError::Keychain(format!("mnemonic for {name}: {e}")))?;

    keychain
        .save_wallet_xpub(&metadata.id, xpub)
        .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))?;

    keychain
        .save_public_descriptor(&metadata.id, ext_descriptor, int_descriptor)
        .map_err(|e| BackupError::Keychain(format!("descriptors for {name}: {e}")))?;

    db.wallets
        .save_new_wallet_metadata(metadata.clone())
        .map_err(|e| BackupError::Database(format!("metadata for {name}: {e}")))?;

    Ok(())
}

pub(crate) fn restore_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), (BackupError, Vec<String>)> {
    with_cleanup(metadata, || restore_descriptor_wallet_inner(metadata, backup))
}

fn restore_descriptor_wallet_inner(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;

    // reject wallets with no xpub and no descriptors — they'd create broken entries
    if backup.xpub.is_none() && backup.descriptors.is_none() {
        return Err(BackupError::Restore(format!(
            "wallet {name} has no xpub or descriptors, cannot restore"
        )));
    }

    if let Some(xpub_str) = &backup.xpub {
        let xpub = bdk_wallet::bitcoin::bip32::Xpub::from_str(xpub_str)
            .map_err(|e| BackupError::Restore(format!("invalid xpub for {name}: {e}")))?;
        keychain
            .save_wallet_xpub(&metadata.id, xpub)
            .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))?;
    }

    // save descriptors and create BDK wallet if present
    if let Some(descs) = &backup.descriptors {
        let ext =
            bdk_wallet::descriptor::ExtendedDescriptor::from_str(&descs.external).map_err(|e| {
                BackupError::Restore(format!("invalid external descriptor for {name}: {e}"))
            })?;
        let int =
            bdk_wallet::descriptor::ExtendedDescriptor::from_str(&descs.internal).map_err(|e| {
                BackupError::Restore(format!("invalid internal descriptor for {name}: {e}"))
            })?;

        keychain
            .save_public_descriptor(&metadata.id, ext.clone(), int.clone())
            .map_err(|e| BackupError::Keychain(format!("descriptors for {name}: {e}")))?;

        // create BDK wallet store from descriptors
        let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, metadata.network)
            .map_err(|e| BackupError::Restore(format!("BDK store for {name}: {e}")))?;

        bdk_wallet::Wallet::create(ext, int)
            .network(metadata.network.into())
            .create_wallet(&mut store.conn)
            .map_err(|e| BackupError::Restore(format!("BDK wallet for {name}: {e}")))?;
    }

    // save tap signer backup inside the cleanup wrapper so failure triggers full rollback
    if let WalletSecret::TapSignerBackup(backup_bytes) = &backup.secret {
        keychain
            .save_tap_signer_backup(&metadata.id, backup_bytes)
            .map_err(|e| BackupError::Keychain(format!("tap signer backup for {name}: {e}")))?;
    }

    db.wallets
        .save_new_wallet_metadata(metadata.clone())
        .map_err(|e| BackupError::Database(format!("metadata for {name}: {e}")))?;

    Ok(())
}

/// Clean up a partially-imported wallet on failure
///
/// Returns a list of cleanup failures; empty means fully cleaned
pub(crate) fn cleanup_failed_wallet(metadata: &WalletMetadata) -> Vec<String> {
    let wallet_id = &metadata.id;
    let name = &metadata.name;
    let mut failures = Vec::new();

    let keychain_ok = Keychain::global().delete_wallet_items(wallet_id);
    if !keychain_ok {
        failures.push(format!("{name}: incomplete keychain deletion"));
    }

    if let Err(e) = crate::wallet::delete_wallet_specific_data(wallet_id) {
        failures.push(format!("{name}: failed to delete wallet data: {e}"));
    }

    let db = Database::global();
    match db.wallets.get_all(metadata.network, metadata.wallet_mode) {
        Ok(mut wallets) => {
            let before = wallets.len();
            wallets.retain(|w| w.id != *wallet_id);
            if wallets.len() < before
                && let Err(e) =
                    db.wallets.save_all_wallets(metadata.network, metadata.wallet_mode, wallets)
            {
                failures.push(format!("{name}: failed to delete metadata: {e}"));
            }
        }
        Err(e) => {
            failures.push(format!("{name}: failed to read wallets for cleanup: {e}"));
        }
    }

    failures
}

fn import_labels(id: &cove_types::WalletId, jsonl: &str) -> Result<(), BackupError> {
    let manager = LabelManager::new(id.clone());
    manager.import(jsonl).map_err(|e| BackupError::Restore(e.to_string()))
}

fn restore_settings(settings: &super::model::AppSettings) -> Result<(), BackupError> {
    let config = &Database::global().global_config;
    let mut errors = Vec::new();

    // skip SelectedNetwork — network is device-specific

    if let Some(fiat) = &settings.selected_fiat_currency
        && let Err(e) = config.set(GlobalConfigKey::SelectedFiatCurrency, fiat.clone())
    {
        errors.push(format!("fiat currency: {e}"));
    }

    if let Some(scheme) = &settings.color_scheme
        && let Err(e) = config.set(GlobalConfigKey::ColorScheme, scheme.clone())
    {
        errors.push(format!("color scheme: {e}"));
    }

    for (network_str, node_json) in &settings.selected_nodes {
        let Ok(network) = Network::try_from(network_str.as_str()) else {
            warn!("skipping unknown network in selected_nodes: {network_str}");
            continue;
        };

        if let Err(e) = serde_json::from_str::<crate::node::Node>(node_json) {
            warn!("skipping invalid node config for {network_str}: {e}");
            continue;
        }

        if let Err(e) = config.set(GlobalConfigKey::SelectedNode(network), node_json.clone()) {
            errors.push(format!("node for {network_str}: {e}"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(BackupError::Database(format!("failed to restore settings: {}", errors.join("; "))))
    }
}

fn wallet_name_from_backup(backup: &WalletBackup) -> String {
    if let Some(name) = backup.metadata.get("name").and_then(|v| v.as_str()) {
        return name.to_string();
    }

    if let Some(id) = backup.metadata.get("id").and_then(|v| v.as_str()) {
        return format!("(id: {id})");
    }

    warn!("wallet backup has no name or id in metadata: {}", backup.metadata);
    "unknown".to_string()
}
