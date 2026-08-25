use std::{
    collections::{BTreeMap, HashMap, HashSet},
    str::FromStr as _,
    sync::Arc,
};

use bdk_wallet::{bitcoin::bip32::Xpub, descriptor::ExtendedDescriptor};
use bip39::Mnemonic;
use cove_device::keychain::{Keychain, WalletSecret as KeychainWalletSecret, WalletXprv};
use cove_types::network::Network;
use cove_util::result_ext::ResultExt as _;
use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use crate::database::global_config::{GlobalConfigKey, GlobalConfigTable, GlobalConfigTableError};
use crate::database::{Database, Error as DatabaseError};
use crate::keys::Descriptors;
use crate::label_manager::LabelManager;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};
use crate::wallet_identity::{
    ExistingWalletIdentitySet, WalletIdentityKey, collect_existing_wallet_identities,
    identity_key_for_backup,
};
use crate::wallet_secret::WalletSecretExt as _;

use super::crypto;
use super::error::BackupError;
use super::model::{BackupImportReport, BackupPayload, WalletBackup, WalletSecret};
use super::recovery::{
    RestoreArtifactSnapshot, RestoreMarkerGuard, ValidatedRestoreWalletId, WalletRestoreLease,
};

#[derive(Debug)]
pub(crate) struct PreparedImportWallet {
    pub(crate) metadata: WalletMetadata,
    pub(crate) snapshot: RestoreArtifactSnapshot,
    identity: WalletIdentityKey,
    kind: PreparedWalletKind,
}

#[derive(Debug)]
enum PreparedWalletKind {
    Hot(PreparedHotWallet),
    Public(PreparedPublicWallet),
}

#[derive(Debug)]
struct PreparedHotWallet {
    secret: KeychainWalletSecret,
    xpub: Xpub,
    descriptors: Descriptors,
}

#[derive(Debug)]
struct PreparedPublicWallet {
    xpub: Option<Xpub>,
    descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
    tap_signer_backup: Option<Vec<u8>>,
    degraded: bool,
}

#[derive(Debug)]
pub(crate) struct ImportPreparationState {
    pub(crate) payload: Option<BackupPayload>,
    pub(crate) payload_digest: String,
    pub(crate) wallets: Vec<PreparedImportWallet>,
}

#[derive(Debug)]
pub(crate) struct ImportApprovalState {
    pub(crate) payload_digest: String,
    pub(crate) snapshots: HashMap<String, RestoreArtifactSnapshot>,
}

pub(crate) type SharedImportPreparation = Arc<Mutex<Option<ImportPreparationState>>>;
pub(crate) type SharedImportApproval = Arc<Mutex<Option<ImportApprovalState>>>;

pub(crate) async fn prepare_import(
    data: Vec<u8>,
    password: String,
) -> Result<ImportPreparationState, BackupError> {
    let password = Zeroizing::new(password);
    let password = crypto::clean_password(&password)?;

    let decrypted = crypto::decrypt(&data, &password)?;
    let decompressed = crypto::decompress(&decrypted)?;
    let payload_digest = hex::encode(Sha256::digest(&decompressed));
    let payload = BackupPayload::decode(&decompressed)?;
    let wallets = preflight_wallets(&payload)?;

    Ok(ImportPreparationState { payload: Some(payload), payload_digest, wallets })
}

pub(crate) fn validate_prepared_import(
    preparation: &ImportPreparationState,
    approval: Option<&ImportApprovalState>,
) -> Result<(), BackupError> {
    let payload = preparation.payload.as_ref().ok_or(BackupError::ImportApprovalUsed)?;
    if payload.wallets.len() != preparation.wallets.len() {
        return Err(BackupError::Restore(
            "prepared import wallet plans do not match the backup payload".to_string(),
        ));
    }

    validate_approval(preparation, approval)?;

    if approval.is_none()
        && let Some(wallet) =
            preparation.wallets.iter().find(|wallet| wallet.snapshot.has_markerless_conflict())
    {
        return Err(BackupError::ImportApprovalRequired(wallet.metadata.id.clone()));
    }

    Ok(())
}

pub(crate) async fn import_prepared(
    mut preparation: ImportPreparationState,
    approval: Option<ImportApprovalState>,
) -> Result<BackupImportReport, BackupError> {
    validate_prepared_import(&preparation, approval.as_ref())?;
    let mut payload =
        preparation.payload.take().expect("prepared import payload was checked as present");
    let mut approval = approval;

    let mut progress = WalletImportProgress {
        report: BackupImportReport::default(),
        existing_identities: collect_existing_wallet_identities()?,
    };
    let had_wallets = !payload.wallets.is_empty();
    let wallet_backups = std::mem::take(&mut payload.wallets);

    for (wallet_backup, prepared_wallet) in wallet_backups.into_iter().zip(preparation.wallets) {
        let prepared_name = prepared_wallet.metadata.name.clone();
        let approval_snapshot = approval
            .as_mut()
            .and_then(|approval| approval.snapshots.remove(prepared_wallet.metadata.id.as_str()))
            .as_ref()
            .cloned();

        if prepared_wallet.snapshot.has_markerless_conflict() && approval_snapshot.is_none() {
            let error = BackupError::ImportApprovalRequired(prepared_wallet.metadata.id.clone());
            progress.record_failure(prepared_name, RestoreError::from(error));
            continue;
        }

        let restore_result = restore_prepared_wallet(
            wallet_backup,
            prepared_wallet,
            &progress.existing_identities,
            approval_snapshot.as_ref(),
        )
        .await;

        progress.record_restore_result(prepared_name, restore_result);
    }

    let mut report = progress.report;

    if report.imported_wallet_names.is_empty()
        && report.skipped_wallet_names.is_empty()
        && !report.failed_wallet_names.is_empty()
    {
        return Err(BackupError::Restore("All wallets failed to import".to_string()));
    }

    // restore settings only if at least one wallet imported, or backup was settings-only
    if !report.imported_wallet_names.is_empty()
        || !had_wallets
        || !report.skipped_wallet_names.is_empty()
    {
        match restore_settings(&payload.settings) {
            Ok(()) => report.settings_restored = true,
            Err(e) => {
                warn!("failed to restore some settings: {e}");
                report.settings_error = Some(e.to_string());
            }
        }
    }

    Ok(report.finalize())
}

fn preflight_wallets(payload: &BackupPayload) -> Result<Vec<PreparedImportWallet>, BackupError> {
    let mut seen_path_keys = HashSet::with_capacity(payload.wallets.len());
    let mut wallets = Vec::with_capacity(payload.wallets.len());

    for wallet_backup in &payload.wallets {
        let metadata: WalletMetadata = serde_json::from_value(wallet_backup.metadata.clone())
            .map_err_prefix("wallet metadata", BackupError::Deserialization)?;

        let validated_id =
            crate::backup::recovery::ValidatedRestoreWalletId::validate(&metadata.id)?;

        let path_key = validated_id.path_key();

        if !seen_path_keys.insert(path_key) {
            return Err(BackupError::InvalidWalletId(format!(
                "duplicate or case-folded wallet id: {}",
                metadata.id
            )));
        }

        let validation = validate_wallet_type_secret(
            &metadata.wallet_type,
            &wallet_backup.secret,
            &metadata.name,
        )?;

        let kind = prepare_wallet_kind(&metadata, wallet_backup, validation)?;
        let identity =
            identity_key_for_backup(&metadata, wallet_backup).map_err_str(BackupError::Restore)?;

        let snapshot = RestoreArtifactSnapshot::capture(&validated_id)?;
        wallets.push(PreparedImportWallet { metadata, snapshot, identity, kind });
    }

    Ok(wallets)
}

fn prepare_wallet_kind(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
    validation: WalletTypeSecretValidation,
) -> Result<PreparedWalletKind, BackupError> {
    let kind = match &backup.secret {
        WalletSecret::Mnemonic(words) => {
            let mnemonic = Mnemonic::from_str(words).map_err_prefix(
                &format!("invalid mnemonic for {}", metadata.name),
                BackupError::Restore,
            )?;

            let secret = KeychainWalletSecret::Mnemonic(mnemonic);
            let xpub = secret.xpub(metadata.network);
            let descriptors =
                secret.clone().into_descriptors(metadata.network, metadata.address_type);

            Ok(PreparedWalletKind::Hot(PreparedHotWallet { secret, xpub, descriptors }))
        }
        WalletSecret::Xprv(value) => {
            let xprv = WalletXprv::parse(value.as_str()).map_err_prefix(
                &format!("invalid extended private key for {}", metadata.name),
                BackupError::Restore,
            )?;

            let secret = KeychainWalletSecret::Xpriv(xprv);
            let xpub = secret.xpub(metadata.network);
            let descriptors =
                secret.clone().into_descriptors(metadata.network, metadata.address_type);

            Ok(PreparedWalletKind::Hot(PreparedHotWallet { secret, xpub, descriptors }))
        }
        WalletSecret::TapSignerBackup(backup_bytes) => {
            let public = prepare_public_wallet(
                backup,
                metadata,
                validation == WalletTypeSecretValidation::Degraded,
            )?;
            Ok(PreparedWalletKind::Public(PreparedPublicWallet {
                tap_signer_backup: Some(backup_bytes.clone()),
                ..public
            }))
        }
        WalletSecret::None | WalletSecret::Unknown => prepare_public_wallet(
            backup,
            metadata,
            validation == WalletTypeSecretValidation::Degraded,
        )
        .map(PreparedWalletKind::Public),
    }?;

    validate_prepared_wallet_storage(metadata, &kind)?;
    Ok(kind)
}

fn validate_prepared_wallet_storage(
    metadata: &WalletMetadata,
    kind: &PreparedWalletKind,
) -> Result<(), BackupError> {
    let mut connection = bdk_wallet::rusqlite::Connection::open_in_memory()
        .map_err_prefix("validate BDK wallet", BackupError::Restore)?;

    match kind {
        PreparedWalletKind::Hot(prepared) => {
            prepared
                .descriptors
                .clone()
                .into_create_params()
                .network(metadata.network.into())
                .create_wallet(&mut connection)
                .map_err_prefix("validate BDK wallet", BackupError::Restore)?;
        }
        PreparedWalletKind::Public(prepared) => {
            let Some((external, internal)) = &prepared.descriptors else {
                return Ok(());
            };

            bdk_wallet::Wallet::create(external.clone(), internal.clone())
                .network(metadata.network.into())
                .create_wallet(&mut connection)
                .map_err_prefix("validate BDK wallet", BackupError::Restore)?;
        }
    }

    Ok(())
}

fn prepare_public_wallet(
    backup: &WalletBackup,
    metadata: &WalletMetadata,
    degraded: bool,
) -> Result<PreparedPublicWallet, BackupError> {
    let xpub = backup.xpub.as_deref().map(Xpub::from_str).transpose().map_err(|error| {
        BackupError::Restore(format!("invalid xpub for {}: {error}", metadata.name))
    })?;

    let descriptors = backup
        .descriptors
        .as_ref()
        .map(|descriptors| {
            let external =
                ExtendedDescriptor::from_str(&descriptors.external).map_err(|error| {
                    BackupError::Restore(format!(
                        "invalid external descriptor for {}: {error}",
                        metadata.name
                    ))
                })?;

            let internal =
                ExtendedDescriptor::from_str(&descriptors.internal).map_err(|error| {
                    BackupError::Restore(format!(
                        "invalid internal descriptor for {}: {error}",
                        metadata.name
                    ))
                })?;

            Ok::<_, BackupError>((external, internal))
        })
        .transpose()?;

    if xpub.is_none() && descriptors.is_none() {
        return Err(BackupError::Restore(format!(
            "wallet {} has no xpub or descriptors, cannot restore",
            metadata.name
        )));
    }

    Ok(PreparedPublicWallet { xpub, descriptors, tap_signer_backup: None, degraded })
}

/// Check that an approval belongs to this exact payload and covers exactly the
/// wallets that need cleanup approval
///
/// The snapshots themselves are rechecked when the approval is created and
/// again while the per-wallet restore lease is held
fn validate_approval(
    preparation: &ImportPreparationState,
    approval: Option<&ImportApprovalState>,
) -> Result<(), BackupError> {
    let Some(approval) = approval else {
        return Ok(());
    };

    if approval.payload_digest != preparation.payload_digest {
        return Err(BackupError::ImportApprovalStale(
            preparation
                .wallets
                .first()
                .map(|wallet| wallet.metadata.id.clone())
                .unwrap_or_default(),
        ));
    }

    let required_ids = preparation
        .wallets
        .iter()
        .filter(|wallet| wallet.snapshot.has_markerless_conflict())
        .map(|wallet| wallet.metadata.id.as_str())
        .collect::<HashSet<_>>();
    if approval.snapshots.len() != required_ids.len()
        || approval.snapshots.keys().any(|id| !required_ids.contains(id.as_str()))
    {
        return Err(BackupError::ImportApprovalStale(
            preparation
                .wallets
                .first()
                .map(|wallet| wallet.metadata.id.clone())
                .unwrap_or_default(),
        ));
    }

    Ok(())
}

struct RestoreError {
    error: BackupError,
    cleanup_warnings: Vec<String>,
}

struct WalletImportProgress {
    report: BackupImportReport,
    existing_identities: ExistingWalletIdentitySet,
}

impl WalletImportProgress {
    fn record_restore_result(
        &mut self,
        prepared_name: String,
        result: Result<RestoreResult, RestoreError>,
    ) {
        match result {
            Ok(RestoreResult::Imported {
                name,
                labels_imported,
                labels_failure,
                duplicate_key,
                degraded,
                cleanup_warnings,
            }) => {
                self.report.imported_wallet_names.push(name.clone());
                self.report.wallets_with_labels_imported += u32::from(labels_imported);

                if let Some((name, error)) = labels_failure {
                    self.report.labels_failed_wallet_names.push(name);
                    self.report.labels_failed_errors.push(error);
                }

                self.existing_identities.insert(duplicate_key);
                if degraded {
                    self.report.degraded_wallet_names.push(name);
                }

                self.report.cleanup_warnings.extend(cleanup_warnings);
            }

            Ok(RestoreResult::Skipped { name }) => self.report.skipped_wallet_names.push(name),
            Err(error) => self.record_failure(prepared_name, error),
        }
    }

    fn record_failure(&mut self, name: String, failure: RestoreError) {
        let error = &failure.error;
        error!("Failed to restore wallet {name}: {error}");

        for warning in &failure.cleanup_warnings {
            error!("Cleanup failure for {name}: {warning}");
        }

        self.report.failed_wallet_names.push(name);
        self.report.failed_wallet_errors.push(failure.error.to_string());
        self.report.cleanup_warnings.extend(failure.cleanup_warnings);
    }
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
        cleanup_warnings: Vec<String>,
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
        (WalletType::Hot, WalletSecret::Mnemonic(_) | WalletSecret::Xprv(_))
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

#[derive(Clone)]
struct RestoredWalletMetadataStore(Database);

impl RestoredWalletMetadataStore {
    fn new(db: &Database) -> Self {
        Self(db.clone())
    }

    fn save(&self, metadata: &WalletMetadata, name: &str) -> Result<(), BackupError> {
        let metadata = metadata.clone_without_local_scan_state();

        let save = self.0.wallets.save_restored_wallet_metadata(metadata);

        save.map_err(|e| BackupError::Database(format!("metadata for {name}: {e}")))
    }
}

fn schedule_cloud_backup_after_local_commit(metadata: &WalletMetadata) {
    crate::manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER
        .backup_new_wallet(metadata.clone_without_local_scan_state());
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

async fn restore_prepared_wallet(
    backup: WalletBackup,
    prepared: PreparedImportWallet,
    existing_identities: &ExistingWalletIdentitySet,
    approval_snapshot: Option<&RestoreArtifactSnapshot>,
) -> Result<RestoreResult, RestoreError> {
    let PreparedImportWallet { metadata, identity, kind, .. } = prepared;
    let name = metadata.name.clone();
    let wallet_id = metadata.id.clone();

    let duplicate_key = identity;

    if existing_identities.contains(&duplicate_key) {
        info!("Skipping wallet {name} - already exists on device");
        return Ok(RestoreResult::Skipped { name });
    }

    let mut labels_failure: Option<(String, String)> = None;
    let degraded = matches!(&kind, PreparedWalletKind::Public(public) if public.degraded);

    let cleanup = match approval_snapshot {
        Some(snapshot) => RestoreCleanup::Approved(snapshot),
        None => RestoreCleanup::RequireEmpty,
    };

    let cleanup_warnings = match kind {
        PreparedWalletKind::Hot(prepared_hot) => {
            restore_hot_wallet_prepared_with_context(&metadata, prepared_hot, cleanup)
                .map_err(|(e, warnings)| RestoreError { error: e, cleanup_warnings: warnings })?
        }
        PreparedWalletKind::Public(prepared_public) => {
            if degraded {
                warn!(
                    "wallet {name} has an unrecognized secret type, importing as descriptor-only"
                );
            }
            restore_descriptor_wallet_prepared_with_context(&metadata, prepared_public, cleanup)
                .map_err(|(e, warnings)| RestoreError { error: e, cleanup_warnings: warnings })?
        }
    };

    let labels_outcome = restore_wallet_labels(
        &wallet_id,
        &name,
        backup.labels_jsonl.as_deref(),
        LabelRestoreBehavior::MarkCloudBackupDirty,
    );
    let labels_imported = labels_outcome.imported;
    if let Some(warning) = labels_outcome.warning {
        let error = &warning.error;
        warn!("failed to import labels for wallet {name}: {error}");
        labels_failure = Some((warning.wallet_name, warning.error));
    }

    Ok(RestoreResult::Imported {
        name,
        labels_imported,
        labels_failure,
        duplicate_key,
        degraded,
        cleanup_warnings,
    })
}

/// How a restore may treat the local artifacts a wallet id already owns
#[derive(Clone, Copy)]
enum RestoreCleanup<'a> {
    /// Fail unless the wallet id is free of every local artifact
    RequireEmpty,
    /// Delete exactly the approved artifacts before restoring over them
    Approved(&'a RestoreArtifactSnapshot),
    /// Restore alongside exactly these artifacts, deleting none of them
    Preserve(&'a RestoreArtifactSnapshot),
}

fn with_restore_journal<F>(
    metadata: &WalletMetadata,
    cleanup: RestoreCleanup<'_>,
    f: F,
) -> Result<Vec<String>, (BackupError, Vec<String>)>
where
    F: FnOnce() -> Result<(), BackupError>,
{
    let _construction = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
        .begin_construction(metadata.id.clone())
        .map_err(|error| (BackupError::Restore(error.to_string()), Vec::new()))?;

    let lease = match cleanup {
        RestoreCleanup::RequireEmpty => WalletRestoreLease::acquire(metadata),
        RestoreCleanup::Approved(snapshot) | RestoreCleanup::Preserve(snapshot) => {
            WalletRestoreLease::acquire_for_approval(metadata, snapshot)
        }
    }
    .map_err(|error| (error, Vec::new()))?;

    let mut journal =
        RestoreMarkerGuard::begin(metadata, lease).map_err(|error| (error, Vec::new()))?;

    if matches!(cleanup, RestoreCleanup::Approved(_))
        && let Err(error) = journal.remove_approved_conflicts()
    {
        let cleanup_warnings = journal.rollback();
        return Err((error, cleanup_warnings));
    }

    match f() {
        Ok(()) => Ok(journal.commit()),
        Err(error) => {
            let cleanup_warnings = journal.rollback();
            Err((error, cleanup_warnings))
        }
    }
}

fn cloud_restore_snapshot(
    metadata: &WalletMetadata,
    expected_xpub: Option<Xpub>,
) -> Result<RestoreArtifactSnapshot, BackupError> {
    let id = ValidatedRestoreWalletId::validate(&metadata.id)?;
    let snapshot = RestoreArtifactSnapshot::capture(&id)?;

    if snapshot.metadata || !snapshot.bdk_paths.is_empty() || snapshot.wallet_data_occupied {
        return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
    }

    let has_non_xpub_keychain_item = snapshot.keychain_entries.keys().any(|kind| kind != "xpub");
    if has_non_xpub_keychain_item {
        return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
    }

    let existing_xpub = Keychain::global()
        .get_wallet_xpub(&metadata.id)
        .map_err(|error| BackupError::Keychain(format!("cloud restore xpub: {error}")))?;

    if existing_xpub.is_some() && existing_xpub != expected_xpub {
        return Err(BackupError::WalletIdOccupied(metadata.id.clone()));
    }

    Ok(snapshot)
}

fn restore_hot_wallet_prepared_with_context(
    metadata: &WalletMetadata,
    prepared: PreparedHotWallet,
    cleanup: RestoreCleanup<'_>,
) -> Result<Vec<String>, (BackupError, Vec<String>)> {
    let result = with_restore_journal(metadata, cleanup, || {
        restore_hot_wallet_inner_prepared(metadata, prepared)
    });
    if result.is_ok() {
        schedule_cloud_backup_after_local_commit(metadata);
    }

    result
}

fn restore_descriptor_wallet_prepared_with_context(
    metadata: &WalletMetadata,
    prepared: PreparedPublicWallet,
    cleanup: RestoreCleanup<'_>,
) -> Result<Vec<String>, (BackupError, Vec<String>)> {
    let result = with_restore_journal(metadata, cleanup, || {
        restore_descriptor_wallet_inner_prepared(metadata, prepared)
    });
    if result.is_ok() {
        schedule_cloud_backup_after_local_commit(metadata);
    }

    result
}

pub(crate) fn restore_cloud_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), (BackupError, Vec<String>)> {
    let secret = KeychainWalletSecret::Mnemonic(mnemonic);
    let xpub = secret.xpub(metadata.network);
    let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
    let prepared = PreparedHotWallet { secret, xpub, descriptors };
    let snapshot = cloud_restore_snapshot(metadata, Some(prepared.xpub))
        .map_err(|error| (error, Vec::new()))?;

    let result = with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
        restore_hot_wallet_inner_prepared(metadata, prepared)
    });
    if let Ok(warnings) = &result {
        let name = &metadata.name;
        for warning in warnings {
            warn!("cloud restore cleanup warning for {name}: {warning}");
        }
    }
    result.map(|_| ())
}

pub(crate) fn restore_cloud_xpriv_wallet(
    metadata: &WalletMetadata,
    xpriv: WalletXprv,
) -> Result<(), (BackupError, Vec<String>)> {
    let secret = KeychainWalletSecret::Xpriv(xpriv);
    let xpub = secret.xpub(metadata.network);
    let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
    let prepared = PreparedHotWallet { secret, xpub, descriptors };
    let snapshot = cloud_restore_snapshot(metadata, Some(prepared.xpub))
        .map_err(|error| (error, Vec::new()))?;

    let result = with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
        restore_hot_wallet_inner_prepared(metadata, prepared)
    });
    if let Ok(warnings) = &result {
        let name = &metadata.name;
        for warning in warnings {
            warn!("cloud restore cleanup warning for {name}: {warning}");
        }
    }
    result.map(|_| ())
}

fn restore_hot_wallet_inner_prepared(
    metadata: &WalletMetadata,
    prepared: PreparedHotWallet,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;
    let network = metadata.network;
    let PreparedHotWallet { secret, xpub, descriptors } = prepared;

    let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, network)
        .map_err(|e| BackupError::Restore(format!("BDK store for {name}: {e}")))?;

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
        .save_wallet_secret(&metadata.id, secret)
        .map_err(|e| BackupError::Keychain(format!("private key for {name}: {e}")))?;

    keychain
        .save_wallet_xpub(&metadata.id, xpub)
        .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))?;

    keychain
        .save_public_descriptor(&metadata.id, ext_descriptor, int_descriptor)
        .map_err(|e| BackupError::Keychain(format!("descriptors for {name}: {e}")))?;

    RestoredWalletMetadataStore::new(&db).save(metadata, name)?;

    Ok(())
}

pub(crate) fn restore_cloud_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), (BackupError, Vec<String>)> {
    let prepared =
        prepare_public_wallet(backup, metadata, matches!(&backup.secret, WalletSecret::Unknown))
            .map_err(|error| (error, Vec::new()))?;

    let prepared = PreparedPublicWallet {
        tap_signer_backup: match &backup.secret {
            WalletSecret::TapSignerBackup(bytes) => Some(bytes.clone()),
            _ => None,
        },
        ..prepared
    };
    let snapshot =
        cloud_restore_snapshot(metadata, prepared.xpub).map_err(|error| (error, Vec::new()))?;

    let result = with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
        restore_descriptor_wallet_inner_prepared(metadata, prepared)
    });
    if let Ok(warnings) = &result {
        let name = &metadata.name;
        for warning in warnings {
            warn!("cloud restore cleanup warning for {name}: {warning}");
        }
    }
    result.map(|_| ())
}

fn restore_descriptor_wallet_inner_prepared(
    metadata: &WalletMetadata,
    prepared: PreparedPublicWallet,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;

    let PreparedPublicWallet { xpub, descriptors, tap_signer_backup, .. } = prepared;

    if let Some(xpub) = xpub {
        keychain
            .save_wallet_xpub(&metadata.id, xpub)
            .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))?;
    }

    // save descriptors and create BDK wallet if present
    if let Some((ext, int)) = descriptors {
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
    if let Some(backup_bytes) = tap_signer_backup {
        keychain
            .save_tap_signer_backup(&metadata.id, &backup_bytes)
            .map_err(|e| BackupError::Keychain(format!("tap signer backup for {name}: {e}")))?;
    }

    RestoredWalletMetadataStore::new(&db).save(metadata, name)?;

    Ok(())
}

fn import_labels(id: &WalletId, jsonl: &str) -> Result<(), BackupError> {
    let manager = LabelManager::try_new(id.clone())
        .map_err(|error| BackupError::Restore(error.to_string()))?;
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

    let manager = LabelManager::try_new(wallet_id.clone());
    let import_result = match behavior {
        LabelRestoreBehavior::MarkCloudBackupDirty => import_labels(wallet_id, jsonl),
        LabelRestoreBehavior::PreserveCloudBackupClean => {
            manager.map_err(|error| BackupError::Restore(error.to_string())).and_then(|manager| {
                manager
                    .import_without_cloud_backup_dirty(jsonl)
                    .map_err(|error| BackupError::Restore(error.to_string()))
            })
        }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
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
    fn unknown_hot_wallet_secret_is_a_hard_failure() {
        let result =
            validate_wallet_type_secret(&WalletType::Hot, &WalletSecret::Unknown, "Hot wallet");

        assert!(
            matches!(result, Err(BackupError::Restore(message)) if message.contains("hot wallet"))
        );
    }

    #[test]
    fn unknown_non_hot_wallet_secrets_are_degraded() {
        for wallet_type in [WalletType::Cold, WalletType::XpubOnly, WalletType::WatchOnly] {
            let result =
                validate_wallet_type_secret(&wallet_type, &WalletSecret::Unknown, "Public wallet");

            assert_eq!(result.unwrap(), WalletTypeSecretValidation::Degraded);
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

        RestoredWalletMetadataStore::new(&db).save(&metadata, &metadata.name).unwrap();

        let restored =
            db.wallets.get(&metadata.id, metadata.network, metadata.wallet_mode).unwrap().unwrap();

        assert_eq!(restored.internal.address_index, None);
        assert_eq!(restored.internal.last_scan_finished, None);
        assert_eq!(restored.internal.last_height_fetched, None);
        assert_eq!(restored.internal.performed_full_scan_at, None);
        assert_eq!(restored.internal.store_type, StoreType::FileStore);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_wallet_skips_duplicate_after_preflight() {
        let metadata = hot_metadata("Existing hot wallet");
        let backup = WalletBackup {
            metadata: serde_json::to_value(&metadata).unwrap(),
            secret: WalletSecret::Mnemonic(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                    .to_string(),
            ),
            descriptors: None,
            xpub: None,
            labels_jsonl: None,
        };

        let duplicate_key = identity_key_for_backup(&metadata, &backup).unwrap();
        let mut existing_identities = ExistingWalletIdentitySet::default();
        existing_identities.insert(duplicate_key);
        let validation =
            validate_wallet_type_secret(&metadata.wallet_type, &backup.secret, &metadata.name)
                .unwrap();
        let prepared_kind = prepare_wallet_kind(&metadata, &backup, validation).unwrap();
        let prepared = PreparedImportWallet {
            metadata: metadata.clone(),
            snapshot: RestoreArtifactSnapshot::default(),
            identity: identity_key_for_backup(&metadata, &backup).unwrap(),
            kind: prepared_kind,
        };

        match restore_prepared_wallet(backup, prepared, &existing_identities, None).await {
            Ok(RestoreResult::Skipped { name }) => assert_eq!(name, metadata.name),
            Ok(RestoreResult::Imported { .. }) => panic!("expected duplicate skip"),
            Err(error) => panic!("expected duplicate skip, got {}", error.error),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn preflight_rejects_invalid_public_identity() {
        let metadata = cold_metadata("Existing malformed public wallet");
        let backup = invalid_descriptor_wallet(&metadata);
        let validation =
            validate_wallet_type_secret(&metadata.wallet_type, &backup.secret, &metadata.name)
                .unwrap();
        let result = prepare_wallet_kind(&metadata, &backup, validation);

        assert!(
            matches!(result, Err(BackupError::Restore(message)) if message.contains("descriptor"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prepared_import_rejects_mismatched_wallet_plan_batch() {
        let payload = BackupPayload {
            version: crate::backup::model::PAYLOAD_VERSION,
            created_at: 0,
            wallets: vec![WalletBackup {
                metadata: serde_json::Value::Null,
                secret: WalletSecret::None,
                descriptors: None,
                xpub: None,
                labels_jsonl: None,
            }],
            settings: crate::backup::model::AppSettings {
                selected_network: None,
                selected_fiat_currency: None,
                color_scheme: None,
                selected_nodes: Vec::new(),
                custom_block_explorers: BTreeMap::new(),
            },
        };
        let preparation = ImportPreparationState {
            payload: Some(payload),
            payload_digest: "test".to_string(),
            wallets: Vec::new(),
        };

        let result = import_prepared(preparation, None).await;

        assert!(matches!(
            result,
            Err(BackupError::Restore(message))
                if message.contains("wallet plans do not match")
        ));
    }

    #[test]
    fn wallet_id_reservation_rejects_existing_keychain_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = hot_metadata("Occupied wallet");
        metadata.id = WalletId::preview_new_random();
        let xpub = bdk_wallet::bitcoin::bip32::Xpub::from_str(
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
        )
        .unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

        let result = WalletRestoreLease::acquire(&metadata);

        assert!(matches!(result, Err(BackupError::WalletIdOccupied(id)) if id == metadata.id));
        assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));
    }

    #[test]
    fn restore_journal_preserves_preexisting_bdk_artifact() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();

        let mut metadata = hot_metadata("Existing BDK artifact");
        metadata.id = WalletId::preview_new_random();
        let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
            .into_iter()
            .find(|path| path.to_string_lossy().ends_with("-wal"))
            .expect("wallet store artifact paths include a WAL path");

        if let Some(parent) = artifact.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        std::fs::write(&artifact, b"pre-existing WAL").unwrap();

        let initial = RestoreArtifactSnapshot {
            metadata: true,
            keychain_items: false,
            bdk_paths: std::collections::HashSet::from([artifact.clone()]),
            ..RestoreArtifactSnapshot::default()
        };
        let mut journal = RestoreMarkerGuard::test_begin_with_snapshot(&metadata, initial);

        assert!(journal.rollback().is_empty());
        assert!(artifact.exists());
        std::fs::remove_file(artifact).unwrap();
    }
}
