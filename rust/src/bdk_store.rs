use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use bdk_file_store::Store as FileStore;
use bdk_wallet::{KeychainKind, Wallet};
use bitcoin::Network;
use eyre::{Context as _, ContextCompat as _, Result};
use tracing::{info, warn};

use crate::{
    app::reconcile::{AppStateReconcileMessage, Updater},
    database::Database,
    wallet::metadata::{StoreType, WalletId},
};
use cove_common::consts::ROOT_DATA_DIR;

pub struct BdkStore {
    id: WalletId,
    network: Network,
    pub conn: bdk_wallet::rusqlite::Connection,
    storage: BdkStoreStorage,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BdkStoreStorage {
    Persistent,
    InMemory,
}

impl BdkStore {
    /// Open the persistent BDK wallet store for a real wallet
    pub fn try_new(id: &WalletId, network: impl Into<Network>) -> Result<Self> {
        let _operation = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())
            .map_err(|error| eyre::eyre!(error))?;
        Self::try_new_with_persistence_operation(id, network, &_operation)
    }

    /// Open a persistent BDK wallet store while the caller retains its persistence permit
    pub(crate) fn try_new_with_persistence_operation(
        id: &WalletId,
        network: impl Into<Network>,
        _operation: &crate::wallet_lifecycle::WalletPersistenceOperation,
    ) -> Result<Self> {
        crate::bootstrap::ensure_storage_bootstrapped()
            .map_err(|e| eyre::eyre!("storage bootstrap failed: {e}"))?;

        let sqlite_data_path = sqlite_data_path(id);
        let conn = open_persistent_connection(&sqlite_data_path)?;

        let mut me = Self {
            id: id.clone(),
            network: network.into(),
            conn,
            storage: BdkStoreStorage::Persistent,
        };

        if let Err(e) = me.check_and_migrate_from_file_store() {
            tracing::error!("{id} failed to migrate from file store: {e:?}");
            return Err(e);
        }

        Ok(me)
    }

    /// Open an in-memory BDK wallet store for ephemeral wallet views
    pub fn in_memory(id: &WalletId, network: impl Into<Network>) -> Result<Self> {
        let network = network.into();
        let conn = bdk_wallet::rusqlite::Connection::open_in_memory()
            .context("unable to open in-memory rusqlite connection")?;

        Ok(Self { id: id.clone(), network, conn, storage: BdkStoreStorage::InMemory })
    }

    pub(crate) const fn is_in_memory(&self) -> bool {
        matches!(self.storage, BdkStoreStorage::InMemory)
    }

    // check if we have a file store
    // if we do, migrate to the new SQLite store
    fn check_and_migrate_from_file_store(&mut self) -> Result<bool> {
        let id = &self.id;
        let network = self.network;

        if !file_store_data_path(id).exists() {
            return Ok(false);
        }

        // get the metadata for the wallet
        let mode = Database::global().global_config().wallet_mode();
        let cove_network =
            cove_types::Network::try_from(self.network).map_err(|e| eyre::eyre!(e))?;

        let Some(mut metadata) = Database::global()
            .wallets()
            .get(id, cove_network, mode)
            .context("unable to get metadata for wallet")?
        else {
            // if not metdata found this is a new wallet so we can just return
            return Ok(false);
        };

        if metadata.internal.store_type == StoreType::Sqlite {
            return Ok(false);
        }

        warn!("{id} migrating wallet from file store");
        let (mut file_store_db, _changeset) = FileStore::<bdk_wallet::ChangeSet>::load(
            id.to_string().as_bytes(),
            file_store_data_path(id),
        )
        .context("failed to open file store")?;

        let file_store_wallet = Wallet::load()
            .load_wallet(&mut file_store_db)
            .context("failed to load wallet")?
            .context("no wallet found")?;

        let external_descriptor = file_store_wallet.public_descriptor(KeychainKind::External);
        let change_descriptor = file_store_wallet.public_descriptor(KeychainKind::Internal);

        let mut persisted_wallet =
            Wallet::create(external_descriptor.clone(), change_descriptor.clone())
                .network(network)
                .create_wallet(&mut self.conn)
                .context("failed to create wallet")?;

        persisted_wallet.persist(&mut self.conn).context("failed to persist wallet")?;

        // reset metadata scanning state to default so we force a full scan
        metadata.internal.last_scan_finished = None;
        metadata.internal.last_height_fetched = None;
        metadata.internal.performed_full_scan_at = None;
        metadata.internal.store_type = StoreType::Sqlite;

        Database::global()
            .wallets()
            .replace_wallet_metadata(metadata)
            .context("unable to save updated metadata")?;

        Updater::send_update(AppStateReconcileMessage::DatabaseUpdated);

        std::fs::remove_file(file_store_data_path(id)).context("unable to delete filestore")?;
        info!("completed migrating from file store to sqlite store");

        Ok(true)
    }

    pub fn delete_wallet_stores(wallet_id: &WalletId) -> Result<()> {
        Self::delete_address_switch_store(wallet_id)?;
        Self::delete_wallet_store_artifacts(wallet_id)
    }

    pub(crate) fn delete_address_switch_store(wallet_id: &WalletId) -> Result<()> {
        let sqlite_data_path = sqlite_data_path(wallet_id);
        let replacement_path = replacement_store_path(&sqlite_data_path);
        Self::remove_wallet_artifact(&replacement_path)
            .context("unable to delete replacement store")?;
        remove_sqlite_auxiliary_files(&replacement_path)
            .context("unable to delete replacement sqlite auxiliary files")
    }

    pub(crate) fn delete_wallet_store_artifacts(wallet_id: &WalletId) -> Result<()> {
        let file_store_data_path = file_store_data_path(wallet_id);
        let sqlite_data_path = sqlite_data_path(wallet_id);

        Self::remove_wallet_artifact(&file_store_data_path)
            .context("unable to delete filestore")?;
        Self::remove_wallet_artifact(&sqlite_data_path).context("unable to delete sqlite store")?;
        remove_sqlite_auxiliary_files(&sqlite_data_path)
            .context("unable to delete sqlite auxiliary files")?;

        Ok(())
    }

    /// Build a replacement persistent store at a temporary path
    ///
    /// `create` receives the temporary connection and must create the new BDK
    /// wallet in it. The live store is untouched until
    /// [`PreparedStoreReplacement::activate`] publishes the file atomically
    pub(crate) fn prepare_replacement(
        id: &WalletId,
        create: impl FnOnce(&mut bdk_wallet::rusqlite::Connection) -> Result<()>,
    ) -> Result<PreparedStoreReplacement> {
        let _operation = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())
            .map_err(|error| eyre::eyre!(error))?;

        crate::bootstrap::ensure_storage_bootstrapped()
            .map_err(|e| eyre::eyre!("storage bootstrap failed: {e}"))?;

        let final_path = sqlite_data_path(id);
        let temporary_path = replacement_store_path(&final_path);
        Self::remove_wallet_artifact(&temporary_path)
            .context("unable to remove stale replacement store")?;

        remove_sqlite_auxiliary_files(&temporary_path)
            .context("unable to remove stale replacement sqlite auxiliary files")?;

        let prepared = PreparedStoreReplacement {
            temporary_path,
            final_path,
            legacy_store_path: file_store_data_path(id),
        };

        let build = || -> Result<()> {
            let mut conn = open_persistent_connection(&prepared.temporary_path)?;
            create(&mut conn)?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .context("unable to checkpoint replacement store")?;

            drop(conn);
            sync_path(&prepared.temporary_path).context("unable to sync replacement store")?;
            Ok(())
        };

        if let Err(error) = build() {
            prepared.discard();
            return Err(error);
        }

        Ok(prepared)
    }

    pub(crate) fn wallet_store_artifact_paths(wallet_id: &WalletId) -> [PathBuf; 5] {
        let sqlite_data_path = sqlite_data_path(wallet_id);

        [
            file_store_data_path(wallet_id),
            sqlite_data_path.clone(),
            sqlite_auxiliary_path(&sqlite_data_path, "wal"),
            sqlite_auxiliary_path(&sqlite_data_path, "shm"),
            sqlite_data_path.with_extension("db-journal"),
        ]
    }

    /// Remove one restore-owned BDK artifact without touching other wallets
    pub(crate) fn remove_wallet_artifact(path: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// A fully built replacement store waiting to be published over the live one
pub(crate) struct PreparedStoreReplacement {
    temporary_path: PathBuf,
    final_path: PathBuf,
    legacy_store_path: PathBuf,
}

/// Result of publishing a prepared wallet store
pub(crate) enum StoreReplacementActivation {
    /// The atomic rename did not occur and the old store remains authoritative
    NotPublished(eyre::Report),
    /// The rename occurred; a later durability sync may still have failed
    Published { durability_error: Option<eyre::Report> },
}

impl PreparedStoreReplacement {
    /// Atomically publish the replacement over the previous store
    ///
    /// The caller must close the previous store's connection first so its WAL
    /// is checkpointed. A crash before the rename leaves the old store; a crash
    /// after it leaves the new one — never a missing or partial store
    pub(crate) fn activate(self) -> StoreReplacementActivation {
        self.activate_with_sync(sync_path)
    }

    fn activate_with_sync(
        self,
        mut sync: impl FnMut(&Path) -> std::io::Result<()>,
    ) -> StoreReplacementActivation {
        let publish = || -> Result<()> {
            // a stale legacy filestore would shadow the sqlite store at the next load
            BdkStore::remove_wallet_artifact(&self.legacy_store_path)
                .context("unable to remove legacy wallet filestore")?;

            // aux files of the old database must not pair with the replacement
            remove_sqlite_auxiliary_files(&self.final_path)
                .context("unable to remove old sqlite auxiliary files")?;

            std::fs::rename(&self.temporary_path, &self.final_path)
                .context("unable to publish replacement wallet store")?;
            Ok(())
        };

        if let Err(error) = publish() {
            return StoreReplacementActivation::NotPublished(error);
        }

        let durability_error = sync(&self.final_path)
            .context("unable to sync published wallet store")
            .and_then(|()| {
                if let Some(parent) = self.final_path.parent() {
                    sync(parent).context("unable to sync wallet store directory")?;
                }

                Ok(())
            })
            .err();

        StoreReplacementActivation::Published { durability_error }
    }

    /// Remove the temporary store after a failed preparation
    fn discard(&self) {
        let _ = BdkStore::remove_wallet_artifact(&self.temporary_path);
        let _ = remove_sqlite_auxiliary_files(&self.temporary_path);
    }
}

fn replacement_store_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_os_string();
    name.push(".switch-tmp");
    PathBuf::from(name)
}

fn sync_path(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// Synchronize the directory that owns every BDK wallet artifact
pub(crate) fn sync_wallet_store_directory() -> std::io::Result<()> {
    sync_path(&ROOT_DATA_DIR)
}

fn open_persistent_connection(sqlite_data_path: &Path) -> Result<bdk_wallet::rusqlite::Connection> {
    // detect plaintext before opening so we can skip the encryption key
    // (plaintext DBs remain when migration fails — they still work unencrypted)
    let is_existing_plaintext = sqlite_data_path.exists()
        && crate::database::migration::is_plaintext_sqlite(sqlite_data_path);

    let conn = bdk_wallet::rusqlite::Connection::open(sqlite_data_path)
        .context("unable to open rusqlite connection")?;

    if !is_existing_plaintext {
        let key = crate::database::encrypted_backend::encryption_key()
            .expect("encryption key must be set");
        conn.pragma_update(None, "key", format!("x'{}'", hex::encode(key)))?;
    }

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        conn.pragma_update(None, "fullfsync", 1)?;
    }

    // in pages (4096 bytes) 2000 pages = 8MB
    conn.pragma_update(None, "cache_size", 2000)?;

    Ok(conn)
}

fn remove_sqlite_auxiliary_files(db_path: &Path) -> std::io::Result<()> {
    for suffix in ["wal", "shm", "journal"] {
        let aux_path = sqlite_auxiliary_path(db_path, suffix);
        match std::fs::remove_file(&aux_path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

pub(crate) fn sqlite_auxiliary_path(db_path: &Path, suffix: &str) -> PathBuf {
    if let Some(ext) = db_path.extension() {
        let mut ext_string = ext.to_os_string();
        ext_string.push("-");
        ext_string.push(suffix);
        db_path.with_extension(ext_string)
    } else {
        let mut name = db_path.as_os_str().to_os_string();
        name.push("-");
        name.push(suffix);
        PathBuf::from(name)
    }
}

fn file_store_data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}

fn sqlite_data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_sqlite_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replacement_in(directory: &Path) -> PreparedStoreReplacement {
        PreparedStoreReplacement {
            temporary_path: directory.join("wallet.db.switch-tmp"),
            final_path: directory.join("wallet.db"),
            legacy_store_path: directory.join("legacy.db"),
        }
    }

    #[test]
    fn pre_publish_failure_keeps_the_old_store_authoritative() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let replacement = replacement_in(directory.path());
        std::fs::write(&replacement.final_path, b"old").expect("old store is written");

        let outcome = replacement.activate_with_sync(|_| Ok(()));

        assert!(matches!(outcome, StoreReplacementActivation::NotPublished(_)));
        assert_eq!(std::fs::read(directory.path().join("wallet.db")).unwrap(), b"old");
    }

    #[test]
    fn file_sync_failure_after_rename_reports_published_store() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let replacement = replacement_in(directory.path());
        std::fs::write(&replacement.final_path, b"old").expect("old store is written");
        std::fs::write(&replacement.temporary_path, b"new").expect("new store is written");

        let outcome = replacement
            .activate_with_sync(|_| Err(std::io::Error::other("injected file sync failure")));

        assert!(matches!(
            outcome,
            StoreReplacementActivation::Published { durability_error: Some(_) }
        ));
        assert_eq!(std::fs::read(directory.path().join("wallet.db")).unwrap(), b"new");
    }

    #[test]
    fn parent_sync_failure_after_rename_reports_published_store() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let replacement = replacement_in(directory.path());
        std::fs::write(&replacement.final_path, b"old").expect("old store is written");
        std::fs::write(&replacement.temporary_path, b"new").expect("new store is written");
        let mut sync_count = 0;

        let outcome = replacement.activate_with_sync(|_| {
            sync_count += 1;
            if sync_count == 1 {
                Ok(())
            } else {
                Err(std::io::Error::other("injected parent sync failure"))
            }
        });

        assert!(matches!(
            outcome,
            StoreReplacementActivation::Published { durability_error: Some(_) }
        ));
        assert_eq!(std::fs::read(directory.path().join("wallet.db")).unwrap(), b"new");
    }

    #[test]
    fn sqlite_auxiliary_cleanup_removes_rollback_journal() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = directory.path().join("wallet.db");
        let journal = sqlite_auxiliary_path(&store, "journal");
        std::fs::write(&journal, b"journal").expect("journal is written");

        remove_sqlite_auxiliary_files(&store).expect("auxiliary cleanup succeeds");

        assert!(!journal.exists());
    }

    #[cfg(unix)]
    #[test]
    fn wallet_artifact_cleanup_removes_broken_symbolic_link() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let link = directory.path().join("wallet.db");
        symlink(directory.path().join("missing.db"), &link).expect("symbolic link is created");

        BdkStore::remove_wallet_artifact(&link).expect("broken symbolic link is removed");

        assert!(std::fs::symlink_metadata(link).is_err());
    }
}
