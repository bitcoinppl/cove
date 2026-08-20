use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use bdk_wallet::KeychainKind;
use bdk_wallet::chain::spk_client::FullScanRequest;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_common::consts::{GAP_LIMIT, ROOT_DATA_DIR};
use cove_types::address::AddressInfoWithDerivation;
use cove_util::result_ext::ResultExt as _;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    bdk_store::BdkStore, keychain::Keychain, keys::Descriptors, wallet_secret::WalletSecretExt as _,
};

use super::{
    AddressInfo, AddressTypeSwitchMetadata, Wallet, WalletAddressType, WalletError, WalletStorage,
    fingerprint::Fingerprint,
    metadata::{DiscoveryState, WalletId, WalletMetadata, WalletMode},
};

static NEXT_SWITCH_BACKUP_ID: AtomicU64 = AtomicU64::new(0);
const ADDRESS_SWITCH_DIRECTORY: &str = ".cove-address-switch";
const ADDRESS_SWITCH_JOURNAL: &str = "journal.json";
const ADDRESS_SWITCH_SQLITE_BACKUP: &str = "sqlite-backup.db";
const ADDRESS_SWITCH_FILE_STORE_BACKUP: &str = "file-store-backup.db";
const ADDRESS_SWITCH_JOURNAL_VERSION: u8 = 1;

/// Owns the old wallet state until an address-type switch commits
pub(crate) struct AddressTypeSwitch {
    metadata: AddressTypeSwitchMetadata,
    old_bdk: Option<bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>>,
    old_storage: Option<WalletStorage>,
    journal: Option<AddressSwitchJournal>,
}

impl AddressTypeSwitch {
    fn new(
        metadata: AddressTypeSwitchMetadata,
        old_bdk: bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>,
        old_storage: Option<WalletStorage>,
        journal: Option<AddressSwitchJournal>,
    ) -> Self {
        Self { metadata, old_bdk: Some(old_bdk), old_storage, journal }
    }

    pub(crate) fn metadata(&self) -> AddressTypeSwitchMetadata {
        self.metadata.clone()
    }

    fn mark_replacing(&mut self) -> Result<(), WalletError> {
        self.set_phase(AddressSwitchPhase::Replacing)
    }

    pub(crate) fn mark_metadata_committing(&mut self) -> Result<(), WalletError> {
        self.set_phase(AddressSwitchPhase::MetadataCommitting)
    }

    pub(crate) fn mark_metadata_committed(&mut self) -> Result<(), WalletError> {
        self.set_phase(AddressSwitchPhase::MetadataCommitted)
    }

    pub(crate) fn begin_rollback(&mut self) -> Result<(), WalletError> {
        self.set_phase(AddressSwitchPhase::Rollback)
    }

    fn set_phase(&mut self, phase: AddressSwitchPhase) -> Result<(), WalletError> {
        if let Some(journal) = self.journal.as_mut() {
            journal.set_phase(phase)?;
        }

        Ok(())
    }

    pub(crate) fn commit(mut self) -> Result<(), WalletError> {
        // persist the commit record before dropping the only durable backup
        if self.journal.is_some() {
            self.set_phase(AddressSwitchPhase::Committed)?;
        }

        self.old_bdk.take();
        self.old_storage.take();

        if let Some(journal) = self.journal.take()
            && let Err(error) = journal.cleanup()
        {
            // a committed transaction is safe to retry during the next bootstrap
            warn!(%error, "address switch cleanup is pending");
        }

        Ok(())
    }

    pub(crate) fn rollback(mut self, wallet: &mut Wallet) -> Result<(), WalletError> {
        let phase_error = self.begin_rollback().err();
        let store_error = self.restore_store(wallet).err();

        if phase_error.is_some() || store_error.is_some() {
            return Err(combine_switch_errors(phase_error, store_error));
        }

        self.complete_rollback()
    }

    pub(crate) fn restore_store(&mut self, wallet: &mut Wallet) -> Result<(), WalletError> {
        let old_bdk = self.old_bdk.take().ok_or_else(|| {
            WalletError::PersistError("address-type switch has already been finalized".to_string())
        })?;

        let new_bdk = std::mem::replace(&mut wallet.bdk, old_bdk);
        drop(new_bdk);

        if let Some(old_storage) = self.old_storage.take() {
            let new_storage = std::mem::replace(&mut wallet.storage, old_storage);
            drop(new_storage);

            return Ok(());
        }

        let journal = self.journal.as_ref().ok_or_else(|| {
            WalletError::PersistError("address-type switch backup is missing".to_string())
        })?;

        // close the replacement connection before restoring files at its path
        let temporary_storage = temporary_storage(&wallet.id, wallet.network)?;
        let replacement_storage = std::mem::replace(&mut wallet.storage, temporary_storage);
        drop(replacement_storage);

        journal.backup.restore()?;
        let restored_store =
            BdkStore::try_new(&wallet.id, wallet.network).map_err_str(WalletError::LoadError)?;
        wallet.storage = WalletStorage::persistent(restored_store.conn);

        Ok(())
    }

    pub(crate) fn complete_rollback(mut self) -> Result<(), WalletError> {
        self.set_phase(AddressSwitchPhase::RolledBack)?;
        if let Some(journal) = self.journal.take() {
            journal.cleanup()?;
        }

        Ok(())
    }
}

fn combine_switch_errors(first: Option<WalletError>, second: Option<WalletError>) -> WalletError {
    match (first, second) {
        (Some(first), Some(second)) => WalletError::PersistError(format!("{first}; {second}")),
        (Some(error), None) | (None, Some(error)) => error,
        (None, None) => WalletError::PersistError("address-type switch failed".to_string()),
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AddressSwitchPhase {
    Prepared,
    Replacing,
    StoreReplaced,
    MetadataCommitting,
    MetadataCommitted,
    Rollback,
    RolledBack,
    Committed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AddressSwitchTarget {
    address_type: WalletAddressType,
    discovery_state: DiscoveryState,
    origin: Option<String>,
    master_fingerprint: Option<Fingerprint>,
    external_descriptor: String,
    internal_descriptor: String,
}

impl AddressSwitchTarget {
    fn from_descriptors(
        descriptors: &Descriptors,
        address_type: WalletAddressType,
        previous_metadata: &WalletMetadata,
    ) -> Self {
        let switch_metadata = address_type_switch_metadata_from_descriptors(descriptors);

        Self {
            address_type,
            discovery_state: DiscoveryState::ChoseAdressType,
            origin: switch_metadata.origin.or_else(|| previous_metadata.origin.clone()),
            master_fingerprint: switch_metadata
                .master_fingerprint
                .map(|fingerprint| *fingerprint.as_ref())
                .or_else(|| previous_metadata.master_fingerprint.as_deref().copied()),
            external_descriptor: descriptors.external.extended_descriptor.to_string(),
            internal_descriptor: descriptors.internal.extended_descriptor.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedAddressSwitch {
    version: u8,
    wallet_id: WalletId,
    network: cove_types::Network,
    wallet_mode: WalletMode,
    phase: AddressSwitchPhase,
    old_metadata: WalletMetadata,
    target: AddressSwitchTarget,
}

#[derive(Debug)]
struct AddressSwitchJournal {
    path: PathBuf,
    backup: WalletStoreBackup,
    record: PersistedAddressSwitch,
}

impl AddressSwitchJournal {
    fn create(
        wallet_id: &WalletId,
        network: cove_types::Network,
        old_metadata: WalletMetadata,
        target: AddressSwitchTarget,
        storage: &WalletStorage,
    ) -> Result<Self, WalletError> {
        let directory = new_switch_directory(wallet_id)?;
        let backup = match WalletStoreBackup::create(directory.clone(), wallet_id, storage) {
            Ok(backup) => backup,
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error);
            }
        };
        let path = directory.join(ADDRESS_SWITCH_JOURNAL);
        let record = PersistedAddressSwitch {
            version: ADDRESS_SWITCH_JOURNAL_VERSION,
            wallet_id: wallet_id.clone(),
            network,
            wallet_mode: old_metadata.wallet_mode,
            phase: AddressSwitchPhase::Prepared,
            old_metadata,
            target,
        };
        let journal = Self { path, backup, record };

        if let Err(error) = journal.persist() {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }

        Ok(journal)
    }

    fn from_path(path: PathBuf, record: PersistedAddressSwitch) -> Result<Self, String> {
        let directory = path
            .parent()
            .ok_or_else(|| format!("address switch journal has no parent: {}", path.display()))?
            .to_path_buf();
        let backup = WalletStoreBackup::from_directory(directory, &record.wallet_id)?;

        Ok(Self { path, backup, record })
    }

    fn set_phase(&mut self, phase: AddressSwitchPhase) -> Result<(), WalletError> {
        self.record.phase = phase;
        self.persist()
    }

    fn persist(&self) -> Result<(), WalletError> {
        let bytes = serde_json::to_vec(&self.record).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to serialize address switch journal: {error}"
            ))
        })?;
        let temporary_path = self.path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary_path)
            .map_err(|error| {
                WalletError::PersistError(format!(
                    "failed to create address switch journal: {error}"
                ))
            })?;
        file.write_all(&bytes).map_err(|error| {
            WalletError::PersistError(format!("failed to write address switch journal: {error}"))
        })?;
        file.sync_all().map_err(|error| {
            WalletError::PersistError(format!("failed to sync address switch journal: {error}"))
        })?;
        fs::rename(&temporary_path, &self.path).map_err(|error| {
            WalletError::PersistError(format!("failed to publish address switch journal: {error}"))
        })?;
        sync_directory(self.path.parent().expect("journal has a parent")).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync address switch journal directory: {error}"
            ))
        })?;

        Ok(())
    }

    fn cleanup(self) -> Result<(), WalletError> {
        match fs::remove_dir_all(self.path.parent().expect("journal has a parent")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(WalletError::PersistError(format!(
                    "failed to remove address switch journal and backup: {error}"
                )));
            }
        }

        sync_directory(&switch_directory_root()).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync address switch directory after cleanup: {error}"
            ))
        })
    }
}

#[derive(Debug)]
struct WalletStoreBackup {
    paths: [PathBuf; 5],
    sqlite_backup: PathBuf,
    artifacts: Vec<(PathBuf, PathBuf)>,
}

impl WalletStoreBackup {
    fn create(
        directory: PathBuf,
        wallet_id: &super::metadata::WalletId,
        storage: &WalletStorage,
    ) -> Result<Self, WalletError> {
        let paths = BdkStore::wallet_store_artifact_paths(wallet_id);
        let sqlite_backup = directory.join(ADDRESS_SWITCH_SQLITE_BACKUP);
        let sqlite_backup_string = sqlite_backup.to_string_lossy().into_owned();

        if let Err(error) =
            storage.connection().lock().execute("VACUUM INTO ?1", [&sqlite_backup_string])
        {
            let _ = fs::remove_dir_all(&directory);
            return Err(WalletError::PersistError(format!(
                "failed to back up address switch sqlite store: {error}"
            )));
        }
        sync_file(&sqlite_backup).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync address switch sqlite backup: {error}"
            ))
        })?;

        let mut artifacts = Vec::new();
        // preserve the legacy filestore separately because it is not included in the sqlite snapshot
        for path in paths.iter().take(1) {
            if !path.exists() {
                continue;
            }

            let backup_path = directory.join(ADDRESS_SWITCH_FILE_STORE_BACKUP);
            if let Err(error) = fs::copy(path, &backup_path) {
                let _ = fs::remove_dir_all(&directory);
                return Err(WalletError::PersistError(format!(
                    "failed to back up address switch store: {error}"
                )));
            }
            sync_file(&backup_path).map_err(|error| {
                WalletError::PersistError(format!("failed to sync address switch backup: {error}"))
            })?;

            artifacts.push((path.clone(), backup_path));
        }
        sync_directory(&directory).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync address switch backup directory: {error}"
            ))
        })?;

        Ok(Self { paths, sqlite_backup, artifacts })
    }

    fn from_directory(directory: PathBuf, wallet_id: &WalletId) -> Result<Self, String> {
        let paths = BdkStore::wallet_store_artifact_paths(wallet_id);
        let sqlite_backup = directory.join(ADDRESS_SWITCH_SQLITE_BACKUP);
        if !sqlite_backup.is_file() {
            return Err(format!(
                "address switch sqlite backup is missing: {}",
                sqlite_backup.display()
            ));
        }

        let file_store_backup = directory.join(ADDRESS_SWITCH_FILE_STORE_BACKUP);
        let artifacts = if file_store_backup.exists() {
            vec![(paths[0].clone(), file_store_backup)]
        } else {
            Vec::new()
        };

        Ok(Self { paths, sqlite_backup, artifacts })
    }

    fn restore(&self) -> Result<(), WalletError> {
        for path in &self.paths {
            BdkStore::remove_wallet_artifact(path).map_err(|error| {
                WalletError::PersistError(format!(
                    "failed to remove replacement wallet store: {error}"
                ))
            })?;
        }

        fs::copy(&self.sqlite_backup, &self.paths[1]).map_err(|error| {
            WalletError::PersistError(format!("failed to restore wallet sqlite store: {error}"))
        })?;
        sync_file(&self.paths[1]).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync restored wallet sqlite store: {error}"
            ))
        })?;

        for (path, backup_path) in &self.artifacts {
            fs::copy(backup_path, path).map_err(|error| {
                WalletError::PersistError(format!("failed to restore wallet store: {error}"))
            })?;
            sync_file(path).map_err(|error| {
                WalletError::PersistError(format!("failed to sync restored wallet store: {error}"))
            })?;
        }
        sync_directory(ROOT_DATA_DIR.as_path()).map_err(|error| {
            WalletError::PersistError(format!(
                "failed to sync restored wallet store directory: {error}"
            ))
        })?;

        Ok(())
    }
}

fn new_switch_directory(wallet_id: &WalletId) -> Result<PathBuf, WalletError> {
    let root = switch_directory_root();
    fs::create_dir_all(&root).map_err(|error| {
        WalletError::PersistError(format!("failed to create address switch directory: {error}"))
    })?;
    let nonce = NEXT_SWITCH_BACKUP_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let directory = root.join(format!("{wallet_id}-{timestamp}-{nonce}"));
    fs::create_dir(&directory).map_err(|error| {
        WalletError::PersistError(format!("failed to create address switch transaction: {error}"))
    })?;
    sync_directory(&root).map_err(|error| {
        WalletError::PersistError(format!("failed to sync address switch directory: {error}"))
    })?;

    Ok(directory)
}

/// Root directory for durable address-switch journals
pub(crate) fn switch_directory_root() -> PathBuf {
    ROOT_DATA_DIR.join(ADDRESS_SWITCH_DIRECTORY)
}

/// Remove leftover switch journals of a wallet whose on-disk data is being deleted
///
/// A deleted wallet can no longer complete or roll back a switch, so its journals
/// must not survive to the next bootstrap recovery
pub(crate) fn remove_address_switch_journals(wallet_id: &WalletId) -> std::io::Result<()> {
    remove_address_switch_journals_in(&switch_directory_root(), wallet_id)
}

/// Remove and durably invalidate every address-switch journal under `root`
pub(crate) fn remove_all_address_switch_journals_in(root: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(root) {
        Ok(()) => sync_directory(root.parent().expect("switch directory has a parent")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = root.parent().expect("switch directory has a parent");
            match sync_directory(parent) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// Remove and durably invalidate journals owned by `wallet_id` under `root`
pub(crate) fn remove_address_switch_journals_in(
    root: &Path,
    wallet_id: &WalletId,
) -> std::io::Result<()> {
    let directories = wallet_switch_journal_directories_in(root, wallet_id)?;
    if directories.is_empty() {
        return match sync_directory(root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }

    for path in directories {
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    sync_directory(root)
}

fn wallet_switch_journal_directories_in(
    root: &Path,
    wallet_id: &WalletId,
) -> std::io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut directories = Vec::new();

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let belongs_to_wallet = entry
            .file_name()
            .to_str()
            .is_some_and(|name| journal_directory_belongs_to_wallet(name, wallet_id.as_str()));
        if belongs_to_wallet {
            directories.push(entry.path());
        }
    }

    Ok(directories)
}

fn journal_directory_belongs_to_wallet(directory_name: &str, wallet_id: &str) -> bool {
    // journal directories are named {wallet_id}-{timestamp}-{nonce} and wallet
    // ids may themselves contain '-', so the suffix must parse as the two
    // numeric fields before a prefix match is accepted
    directory_name
        .strip_prefix(wallet_id)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.split_once('-'))
        .is_some_and(|(timestamp, nonce)| {
            timestamp.parse::<u128>().is_ok() && nonce.parse::<u64>().is_ok()
        })
}

fn sync_file(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn read_address_switch_journal(path: &Path) -> Result<PersistedAddressSwitch, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!("failed to read address switch journal {}: {error}", path.display())
    })?;
    let record: PersistedAddressSwitch = serde_json::from_slice(&bytes).map_err(|error| {
        format!("failed to parse address switch journal {}: {error}", path.display())
    })?;
    if record.version != ADDRESS_SWITCH_JOURNAL_VERSION {
        return Err(format!(
            "unsupported address switch journal version {} in {}",
            record.version,
            path.display()
        ));
    }

    Ok(record)
}

/// Recover address-type switches before a wallet can be opened
pub(crate) fn recover_address_type_switches(
    wallets: &crate::database::wallet::WalletsTable,
) -> Result<(), String> {
    let root = switch_directory_root();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to read address switch directory: {error}")),
    };

    let mut failures = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!("failed to read address switch entry: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                failures.push(format!(
                    "failed to inspect address switch entry {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if file_type.is_symlink() {
            failures.push(format!("address switch entry is a symbolic link: {}", path.display()));
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }

        let journal_path = path.join(ADDRESS_SWITCH_JOURNAL);
        if !journal_path.exists() {
            // a crash before the first journal publish cannot have replaced the store
            if let Err(error) = fs::remove_dir_all(&path) {
                failures.push(format!(
                    "failed to remove unpublished address switch {}: {error}",
                    path.display()
                ));
            }
            continue;
        }

        if let Err(error) = recover_address_switch_journal(&journal_path, wallets) {
            failures.push(format!("{}: {error}", journal_path.display()));
        }
    }

    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

fn recover_address_switch_journal(
    path: &Path,
    wallets: &crate::database::wallet::WalletsTable,
) -> Result<(), String> {
    let record = read_address_switch_journal(path)?;
    let mut journal = AddressSwitchJournal::from_path(path.to_path_buf(), record.clone())?;

    match record.phase {
        AddressSwitchPhase::Prepared => journal.cleanup().map_err(|error| error.to_string()),
        AddressSwitchPhase::Replacing
        | AddressSwitchPhase::StoreReplaced
        | AddressSwitchPhase::MetadataCommitting
        | AddressSwitchPhase::Rollback => {
            journal.set_phase(AddressSwitchPhase::Rollback).map_err(|error| error.to_string())?;
            restore_metadata(wallets, &record.old_metadata)?;
            journal.backup.restore().map_err(|error| error.to_string())?;
            journal.set_phase(AddressSwitchPhase::RolledBack).map_err(|error| error.to_string())?;
            journal.cleanup().map_err(|error| error.to_string())
        }
        AddressSwitchPhase::MetadataCommitted => {
            let current = wallets
                .get(&record.wallet_id, record.network, record.wallet_mode)
                .map_err(|error| error.to_string())?;
            if let Some(current) = current {
                if metadata_matches_target(&current, &record.target) {
                    match store_matches_target(&record) {
                        Ok(true) => {
                            journal
                                .set_phase(AddressSwitchPhase::Committed)
                                .map_err(|error| error.to_string())?;
                            return journal.cleanup().map_err(|error| error.to_string());
                        }
                        // an uninspectable store is not a mismatch: keep the journal
                        // and retry at the next bootstrap instead of rolling back
                        // metadata the user may already see
                        Err(error) => {
                            return Err(format!(
                                "failed to inspect wallet store after committed address switch {}: {error}",
                                record.wallet_id
                            ));
                        }
                        Ok(false) => {}
                    }
                }

                if current == record.old_metadata
                    || metadata_matches_target(&current, &record.target)
                {
                    journal
                        .set_phase(AddressSwitchPhase::Rollback)
                        .map_err(|error| error.to_string())?;
                    restore_metadata(wallets, &record.old_metadata)?;
                    journal.backup.restore().map_err(|error| error.to_string())?;
                    journal
                        .set_phase(AddressSwitchPhase::RolledBack)
                        .map_err(|error| error.to_string())?;
                    return journal.cleanup().map_err(|error| error.to_string());
                }
            }

            Err(format!(
                "wallet metadata does not match address switch target or previous state: {}",
                record.wallet_id
            ))
        }
        AddressSwitchPhase::RolledBack | AddressSwitchPhase::Committed => {
            journal.cleanup().map_err(|error| error.to_string())
        }
    }
}

fn store_matches_target(record: &PersistedAddressSwitch) -> Result<bool, String> {
    BdkStore::persistent_wallet_matches_descriptors(
        &record.wallet_id,
        &record.target.external_descriptor,
        &record.target.internal_descriptor,
    )
    .map_err(|error| error.to_string())
}

fn metadata_matches_target(metadata: &WalletMetadata, target: &AddressSwitchTarget) -> bool {
    metadata.address_type == target.address_type
        && metadata.discovery_state == target.discovery_state
        && metadata.origin == target.origin
        && metadata.master_fingerprint.as_deref() == target.master_fingerprint.as_ref()
}

fn restore_metadata(
    wallets: &crate::database::wallet::WalletsTable,
    metadata: &WalletMetadata,
) -> Result<(), String> {
    match wallets
        .get(&metadata.id, metadata.network, metadata.wallet_mode)
        .map_err(|error| error.to_string())?
    {
        Some(current) if current == *metadata => Ok(()),
        Some(_) => wallets
            .replace_wallet_metadata(metadata.clone())
            .map(|_| ())
            .map_err(|error| error.to_string()),
        None => wallets
            .save_restored_wallet_metadata(metadata.clone())
            .map_err(|error| error.to_string()),
    }
}

impl Wallet {
    pub(crate) fn start_receive_prioritized_full_scan(&self) -> FullScanRequest<KeychainKind> {
        receive_prioritized_full_scan_request(&self.bdk)
    }

    /// The user imported a hww and wants to switch from native segwit to a different address type
    pub(crate) fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitch, WalletError> {
        debug!("switching public descriptor wallet to new address type: {address_type:?}");

        let descriptors: Descriptors = descriptors.into();
        self.switch_to_descriptors(descriptors, address_type)
    }

    /// The user imported a hot wallet and wants to switch from native segwit to a different address type
    pub(crate) fn switch_private_wallet_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitch, WalletError> {
        debug!("switching private wallet to new address type");

        let secret = Keychain::global()
            .get_wallet_secret(&self.id)
            .ok()
            .flatten()
            .ok_or(WalletError::WalletNotFound)?;

        let descriptors = secret.into_descriptors(self.network, address_type);
        self.switch_to_descriptors(descriptors, address_type)
    }

    fn switch_to_descriptors(
        &mut self,
        descriptors: Descriptors,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitch, WalletError> {
        let is_persistent = self.uses_persistent_storage();
        let (placeholder_bdk, placeholder_storage) =
            in_memory_wallet(&self.id, &descriptors, self.network)?;

        let journal = if is_persistent {
            Some(AddressSwitchJournal::create(
                &self.id,
                self.network,
                self.metadata.clone(),
                AddressSwitchTarget::from_descriptors(&descriptors, address_type, &self.metadata),
                &self.storage,
            )?)
        } else {
            None
        };
        let old_bdk = std::mem::replace(&mut self.bdk, placeholder_bdk);
        let old_storage = std::mem::replace(&mut self.storage, placeholder_storage);
        let switch_metadata = address_type_switch_metadata_from_descriptors(&descriptors);
        let mut switch =
            AddressTypeSwitch::new(switch_metadata, old_bdk, Some(old_storage), journal);

        if is_persistent {
            if let Err(error) = switch.mark_replacing() {
                let rollback_error = switch.rollback(self).err();
                return Err(combine_switch_errors(Some(error), rollback_error));
            }

            // release the old SQLite connection before replacing files at its path
            switch.old_storage.take();
        }

        let replacement = if is_persistent {
            replace_persistent_store(&self.id, self.network, &descriptors)
        } else {
            in_memory_wallet(&self.id, &descriptors, self.network)
        };

        match replacement {
            Ok((bdk, storage)) => {
                self.bdk = bdk;
                self.storage = storage;
                if let Err(error) = switch.set_phase(AddressSwitchPhase::StoreReplaced) {
                    if let Err(rollback_error) = switch.rollback(self) {
                        return Err(WalletError::PersistError(format!(
                            "{error}; failed to restore previous wallet store: {rollback_error}"
                        )));
                    }

                    return Err(error);
                }

                Ok(switch)
            }
            Err(error) => {
                if let Err(rollback_error) = switch.rollback(self) {
                    return Err(WalletError::PersistError(format!(
                        "{error}; failed to restore previous wallet store: {rollback_error}"
                    )));
                }

                Err(error)
            }
        }
    }

    pub fn get_next_address(&mut self) -> Result<AddressInfoWithDerivation, WalletError> {
        const MAX_ADDRESSES: usize = (GAP_LIMIT - 5) as usize;

        let addresses: Vec<AddressInfo> = self
            .bdk
            .list_unused_addresses(KeychainKind::External)
            .take(MAX_ADDRESSES)
            .map(Into::into)
            .collect();

        // get up to 25 revealed but unused addresses
        if addresses.len() < MAX_ADDRESSES {
            let address_info =
                AddressInfo::from(self.bdk.reveal_next_address(KeychainKind::External));

            self.persist()?;

            let derivation_path =
                self.bdk.public_descriptor(KeychainKind::External).derivation_path().ok();
            let info = AddressInfoWithDerivation::new(address_info, derivation_path);
            return Ok(info);
        }

        // if we have already revealed 25 addresses, we cycle back to the first one
        // and present those addresses, until a next unused address is available, if we don't
        // do this we could hit the gap limit and users might use a an adddress past
        // the gap limit and not be able to see it their wallet
        //
        // note: index to use is the index of the address in the list of addresses, not the derivation index
        let index_to_use =
            if let Some(last_index) = self.metadata.internal.last_seen_address_index(&addresses) {
                (last_index + 1) % MAX_ADDRESSES
            } else {
                0
            };

        let address_info = addresses[index_to_use].clone();
        self.metadata.internal.set_last_seen_address_index(&addresses, index_to_use);

        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();
        let address_info_with_derivation =
            AddressInfoWithDerivation::new(address_info, derivation_path);

        Ok(address_info_with_derivation)
    }

    pub fn receive_address_at_index(&self, index: u32) -> AddressInfoWithDerivation {
        let address_info = AddressInfo::from(self.bdk.peek_address(KeychainKind::External, index));
        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();

        AddressInfoWithDerivation::new(address_info, derivation_path)
    }

    pub fn receive_address_is_unused(&self, index: u32) -> bool {
        self.bdk.list_unused_addresses(KeychainKind::External).any(|address| address.index == index)
    }

    pub fn mark_receive_address_used(&mut self, index: u32) -> Result<(), WalletError> {
        if self.bdk.mark_used(KeychainKind::External, index) {
            self.persist()?;
        }

        Ok(())
    }

    pub fn unreserve_tx_change_addresses(&mut self, tx: &bdk_wallet::bitcoin::Transaction) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                self.bdk.derivation_of_spk(txout.script_pubkey.clone())
            {
                self.bdk.unmark_used(KeychainKind::Internal, index);
            }
        }
    }
}

fn in_memory_wallet(
    id: &super::metadata::WalletId,
    descriptors: &Descriptors,
    network: cove_types::Network,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    let mut store = BdkStore::in_memory(id, network).map_err_str(WalletError::LoadError)?;
    let wallet = descriptors
        .clone()
        .into_create_params()
        .network(network.into())
        .create_wallet(&mut store.conn)
        .map_err_str(WalletError::BdkError)?;

    Ok((wallet, WalletStorage::in_memory(store.conn)))
}

fn temporary_storage(
    id: &super::metadata::WalletId,
    network: cove_types::Network,
) -> Result<WalletStorage, WalletError> {
    let store = BdkStore::in_memory(id, network).map_err_str(WalletError::LoadError)?;
    Ok(WalletStorage::in_memory(store.conn))
}

fn replace_persistent_store(
    id: &super::metadata::WalletId,
    network: cove_types::Network,
    descriptors: &Descriptors,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    for path in BdkStore::wallet_store_artifact_paths(id).into_iter().skip(1) {
        BdkStore::remove_wallet_artifact(&path).map_err(|error| {
            WalletError::PersistError(format!("failed to delete wallet filestore: {error}"))
        })?;
    }

    let mut store = BdkStore::try_new(id, network).map_err_str(WalletError::LoadError)?;
    let wallet = descriptors
        .clone()
        .into_create_params()
        .network(network.into())
        .create_wallet(&mut store.conn)
        .map_err_str(WalletError::BdkError)?;
    store
        .conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .map_err_str(WalletError::PersistError)?;
    sync_file(
        &BdkStore::wallet_store_artifact_paths(id).get(1).cloned().expect("sqlite path exists"),
    )
    .map_err_str(WalletError::PersistError)?;
    sync_directory(ROOT_DATA_DIR.as_path()).map_err_str(WalletError::PersistError)?;

    Ok((wallet, WalletStorage::persistent(store.conn)))
}

/// Builds an incremental scan request that checks revealed-unused receive addresses first
///
/// The request still uses unbounded BDK SPK iterators. The progressive scanner owns stop-gap
/// enforcement, so the normal external iterator resumes from index `0` with prioritized indexes
/// filtered out instead of being capped to the gap limit
fn receive_prioritized_full_scan_request(
    wallet: &bdk_wallet::Wallet,
) -> FullScanRequest<KeychainKind> {
    let mut builder = FullScanRequest::builder().chain_tip(wallet.local_chain().tip());

    let priority_spks = wallet
        .list_unused_addresses(KeychainKind::External)
        .take(GAP_LIMIT as usize)
        .map(|address| (address.index, address.address.script_pubkey()))
        .collect::<Vec<_>>();

    let priority_indices = priority_spks.iter().map(|(index, _)| *index).collect::<Vec<_>>();

    if let Some(external_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::External) {
        let external_spks = priority_spks
            .into_iter()
            .chain(external_spks.filter(move |(index, _)| !priority_indices.contains(index)));

        builder = builder.spks_for_keychain(KeychainKind::External, external_spks);
    }

    if let Some(internal_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::Internal) {
        builder = builder.spks_for_keychain(KeychainKind::Internal, internal_spks);
    }

    builder.build()
}

fn address_type_switch_metadata_from_descriptors(
    descriptors: &Descriptors,
) -> AddressTypeSwitchMetadata {
    AddressTypeSwitchMetadata {
        master_fingerprint: descriptors
            .fingerprint()
            .map(|fingerprint| Arc::new(fingerprint.into())),
        origin: descriptors.origin().ok(),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn wallet_switch_journal_directories(
        wallet_id: &WalletId,
    ) -> std::io::Result<Vec<PathBuf>> {
        wallet_switch_journal_directories_in(&switch_directory_root(), wallet_id)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bdk_wallet::bitcoin::{
        Address as BdkAddress, Amount, BlockHash, Network, hashes::Hash as _,
    };
    use bdk_wallet::chain::{BlockId, ConfirmationBlockTime};
    use bdk_wallet::test_utils::{
        get_funded_wallet_wpkh, get_test_wpkh_and_change_desc, insert_anchor, insert_checkpoint,
        insert_tx,
    };

    use super::*;

    fn test_bdk_wallet() -> bdk_wallet::Wallet {
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();

        bdk_wallet::Wallet::create(external_descriptor, internal_descriptor)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .expect("wallet is created")
    }

    fn scan_indexes(
        request: &mut FullScanRequest<KeychainKind>,
        keychain: KeychainKind,
        count: usize,
    ) -> Vec<u32> {
        request.iter_spks(keychain).take(count).map(|(index, _)| index).collect()
    }

    fn build_tx_with_change(wallet: &mut bdk_wallet::Wallet) -> bdk_wallet::bitcoin::Psbt {
        let address = BdkAddress::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let mut builder = wallet.build_tx();
        builder.add_recipient(address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));
        builder.finish().unwrap()
    }

    fn tx_output_index(
        wallet: &bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
        keychain: KeychainKind,
    ) -> u32 {
        tx.output
            .iter()
            .find_map(|txout| match wallet.derivation_of_spk(txout.script_pubkey.clone()) {
                Some((txout_keychain, index)) if txout_keychain == keychain => Some(index),
                _ => None,
            })
            .unwrap()
    }

    fn unused_addresses_contain(
        wallet: &bdk_wallet::Wallet,
        keychain: KeychainKind,
        index: u32,
    ) -> bool {
        wallet.list_unused_addresses(keychain).any(|address| address.index == index)
    }

    fn unreserve_tx_change_addresses(
        wallet: &mut bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
    ) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                wallet.derivation_of_spk(txout.script_pubkey.clone())
            {
                wallet.unmark_used(KeychainKind::Internal, index);
            }
        }
    }

    #[test]
    fn unreserve_tx_change_addresses_releases_reserved_change_index() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_confirmed_change_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);
        let block_id = BlockId { height: 1, hash: BlockHash::hash(b"confirmed change") };
        let confirmation = ConfirmationBlockTime { block_id, confirmation_time: 1 };

        insert_checkpoint(&mut wallet, block_id);
        insert_tx(&mut wallet, psbt.unsigned_tx.clone());
        insert_anchor(&mut wallet, psbt.unsigned_tx.compute_txid(), confirmation);

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_self_send_receive_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let receive_address = wallet.reveal_next_address(KeychainKind::External);

        assert!(wallet.mark_used(KeychainKind::External, receive_address.index));

        let mut builder = wallet.build_tx();
        builder.add_recipient(receive_address.address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));

        let psbt = builder.finish().unwrap();
        let receive_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::External);

        assert_eq!(receive_address.index, receive_index);
        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));
    }

    #[test]
    fn receive_prioritized_scan_checks_revealed_unused_external_indexes_first() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 7);

        assert_eq!(indexes, vec![1, 3, 4, 0, 2, 5, 6]);
    }

    #[test]
    fn receive_prioritized_scan_deduplicates_priority_indexes_from_normal_external_scan() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 10);
        let unique_indexes = indexes.iter().copied().collect::<std::collections::BTreeSet<_>>();

        assert_eq!(indexes.len(), unique_indexes.len());
    }

    #[test]
    fn receive_prioritized_scan_prefix_is_capped_at_gap_limit() {
        let mut wallet = test_bdk_wallet();
        let gap_limit = u32::from(GAP_LIMIT);
        let _ = wallet.reveal_addresses_to(KeychainKind::External, gap_limit + 2).last();
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, GAP_LIMIT as usize + 2);
        let expected_prefix = (0..gap_limit).collect::<Vec<_>>();

        assert_eq!(&indexes[..GAP_LIMIT as usize], expected_prefix.as_slice());
        assert_eq!(indexes[GAP_LIMIT as usize], gap_limit);
    }

    #[test]
    fn receive_prioritized_scan_prefix_does_not_fill_with_unrevealed_external_indexes() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 4);

        assert_eq!(indexes, vec![1, 2, 0, 3]);
    }

    #[test]
    fn receive_prioritized_scan_keeps_internal_keychain_after_external_keychain() {
        let wallet = test_bdk_wallet();
        let request = receive_prioritized_full_scan_request(&wallet);

        assert_eq!(request.keychains(), vec![KeychainKind::External, KeychainKind::Internal]);
    }

    #[test]
    fn receive_prioritized_scan_construction_does_not_reveal_or_mark_addresses_used() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let last_revealed_before = wallet.spk_index().last_revealed_indices();
        let unused_before = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        let _request = receive_prioritized_full_scan_request(&wallet);

        let last_revealed_after = wallet.spk_index().last_revealed_indices();
        let unused_after = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        assert_eq!(last_revealed_after, last_revealed_before);
        assert_eq!(unused_after, unused_before);
    }
}
