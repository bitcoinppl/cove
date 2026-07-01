use std::{collections::BTreeMap, str::FromStr as _};

use bip39::Mnemonic;
use cove_device::keychain::Keychain;
use cove_types::network::Network;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use crate::database::global_config::{GlobalConfigKey, GlobalConfigTable, GlobalConfigTableError};
use crate::database::{Database, Error as DatabaseError};
use crate::label_manager::LabelManager;
use crate::mnemonic::MnemonicExt as _;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};
use crate::wallet_identity::{
    ExistingWalletIdentitySet, WalletIdentityKey, collect_existing_wallet_identities,
    fallback_identity_key_for_backup, identity_key_for_backup,
};

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

    let mut existing_identities = collect_existing_wallet_identities()?;

    for wallet_backup in &payload.wallets {
        match restore_wallet(wallet_backup, &existing_identities).await {
            Ok(RestoreResult::Imported {
                name,
                labels_imported,
                labels_failure,
                duplicate_key,
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
                existing_identities.insert(duplicate_key);
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
        duplicate_key: WalletIdentityKey,
        degraded: bool,
    },
    Skipped {
        name: String,
    },
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

#[derive(Clone, Copy)]
enum RestoreSaveBehavior {
    BackupAsNewWallet,
    SkipCloudBackup,
}

#[derive(Clone)]
struct RestoredWalletMetadataStore(Database);

impl RestoredWalletMetadataStore {
    fn new(db: &Database) -> Self {
        Self(db.clone())
    }

    fn save(
        &self,
        metadata: &WalletMetadata,
        name: &str,
        save_behavior: RestoreSaveBehavior,
    ) -> Result<(), BackupError> {
        let metadata = metadata.clone_without_local_scan_state();

        let save = match save_behavior {
            RestoreSaveBehavior::BackupAsNewWallet => {
                self.0.wallets.save_new_wallet_metadata(metadata)
            }
            RestoreSaveBehavior::SkipCloudBackup => {
                self.0.wallets.save_restored_wallet_metadata(metadata)
            }
        };

        save.map_err(|e| BackupError::Database(format!("metadata for {name}: {e}")))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum LabelRestoreBehavior {
    MarkCloudBackupDirty,
    PreserveCloudBackupClean,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LabelRestoreWarning {
    pub wallet_name: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct LabelRestoreOutcome {
    pub imported: bool,
    pub warning: Option<LabelRestoreWarning>,
}

async fn restore_wallet(
    backup: &WalletBackup,
    existing_identities: &ExistingWalletIdentitySet,
) -> Result<RestoreResult, RestoreError> {
    let metadata: WalletMetadata = serde_json::from_value(backup.metadata.clone())
        .map_err(|e| BackupError::Deserialization(format!("wallet metadata: {e}")))?;

    let name = metadata.name.clone();
    let wallet_id = metadata.id.clone();

    let duplicate_key = match identity_key_for_backup(&metadata, backup) {
        Ok(duplicate_key) => duplicate_key,
        Err(error) => {
            let fallback_key = fallback_identity_key_for_backup(&metadata);
            if existing_identities.contains(&fallback_key) {
                info!("Skipping wallet {name} - already exists on device");
                return Ok(RestoreResult::Skipped { name });
            }

            return Err(BackupError::from(error).into());
        }
    };

    if existing_identities.contains(&duplicate_key) {
        info!("Skipping wallet {name} - already exists on device");
        return Ok(RestoreResult::Skipped { name });
    }

    let validation = validate_wallet_type_secret(&metadata.wallet_type, &backup.secret, &name)?;
    let cold_missing_backup = validation == WalletTypeSecretValidation::Degraded;

    let mut labels_failure: Option<(String, String)> = None;
    let mut degraded = cold_missing_backup;

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

    let labels_outcome = restore_wallet_labels(
        &wallet_id,
        &name,
        backup.labels_jsonl.as_deref(),
        LabelRestoreBehavior::MarkCloudBackupDirty,
    );
    let labels_imported = labels_outcome.imported;
    if let Some(warning) = labels_outcome.warning {
        warn!("Failed to import labels for wallet {name}: {}", warning.error);
        labels_failure = Some((warning.wallet_name, warning.error));
    }

    Ok(RestoreResult::Imported { name, labels_imported, labels_failure, duplicate_key, degraded })
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
    with_cleanup(metadata, || {
        restore_mnemonic_wallet_inner(metadata, mnemonic, RestoreSaveBehavior::BackupAsNewWallet)
    })
}

pub(crate) fn restore_cloud_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), (BackupError, Vec<String>)> {
    with_cleanup(metadata, || {
        restore_mnemonic_wallet_inner(metadata, mnemonic, RestoreSaveBehavior::SkipCloudBackup)
    })
}

fn restore_mnemonic_wallet_inner(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
    save_behavior: RestoreSaveBehavior,
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

    RestoredWalletMetadataStore::new(&db).save(metadata, name, save_behavior)?;

    Ok(())
}

pub(crate) fn restore_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), (BackupError, Vec<String>)> {
    with_cleanup(metadata, || {
        restore_descriptor_wallet_inner(metadata, backup, RestoreSaveBehavior::BackupAsNewWallet)
    })
}

pub(crate) fn restore_cloud_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), (BackupError, Vec<String>)> {
    with_cleanup(metadata, || {
        restore_descriptor_wallet_inner(metadata, backup, RestoreSaveBehavior::SkipCloudBackup)
    })
}

fn restore_descriptor_wallet_inner(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
    save_behavior: RestoreSaveBehavior,
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

    RestoredWalletMetadataStore::new(&db).save(metadata, name, save_behavior)?;

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

fn import_labels(id: &WalletId, jsonl: &str) -> Result<(), BackupError> {
    let manager = LabelManager::new(id.clone());
    manager.import(jsonl).map_err(|e| BackupError::Restore(e.to_string()))
}

pub(crate) fn restore_wallet_labels(
    wallet_id: &WalletId,
    wallet_name: &str,
    labels_jsonl: Option<&str>,
    behavior: LabelRestoreBehavior,
) -> LabelRestoreOutcome {
    let Some(jsonl) = labels_jsonl.filter(|jsonl| !jsonl.is_empty()) else {
        return LabelRestoreOutcome::default();
    };

    let manager = LabelManager::new(wallet_id.clone());
    let import_result = match behavior {
        LabelRestoreBehavior::MarkCloudBackupDirty => import_labels(wallet_id, jsonl),
        LabelRestoreBehavior::PreserveCloudBackupClean => manager
            .import_without_cloud_backup_dirty(jsonl)
            .map_err(|error| BackupError::Restore(error.to_string())),
    };

    match import_result {
        Ok(()) => LabelRestoreOutcome { imported: true, warning: None },
        Err(error) => LabelRestoreOutcome {
            imported: false,
            warning: Some(LabelRestoreWarning {
                wallet_name: wallet_name.to_string(),
                error: error.to_string(),
            }),
        },
    }
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

    errors.extend(restore_custom_block_explorers(config, &settings.custom_block_explorers));

    if errors.is_empty() {
        Ok(())
    } else {
        Err(BackupError::Database(format!("failed to restore settings: {}", errors.join("; "))))
    }
}

fn restore_custom_block_explorers(
    config: &GlobalConfigTable,
    custom_block_explorers: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();

    for (network_str, template) in custom_block_explorers {
        let Ok(network) = Network::try_from(network_str.as_str()) else {
            warn!("skipping unknown network in custom_block_explorers: {network_str}");
            continue;
        };

        if template.trim().is_empty() {
            warn!("skipping empty custom block explorer for {network_str}");
            continue;
        }

        if let Err(error) = config.set_custom_block_explorer(network, template.clone()) {
            warn!("skipping invalid custom block explorer for {network_str}: {error}");
            if !matches!(
                error,
                DatabaseError::GlobalConfig(GlobalConfigTableError::InvalidCustomBlockExplorer(_))
            ) {
                errors.push(format!("custom block explorer for {network_str}: {error}"));
            }
        }
    }

    errors
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr as _;
    use std::sync::Arc;
    use std::time::Duration;

    use cove_types::BlockSizeLast;

    use crate::wallet::fingerprint::Fingerprint;
    use crate::wallet::metadata::StoreType;

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

    fn invalid_descriptor_wallet(metadata: &WalletMetadata) -> WalletBackup {
        WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret: WalletSecret::None,
            descriptors: Some(crate::backup::model::DescriptorPair {
                external: "not a descriptor".to_string(),
                internal: "also not a descriptor".to_string(),
            }),
            xpub: None,
            labels_jsonl: None,
        }
    }

    #[test]
    fn backup_import_restores_valid_custom_block_explorers() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, config) = test_config();
        let explorers =
            BTreeMap::from([("Bitcoin".to_string(), "https://example.com".to_string())]);

        let errors = restore_custom_block_explorers(&config, &explorers);

        assert!(errors.is_empty());
        assert_eq!(
            config.custom_block_explorer(Network::Bitcoin).as_deref(),
            Some("https://example.com/tx/{txid}")
        );
    }

    #[test]
    fn backup_import_skips_invalid_custom_block_explorer_without_clearing_existing() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, config) = test_config();
        config
            .set_custom_block_explorer(Network::Bitcoin, "https://existing.example".to_string())
            .unwrap();
        let explorers = BTreeMap::from([
            ("Bitcoin".to_string(), "https://bad.example/{address}".to_string()),
            ("Signet".to_string(), "   ".to_string()),
            ("unknown".to_string(), "https://ignored.example".to_string()),
        ]);

        let errors = restore_custom_block_explorers(&config, &explorers);

        assert!(errors.is_empty());
        assert_eq!(
            config.custom_block_explorer(Network::Bitcoin).as_deref(),
            Some("https://existing.example/tx/{txid}")
        );
        assert_eq!(config.custom_block_explorer(Network::Signet), None);
    }

    fn test_config() -> (tempfile::TempDir, GlobalConfigTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = GlobalConfigTable::new(db.clone(), &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }

    #[test]
    fn restored_metadata_store_clears_local_scan_state() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        let db = Database::global();
        let mut metadata = hot_metadata("Restored wallet");
        metadata.internal.address_index =
            Some(cove_types::AddressIndex { last_seen_index: 4, address_list_hash: 2 });
        metadata.internal.last_scan_finished = Some(Duration::from_secs(10));
        metadata.internal.last_height_fetched =
            Some(BlockSizeLast { block_height: 1, last_seen: Duration::from_secs(20) });
        metadata.internal.performed_full_scan_at = Some(30);
        metadata.internal.store_type = StoreType::FileStore;

        RestoredWalletMetadataStore::new(&db)
            .save(&metadata, &metadata.name, RestoreSaveBehavior::SkipCloudBackup)
            .unwrap();

        let restored =
            db.wallets.get(&metadata.id, metadata.network, metadata.wallet_mode).unwrap().unwrap();

        assert_eq!(restored.internal.address_index, None);
        assert_eq!(restored.internal.last_scan_finished, None);
        assert_eq!(restored.internal.last_height_fetched, None);
        assert_eq!(restored.internal.performed_full_scan_at, None);
        assert_eq!(restored.internal.store_type, StoreType::FileStore);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_wallet_skips_duplicate_before_secret_validation() {
        let metadata = hot_metadata("Existing hot wallet");
        let backup = WalletBackup {
            metadata: serde_json::to_value(&metadata).unwrap(),
            secret: WalletSecret::Unknown,
            descriptors: None,
            xpub: None,
            labels_jsonl: None,
        };
        let duplicate_key = identity_key_for_backup(&metadata, &backup).unwrap();
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(duplicate_key);

        match restore_wallet(&backup, &existing_identities).await {
            Ok(RestoreResult::Skipped { name }) => assert_eq!(name, metadata.name),
            Ok(RestoreResult::Imported { .. }) => panic!("expected duplicate skip"),
            Err(error) => panic!("expected duplicate skip, got {}", error.error),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_wallet_skips_duplicate_with_invalid_public_identity() {
        let metadata = cold_metadata("Existing malformed public wallet");
        let backup = invalid_descriptor_wallet(&metadata);
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(fallback_identity_key_for_backup(&metadata));

        match restore_wallet(&backup, &existing_identities).await {
            Ok(RestoreResult::Skipped { name }) => assert_eq!(name, metadata.name),
            Ok(RestoreResult::Imported { .. }) => panic!("expected duplicate skip"),
            Err(error) => panic!("expected duplicate skip, got {}", error.error),
        }
    }
}
