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
        let file_store_data_path = file_store_data_path(wallet_id);
        let sqlite_data_path = sqlite_data_path(wallet_id);

        if file_store_data_path.exists() {
            std::fs::remove_file(file_store_data_path).context("unable to delete filestore")?;
        }

        if sqlite_data_path.exists() {
            std::fs::remove_file(&sqlite_data_path).context("unable to delete sqlite store")?;
        }
        remove_sqlite_auxiliary_files(&sqlite_data_path)
            .context("unable to delete sqlite auxiliary files")?;

        let replacement_path = replacement_store_path(&sqlite_data_path);
        Self::remove_wallet_artifact(&replacement_path)
            .context("unable to delete replacement store")?;
        remove_sqlite_auxiliary_files(&replacement_path)
            .context("unable to delete replacement sqlite auxiliary files")?;

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

impl PreparedStoreReplacement {
    /// Atomically publish the replacement over the previous store
    ///
    /// The caller must close the previous store's connection first so its WAL
    /// is checkpointed. A crash before the rename leaves the old store; a crash
    /// after it leaves the new one — never a missing or partial store
    pub(crate) fn activate(self) -> Result<()> {
        // a stale legacy filestore would shadow the sqlite store at the next load
        BdkStore::remove_wallet_artifact(&self.legacy_store_path)
            .context("unable to remove legacy wallet filestore")?;
        // aux files of the old database must not pair with the replacement
        remove_sqlite_auxiliary_files(&self.final_path)
            .context("unable to remove old sqlite auxiliary files")?;

        std::fs::rename(&self.temporary_path, &self.final_path)
            .context("unable to publish replacement wallet store")?;
        sync_path(&self.final_path).context("unable to sync published wallet store")?;
        if let Some(parent) = self.final_path.parent() {
            sync_path(parent).context("unable to sync wallet store directory")?;
        }

        Ok(())
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
    for suffix in ["wal", "shm"] {
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
