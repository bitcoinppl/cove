use std::{
    collections::{BTreeMap, HashMap, HashSet},
    str::FromStr as _,
    sync::Arc,
};

use bdk_wallet::{bitcoin::bip32::Xpub, descriptor::ExtendedDescriptor};
use bip39::Mnemonic;
use cove_device::keychain::{
    Keychain, KeychainError, WalletSecret as KeychainWalletSecret, WalletXprv,
};
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

/// One keychain item as a restore must treat it
///
/// `Create` owns the value to write, so a planned write cannot run without the
/// value that produced the decision
enum RestoreEntry<T> {
    /// Neither the backup nor this device holds the item
    Absent,
    /// Only the backup holds the item, so the restore writes it
    Create(T),
    /// This device already holds exactly the backup's value, so nothing is written
    PreserveVerified,
}

impl<T> RestoreEntry<T> {
    /// Plan an item for a wallet id the caller already required to be free of local data
    fn create(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Create(value),
            None => Self::Absent,
        }
    }

    /// Run `write` only when the restore must create the item
    fn write<E>(self, write: impl FnOnce(T) -> Result<(), E>) -> Result<(), E> {
        match self {
            Self::Create(value) => write(value),
            Self::Absent | Self::PreserveVerified => Ok(()),
        }
    }
}

/// Why a cloud restore refused to touch the local data a wallet id already owns
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LocalWalletConflict {
    /// Local wallet data exists for this wallet id and differs from the backup
    #[error("local wallet data does not match the backup")]
    Mismatch,

    /// Local wallet data exists for this wallet id but could not be read to compare it
    #[error("local wallet data could not be read")]
    Unreadable,
}

/// A cloud restore failure, keeping local conflicts distinct from every other failure
///
/// A local conflict means the restore wrote nothing and kept local data unchanged,
/// which the reader must be told instead of a generic failure
#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudRestoreError {
    #[error(transparent)]
    LocalConflict(#[from] LocalWalletConflict),

    #[error(transparent)]
    Backup(#[from] BackupError),
}

/// Decide what a restore does with one keychain item
///
/// Local data the backup cannot account for is a conflict: a restore must never
/// write over local data it did not verify
fn plan_entry<T: PartialEq>(
    local: Option<T>,
    incoming: Option<T>,
) -> Result<RestoreEntry<T>, LocalWalletConflict> {
    match (local, incoming) {
        (None, None) => Ok(RestoreEntry::Absent),
        (None, Some(incoming)) => Ok(RestoreEntry::Create(incoming)),
        (Some(local), Some(incoming)) if local == incoming => Ok(RestoreEntry::PreserveVerified),
        _ => Err(LocalWalletConflict::Mismatch),
    }
}

fn public_descriptor_pair(descriptors: &Descriptors) -> (ExtendedDescriptor, ExtendedDescriptor) {
    (
        descriptors.external.extended_descriptor.clone(),
        descriptors.internal.extended_descriptor.clone(),
    )
}

/// The keychain writes a hot wallet restore performs
struct HotWalletWrites {
    /// Descriptors with their key maps
    ///
    /// The BDK wallet is always created because every restore rejects a wallet
    /// id that already owns BDK artifacts
    bdk_descriptors: Descriptors,
    secret: RestoreEntry<KeychainWalletSecret>,
    xpub: RestoreEntry<Xpub>,
    descriptors: RestoreEntry<(ExtendedDescriptor, ExtendedDescriptor)>,
}

impl HotWalletWrites {
    /// Write every item the backup carries
    ///
    /// The file-import callers reach this only after requiring an empty wallet
    /// id or removing the artifacts an approval covered
    fn create_all(prepared: PreparedHotWallet) -> Self {
        let PreparedHotWallet { secret, xpub, descriptors } = prepared;

        Self {
            descriptors: RestoreEntry::Create(public_descriptor_pair(&descriptors)),
            bdk_descriptors: descriptors,
            secret: RestoreEntry::Create(secret),
            xpub: RestoreEntry::Create(xpub),
        }
    }
}

/// The keychain writes a public wallet restore performs
struct PublicWalletWrites {
    /// Descriptors used to create the BDK wallet when the backup carries them
    bdk_descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
    xpub: RestoreEntry<Xpub>,
    descriptors: RestoreEntry<(ExtendedDescriptor, ExtendedDescriptor)>,
    tap_signer_backup: RestoreEntry<Zeroizing<Vec<u8>>>,
}

impl PublicWalletWrites {
    /// Write every item the backup carries
    ///
    /// The file-import callers reach this only after requiring an empty wallet
    /// id or removing the artifacts an approval covered
    fn create_all(prepared: PreparedPublicWallet) -> Self {
        let PreparedPublicWallet { xpub, descriptors, tap_signer_backup, .. } = prepared;

        Self {
            bdk_descriptors: descriptors.clone(),
            xpub: RestoreEntry::create(xpub),
            descriptors: RestoreEntry::create(descriptors),
            tap_signer_backup: RestoreEntry::create(tap_signer_backup.map(Zeroizing::new)),
        }
    }
}

/// The keychain items a wallet id already owns on this device
struct LocalWalletItems {
    secret: Option<KeychainWalletSecret>,
    xpub: Option<Xpub>,
    descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
    tap_signer_backup: Option<Zeroizing<Vec<u8>>>,
}

impl LocalWalletItems {
    /// Read every wallet-owned keychain item for comparison against a backup
    ///
    /// A read failure means local data exists that cannot be compared, so the
    /// restore reports a conflict instead of writing over it
    fn read(id: &WalletId) -> Result<Self, LocalWalletConflict> {
        let keychain = Keychain::global();
        let unreadable = |kind: &str, error: KeychainError| {
            warn!("cloud restore cannot read local {kind} for wallet id={id}: {error}");
            LocalWalletConflict::Unreadable
        };

        Ok(Self {
            secret: keychain
                .get_wallet_secret(id)
                .map_err(|error| unreadable("wallet secret", error))?,
            xpub: keychain.get_wallet_xpub(id).map_err(|error| unreadable("xpub", error))?,
            descriptors: keychain
                .get_public_descriptor(id)
                .map_err(|error| unreadable("descriptors", error))?,
            tap_signer_backup: keychain
                .get_tap_signer_backup(id)
                .map_err(|error| unreadable("TapSigner backup", error))?,
        })
    }
}

/// A cloud restore whose every pre-existing local item was checked against the backup
///
/// The validating constructors are the only way to build this type, so planned
/// writes can never skip the comparison that decided them
struct VerifiedCloudRestorePlan<W> {
    snapshot: RestoreArtifactSnapshot,
    writes: W,
}

/// Capture the local artifacts a wallet id owns and read its keychain items
///
/// Metadata rows, BDK artifacts and occupied wallet-data paths still reserve the
/// wallet id outright: only keychain items can be verified and adopted
fn capture_local_wallet_state(
    metadata: &WalletMetadata,
) -> Result<(RestoreArtifactSnapshot, LocalWalletItems), CloudRestoreError> {
    let wallet_id = &metadata.id;
    let id = ValidatedRestoreWalletId::validate(wallet_id).map_err(CloudRestoreError::Backup)?;

    let snapshot = match RestoreArtifactSnapshot::capture(&id) {
        Ok(snapshot) => snapshot,
        // capture only reads this wallet id, so a keychain failure is local data we cannot compare
        Err(BackupError::Keychain(error)) => {
            warn!(
                "cloud restore cannot snapshot local keychain data for wallet id={wallet_id}: {error}"
            );
            return Err(LocalWalletConflict::Unreadable.into());
        }
        Err(error) => return Err(error.into()),
    };

    if snapshot.metadata || !snapshot.bdk_paths.is_empty() || snapshot.wallet_data_occupied {
        return Err(BackupError::WalletIdOccupied(wallet_id.clone()).into());
    }

    // rollback can only preserve keychain kinds the snapshot fingerprinted, so a
    // wallet id holding items the snapshot could not describe stays untouched
    if snapshot.keychain_items && snapshot.keychain_entries.is_empty() {
        warn!("cloud restore found undescribed local keychain items for wallet id={wallet_id}");
        return Err(LocalWalletConflict::Unreadable.into());
    }

    let items = LocalWalletItems::read(wallet_id)?;

    Ok((snapshot, items))
}

impl VerifiedCloudRestorePlan<HotWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedHotWallet,
    ) -> Result<Self, CloudRestoreError> {
        let (snapshot, local) = capture_local_wallet_state(metadata)?;

        // a hot wallet backup carries no TapSigner backup, so a local one is data we cannot verify
        if local.tap_signer_backup.is_some() {
            return Err(LocalWalletConflict::Mismatch.into());
        }

        let PreparedHotWallet { secret, xpub, descriptors } = prepared;
        let writes = HotWalletWrites {
            secret: plan_entry(local.secret, Some(secret))?,
            xpub: plan_entry(local.xpub, Some(xpub))?,
            descriptors: plan_entry(local.descriptors, Some(public_descriptor_pair(&descriptors)))?,
            bdk_descriptors: descriptors,
        };

        Ok(Self { snapshot, writes })
    }

    /// Run the planned writes under a journal that preserves every adopted item
    fn execute(self, metadata: &WalletMetadata) -> Result<Vec<String>, (BackupError, Vec<String>)> {
        let Self { snapshot, writes } = self;

        with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
            restore_hot_wallet_inner_prepared(metadata, writes)
        })
    }
}

impl VerifiedCloudRestorePlan<PublicWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedPublicWallet,
    ) -> Result<Self, CloudRestoreError> {
        let (snapshot, local) = capture_local_wallet_state(metadata)?;

        // a public wallet backup carries no private key, so a local secret is data we cannot verify
        if local.secret.is_some() {
            return Err(LocalWalletConflict::Mismatch.into());
        }

        let PreparedPublicWallet { xpub, descriptors, tap_signer_backup, .. } = prepared;
        let writes = PublicWalletWrites {
            bdk_descriptors: descriptors.clone(),
            xpub: plan_entry(local.xpub, xpub)?,
            descriptors: plan_entry(local.descriptors, descriptors)?,
            tap_signer_backup: plan_entry(
                local.tap_signer_backup,
                tap_signer_backup.map(Zeroizing::new),
            )?,
        };

        Ok(Self { snapshot, writes })
    }

    /// Run the planned writes under a journal that preserves every adopted item
    fn execute(self, metadata: &WalletMetadata) -> Result<Vec<String>, (BackupError, Vec<String>)> {
        let Self { snapshot, writes } = self;

        with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
            restore_descriptor_wallet_inner_prepared(metadata, writes)
        })
    }
}

fn restore_hot_wallet_prepared_with_context(
    metadata: &WalletMetadata,
    prepared: PreparedHotWallet,
    cleanup: RestoreCleanup<'_>,
) -> Result<Vec<String>, (BackupError, Vec<String>)> {
    let result = with_restore_journal(metadata, cleanup, || {
        restore_hot_wallet_inner_prepared(metadata, HotWalletWrites::create_all(prepared))
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
        restore_descriptor_wallet_inner_prepared(metadata, PublicWalletWrites::create_all(prepared))
    });
    if result.is_ok() {
        schedule_cloud_backup_after_local_commit(metadata);
    }

    result
}

pub(crate) fn restore_cloud_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), (CloudRestoreError, Vec<String>)> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Mnemonic(mnemonic))
}

pub(crate) fn restore_cloud_xpriv_wallet(
    metadata: &WalletMetadata,
    xpriv: WalletXprv,
) -> Result<(), (CloudRestoreError, Vec<String>)> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Xpriv(xpriv))
}

fn restore_cloud_hot_wallet(
    metadata: &WalletMetadata,
    secret: KeychainWalletSecret,
) -> Result<(), (CloudRestoreError, Vec<String>)> {
    let xpub = secret.xpub(metadata.network);
    let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
    let prepared = PreparedHotWallet { secret, xpub, descriptors };
    let plan = VerifiedCloudRestorePlan::<HotWalletWrites>::prepare(metadata, prepared)
        .map_err(|error| (error, Vec::new()))?;

    report_cloud_restore(metadata, plan.execute(metadata))
}

pub(crate) fn restore_cloud_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), (CloudRestoreError, Vec<String>)> {
    let prepared =
        prepare_public_wallet(backup, metadata, matches!(&backup.secret, WalletSecret::Unknown))
            .map_err(|error| (CloudRestoreError::Backup(error), Vec::new()))?;

    let prepared = PreparedPublicWallet {
        tap_signer_backup: match &backup.secret {
            WalletSecret::TapSignerBackup(bytes) => Some(bytes.clone()),
            _ => None,
        },
        ..prepared
    };
    let plan = VerifiedCloudRestorePlan::<PublicWalletWrites>::prepare(metadata, prepared)
        .map_err(|error| (error, Vec::new()))?;

    report_cloud_restore(metadata, plan.execute(metadata))
}

/// Log the cleanup warnings a cloud restore left behind and drop them from the result
fn report_cloud_restore(
    metadata: &WalletMetadata,
    result: Result<Vec<String>, (BackupError, Vec<String>)>,
) -> Result<(), (CloudRestoreError, Vec<String>)> {
    let warnings = match result {
        Ok(warnings) => warnings,
        Err((error, warnings)) => return Err((error.into(), warnings)),
    };

    let name = &metadata.name;
    for warning in warnings {
        warn!("cloud restore cleanup warning for {name}: {warning}");
    }

    Ok(())
}

fn restore_hot_wallet_inner_prepared(
    metadata: &WalletMetadata,
    writes: HotWalletWrites,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;
    let network = metadata.network;
    let HotWalletWrites { bdk_descriptors, secret, xpub, descriptors } = writes;

    let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, network)
        .map_err(|e| BackupError::Restore(format!("BDK store for {name}: {e}")))?;

    // create BDK wallet first — if this fails we haven't touched the keychain yet
    bdk_wallet::Wallet::create(
        bdk_descriptors.external.into_tuple(),
        bdk_descriptors.internal.into_tuple(),
    )
    .network(network.into())
    .create_wallet(&mut store.conn)
    .map_err(|e| BackupError::Restore(format!("BDK wallet for {name}: {e}")))?;

    secret.write(|secret| {
        keychain
            .save_wallet_secret(&metadata.id, secret)
            .map_err(|e| BackupError::Keychain(format!("private key for {name}: {e}")))
    })?;

    xpub.write(|xpub| {
        keychain
            .save_wallet_xpub(&metadata.id, xpub)
            .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))
    })?;

    descriptors.write(|(external, internal)| {
        keychain
            .save_public_descriptor(&metadata.id, external, internal)
            .map_err(|e| BackupError::Keychain(format!("descriptors for {name}: {e}")))
    })?;

    RestoredWalletMetadataStore::new(&db).save(metadata, name)?;

    Ok(())
}

fn restore_descriptor_wallet_inner_prepared(
    metadata: &WalletMetadata,
    writes: PublicWalletWrites,
) -> Result<(), BackupError> {
    let keychain = Keychain::global();
    let db = Database::global();
    let name = &metadata.name;

    let PublicWalletWrites { bdk_descriptors, xpub, descriptors, tap_signer_backup } = writes;

    xpub.write(|xpub| {
        keychain
            .save_wallet_xpub(&metadata.id, xpub)
            .map_err(|e| BackupError::Keychain(format!("xpub for {name}: {e}")))
    })?;

    descriptors.write(|(external, internal)| {
        keychain
            .save_public_descriptor(&metadata.id, external, internal)
            .map_err(|e| BackupError::Keychain(format!("descriptors for {name}: {e}")))
    })?;

    // create the BDK wallet from the backup's descriptors, whether they were written or adopted
    if let Some((external, internal)) = bdk_descriptors {
        let mut store = crate::bdk_store::BdkStore::try_new(&metadata.id, metadata.network)
            .map_err(|e| BackupError::Restore(format!("BDK store for {name}: {e}")))?;

        bdk_wallet::Wallet::create(external, internal)
            .network(metadata.network.into())
            .create_wallet(&mut store.conn)
            .map_err(|e| BackupError::Restore(format!("BDK wallet for {name}: {e}")))?;
    }

    // save tap signer backup inside the cleanup wrapper so failure triggers full rollback
    tap_signer_backup.write(|backup| {
        keychain
            .save_tap_signer_backup(&metadata.id, &backup)
            .map_err(|e| BackupError::Keychain(format!("tap signer backup for {name}: {e}")))
    })?;

    RestoredWalletMetadataStore::new(&db).save(metadata, name)?;

    Ok(())
}

fn import_labels(id: &WalletId, jsonl: &str) -> Result<(), BackupError> {
    let manager = LabelManager::try_new(id.clone()).map_err_str(BackupError::Restore)?;
    manager.import(jsonl).map_err_str(BackupError::Restore)
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
            manager.map_err_str(BackupError::Restore).and_then(|manager| {
                manager.import_without_cloud_backup_dirty(jsonl).map_err_str(BackupError::Restore)
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
        let table = GlobalConfigTable::new(db, &write_txn);
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

    const KEYCHAIN_KEY_SUFFIXES: [&str; 6] = [
        "::wallet_mnemonic",
        "::wallet_mnemonic_encryption_key_and_nonce",
        "::wallet_xpub",
        "::wallet_public_descriptor",
        "::tap_signer_backup",
        "::wallet_tap_signer_encryption_key_and_nonce_key_name",
    ];

    fn init_cloud_restore_test_state() {
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();
    }

    /// The raw stored values, so a rewrite of an adopted item is visible even
    /// when the decrypted value would still compare equal
    fn raw_keychain_entries(id: &WalletId) -> Vec<(String, Option<String>)> {
        let keychain = crate::test_support::shared_mock_keychain();

        KEYCHAIN_KEY_SUFFIXES
            .iter()
            .map(|suffix| {
                let key = format!("{id}{suffix}");
                let value = keychain.get_entry(&key);
                (key, value)
            })
            .collect()
    }

    fn cloud_restore_metadata(name: &str, wallet_type: WalletType) -> WalletMetadata {
        let mut metadata = hot_metadata(name);
        metadata.wallet_type = wallet_type;
        metadata.id = WalletId::preview_new_random();
        metadata
    }

    fn backup_mnemonic() -> Mnemonic {
        Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    fn unrelated_mnemonic() -> Mnemonic {
        Mnemonic::from_str("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong").unwrap()
    }

    fn hot_wallet_items(
        metadata: &WalletMetadata,
        secret: KeychainWalletSecret,
    ) -> (KeychainWalletSecret, Xpub, (ExtendedDescriptor, ExtendedDescriptor)) {
        let xpub = secret.xpub(metadata.network);
        let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);

        (secret, xpub, public_descriptor_pair(&descriptors))
    }

    fn public_wallet_backup(
        metadata: &WalletMetadata,
        xpub: Option<Xpub>,
        descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
        secret: WalletSecret,
    ) -> WalletBackup {
        WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret,
            descriptors: descriptors.map(|(external, internal)| {
                crate::backup::model::DescriptorPair {
                    external: external.to_string(),
                    internal: internal.to_string(),
                }
            }),
            xpub: xpub.map(|xpub| xpub.to_string()),
            labels_jsonl: None,
        }
    }

    fn restored_metadata_exists(metadata: &WalletMetadata) -> bool {
        Database::global()
            .wallets
            .get(&metadata.id, metadata.network, metadata.wallet_mode)
            .unwrap()
            .is_some()
    }

    fn bdk_artifacts_exist(id: &WalletId) -> bool {
        crate::bdk_store::BdkStore::wallet_store_artifact_paths(id)
            .into_iter()
            .any(|path| path.exists())
    }

    #[test]
    fn cloud_restore_adopts_matching_hot_wallet_keychain_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled hot wallet", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (secret, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_mnemonic_wallet(&metadata, mnemonic).expect("adopt matching keychain items");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_xpriv_and_creates_missing_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled xpriv wallet", WalletType::Hot);
        let xpriv = WalletXprv::try_from(
            bdk_wallet::bitcoin::bip32::Xpriv::new_master(
                bdk_wallet::bitcoin::NetworkKind::Main,
                &[7; 32],
            )
            .unwrap(),
        )
        .unwrap();
        let (secret, _, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Xpriv(xpriv.clone()));
        Keychain::global().save_wallet_secret(&metadata.id, secret).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_xpriv_wallet(&metadata, xpriv).expect("adopt matching wallet secret");

        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], before[1]);
        assert!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap().is_some());
        assert!(Keychain::global().get_public_descriptor(&metadata.id).unwrap().is_some());
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_xpub_and_creates_the_missing_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled xpub leftover", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (_, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_mnemonic_wallet(&metadata, mnemonic.clone()).expect("adopt matching xpub");

        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after[2], before[2]);
        assert_eq!(
            Keychain::global().get_wallet_secret(&metadata.id).unwrap(),
            Some(KeychainWalletSecret::Mnemonic(mnemonic))
        );
        assert!(Keychain::global().get_public_descriptor(&metadata.id).unwrap().is_some());
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_public_wallet_keychain_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled watch-only wallet", WalletType::Cold);
        let (_, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external.clone(), internal.clone()).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(
            &metadata,
            Some(xpub),
            Some((external, internal)),
            WalletSecret::None,
        );

        restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("adopt matching public keychain items");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_a_matching_tap_signer_backup() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled TapSigner wallet", WalletType::Cold);
        let (_, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let tap_signer_backup = vec![9u8; 64];
        let keychain = Keychain::global();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external.clone(), internal.clone()).unwrap();
        keychain.save_tap_signer_backup(&metadata.id, &tap_signer_backup).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(
            &metadata,
            Some(xpub),
            Some((external, internal)),
            WalletSecret::TapSignerBackup(tap_signer_backup.clone()),
        );

        restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("adopt matching TapSigner backup");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert_eq!(
            keychain.get_tap_signer_backup(&metadata.id).unwrap().as_deref(),
            Some(&tap_signer_backup)
        );
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_keeps_a_mismatched_wallet_secret_and_writes_nothing() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Mismatched hot wallet", WalletType::Hot);
        let (secret, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(unrelated_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());

        assert!(matches!(
            result,
            Err((CloudRestoreError::LocalConflict(LocalWalletConflict::Mismatch), _))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert!(!bdk_artifacts_exist(&metadata.id));
    }

    #[test]
    fn cloud_restore_keeps_an_unreadable_wallet_secret_and_writes_nothing() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Unreadable hot wallet", WalletType::Hot);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        // an encrypted secret whose cryptor is missing cannot be compared to the backup
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "half-written-ciphertext")]);
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());

        assert!(matches!(
            result,
            Err((CloudRestoreError::LocalConflict(LocalWalletConflict::Unreadable), _))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert!(!bdk_artifacts_exist(&metadata.id));
    }

    #[test]
    fn cloud_restore_keeps_a_local_secret_a_public_backup_cannot_explain() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Unexpected secret", WalletType::Cold);
        let (secret, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(&metadata, Some(xpub), None, WalletSecret::None);

        let result = restore_cloud_descriptor_wallet(&metadata, &backup);

        assert!(matches!(
            result,
            Err((CloudRestoreError::LocalConflict(LocalWalletConflict::Mismatch), _))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_refuses_a_wallet_id_that_already_owns_bdk_artifacts() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Occupied by BDK data", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (_, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
            .into_iter()
            .find(|path| path.to_string_lossy().ends_with("-wal"))
            .expect("wallet store artifact paths include a WAL path");
        if let Some(parent) = artifact.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&artifact, b"pre-existing WAL").unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, mnemonic);

        assert!(matches!(
            result,
            Err((CloudRestoreError::Backup(BackupError::WalletIdOccupied(id)), _))
                if id == metadata.id
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        std::fs::remove_file(artifact).unwrap();
    }
}
