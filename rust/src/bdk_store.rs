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
}

impl BdkStore {
    pub fn try_new(id: &WalletId, network: impl Into<Network>) -> Result<Self> {
        crate::bootstrap::ensure_storage_bootstrapped()
            .map_err(|e| eyre::eyre!("storage bootstrap failed: {e}"))?;

        let sqlite_data_path = sqlite_data_path(id);

        // detect plaintext before opening so we can skip the encryption key
        // (plaintext DBs remain when migration fails — they still work unencrypted)
        let is_existing_plaintext = sqlite_data_path.exists()
            && crate::database::migration::is_plaintext_sqlite(&sqlite_data_path);

        let key = if is_existing_plaintext {
            None
        } else {
            Some(
                crate::database::encrypted_backend::encryption_key()
                    .expect("encryption key must be set"),
            )
        };

        let conn = open_store_connection(&sqlite_data_path, key)?;

        let mut me = Self { id: id.clone(), network: network.into(), conn };

        if let Err(e) = me.check_and_migrate_from_file_store() {
            tracing::error!("{id} failed to migrate from file store: {e:?}");
            return Err(e);
        }

        Ok(me)
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
            remove_sqlite_auxiliary_files(&sqlite_data_path);
        }

        Ok(())
    }

    pub fn delete_sqlite_store(wallet_id: &WalletId) -> Result<()> {
        let sqlite_data_path = sqlite_data_path(wallet_id);

        if sqlite_data_path.exists() {
            std::fs::remove_file(&sqlite_data_path).context("unable to delete sqlite store")?;
        }

        remove_sqlite_auxiliary_files(&sqlite_data_path);

        Ok(())
    }
}

/// Open a wallet store connection: apply the SQLCipher key when the database is
/// encrypted, then the pragmas every wallet connection needs
///
/// `key` is `None` only for databases still in plaintext because their migration failed
fn open_store_connection(
    path: &Path,
    key: Option<[u8; 32]>,
) -> Result<bdk_wallet::rusqlite::Connection> {
    let conn = bdk_wallet::rusqlite::Connection::open(path)
        .context("unable to open rusqlite connection")?;

    if let Some(key) = key {
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

fn remove_sqlite_auxiliary_files(db_path: &Path) {
    for suffix in ["wal", "shm"] {
        let aux_path = sqlite_auxiliary_path(db_path, suffix);
        match std::fs::remove_file(&aux_path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => warn!("unable to delete sqlite {suffix} file {}: {e}", aux_path.display()),
        }
    }
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
    use bdk_wallet::rusqlite::Connection;
    use tempfile::TempDir;

    /// A wallet database written by the pre-upgrade dependency stack
    /// (rusqlite 0.31 / libsqlite3-sys 0.28 / SQLCipher 4.5.3), generated at merge-base
    /// 66674a53 through the same open sequence as `open_store_connection`
    const PRE_UPGRADE_DB: &[u8] = include_bytes!("../test_data/sqlcipher_v4_pre_upgrade.db");

    /// The key the fixture was encrypted with, not a key any real wallet uses
    const PRE_UPGRADE_KEY: [u8; 32] = [0x42; 32];

    fn pre_upgrade_db(dir: &TempDir) -> PathBuf {
        let path = dir.path().join("bdk_wallet_sqlite_pre_upgrade.db");
        std::fs::write(&path, PRE_UPGRADE_DB).unwrap();
        path
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
        let names = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        names.map(Result::unwrap).collect()
    }

    /// Upgrading SQLCipher must not strand existing users on a wallet database
    /// their new binary cannot decrypt
    #[test]
    fn opens_wallet_db_written_by_previous_sqlcipher() {
        let dir = TempDir::new().unwrap();
        let path = pre_upgrade_db(&dir);

        let mut conn = open_store_connection(&path, Some(PRE_UPGRADE_KEY))
            .expect("pre-upgrade wallet database must open with the current SQLCipher");

        let tables = table_names(&conn);
        assert!(
            tables.iter().any(|name| name.starts_with("bdk_")),
            "expected bdk tables, found {tables:?}"
        );

        // the wallet itself must still load, not just the raw pages decrypt
        let mut wallet = bdk_wallet::Wallet::load()
            .load_wallet(&mut conn)
            .expect("failed to load persisted wallet")
            .expect("no wallet found in pre-upgrade database");

        assert_eq!(wallet.network(), Network::Bitcoin);

        // writes must land too, an upgraded wallet keeps persisting scan results
        let revealed = wallet.reveal_addresses_to(KeychainKind::External, 20).count();
        assert!(revealed > 0, "fixture already revealed every address, nothing to persist");
        assert!(
            wallet.persist(&mut conn).expect("failed to persist into pre-upgrade database"),
            "expected the revealed addresses to be written back"
        );

        drop(conn);

        let mut reopened = open_store_connection(&path, Some(PRE_UPGRADE_KEY)).unwrap();
        let reloaded = bdk_wallet::Wallet::load()
            .load_wallet(&mut reopened)
            .expect("failed to reload after writing")
            .expect("no wallet found after writing");

        assert_eq!(
            reloaded.derivation_index(KeychainKind::External),
            wallet.derivation_index(KeychainKind::External)
        );
    }

    /// Guards the test above from passing vacuously on a file that turned out to be plaintext
    #[test]
    fn wrong_key_cannot_open_wallet_db_written_by_previous_sqlcipher() {
        let dir = TempDir::new().unwrap();
        let path = pre_upgrade_db(&dir);

        let error = open_store_connection(&path, Some([0x43; 32]))
            .map(|conn| table_names(&conn))
            .expect_err("wrong key must not decrypt the wallet database");

        assert!(
            error.to_string().contains("not a database"),
            "expected a decryption failure, got {error}"
        );
    }
}
