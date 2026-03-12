use std::sync::{Arc, LazyLock};

use flume::{Receiver, Sender};
use parking_lot::RwLock;
use rand::RngExt as _;
use strum::IntoEnumIterator as _;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

use cove_cspp::backup_data::{
    BackupManifest, DescriptorPair, WalletEntry, WalletMode, WalletSecret, wallet_record_id,
};
use cove_cspp::master_key_crypto;
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use cove_types::network::Network;

use crate::database::Database;
use crate::database::global_config::CloudBackup;
use crate::wallet::metadata::WalletType;

const RP_ID: &str = "covebitcoinwallet.com";
const CREDENTIAL_ID_KEY: &str = "cspp::v1::credential_id";
const PRF_SALT_KEY: &str = "cspp::v1::prf_salt";

type Message = CloudBackupReconcileMessage;

pub static CLOUD_BACKUP_MANAGER: LazyLock<Arc<RustCloudBackupManager>> =
    LazyLock::new(RustCloudBackupManager::init);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudBackupState {
    Disabled,
    Enabling,
    Restoring,
    Enabled,
    Error(String),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupReconcileMessage {
    StateChanged(CloudBackupState),
    ProgressUpdated { completed: u32, total: u32 },
    EnableComplete,
    RestoreComplete(CloudBackupRestoreReport),
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudBackupRestoreReport {
    pub wallets_restored: u32,
    pub wallets_failed: u32,
    pub failed_wallet_errors: Vec<String>,
}

#[uniffi::export(callback_interface)]
pub trait CloudBackupManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: CloudBackupReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustCloudBackupManager {
    #[allow(dead_code)]
    pub state: Arc<RwLock<CloudBackupState>>,
    pub reconciler: Sender<Message>,
    pub reconcile_receiver: Arc<Receiver<Message>>,
}

impl RustCloudBackupManager {
    fn init() -> Arc<Self> {
        let (sender, receiver) = flume::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(CloudBackupState::Disabled)),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
        .into()
    }

    fn send(&self, message: Message) {
        if let Message::StateChanged(state) = &message {
            *self.state.write() = state.clone();
        }

        if let Err(e) = self.reconciler.send(message) {
            error!("unable to send cloud backup message: {e:?}");
        }
    }
}

#[uniffi::export]
impl RustCloudBackupManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        CLOUD_BACKUP_MANAGER.clone()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn CloudBackupManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                reconciler.reconcile(field);
            }
        });
    }

    pub fn current_state(&self) -> CloudBackupState {
        self.state.read().clone()
    }

    /// Enable cloud backup — idempotent, safe to retry
    ///
    /// Creates passkey (or reuses existing), encrypts master key + all wallets,
    /// uploads to CloudKit, marks enabled only after manifest upload succeeds
    pub fn enable_cloud_backup(&self) {
        let this = CLOUD_BACKUP_MANAGER.clone();
        std::thread::spawn(move || {
            if let Err(e) = this.do_enable_cloud_backup() {
                error!("Cloud backup enable failed: {e}");
                this.send(Message::StateChanged(CloudBackupState::Error(e.to_string())));
            }
        });
    }

    /// Restore from cloud backup — called after device restore
    ///
    /// Uses discoverable credential assertion (no local keychain state required)
    pub fn restore_from_cloud_backup(&self) {
        let this = CLOUD_BACKUP_MANAGER.clone();
        std::thread::spawn(move || {
            if let Err(e) = this.do_restore_from_cloud_backup() {
                error!("Cloud backup restore failed: {e}");
                this.send(Message::StateChanged(CloudBackupState::Error(e.to_string())));
            }
        });
    }
}

impl RustCloudBackupManager {
    fn do_enable_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.send(Message::StateChanged(CloudBackupState::Enabling));

        let passkey = PasskeyAccess::global();
        if !passkey.is_prf_supported() {
            return Err(CloudBackupError::NotSupported(
                "PRF extension not supported on this device".into(),
            ));
        }

        // get or create local master key
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = cspp
            .get_or_create_master_key()
            .map_err(|e| CloudBackupError::Internal(format!("master key: {e}")))?;

        let keychain = Keychain::global();

        // load or create passkey credentials
        let (credential_id, prf_salt): (Vec<u8>, [u8; 32]) = match (
            keychain.get(CREDENTIAL_ID_KEY.to_string()),
            keychain.get(PRF_SALT_KEY.to_string()),
        ) {
            (Some(cred_hex), Some(salt_hex)) => {
                let cred = hex::decode(&cred_hex)
                    .map_err(|e| CloudBackupError::Internal(format!("credential decode: {e}")))?;
                let salt: [u8; 32] = hex::decode(&salt_hex)
                    .map_err(|e| CloudBackupError::Internal(format!("salt decode: {e}")))?
                    .try_into()
                    .map_err(|_| CloudBackupError::Internal("prf_salt is not 32 bytes".into()))?;
                info!("Reusing existing passkey credentials");
                (cred, salt)
            }
            _ => {
                info!("Creating new passkey");
                let prf_salt: [u8; 32] = rand::rng().random();
                let challenge: Vec<u8> = rand::rng().random::<[u8; 32]>().to_vec();

                let user_id = rand::rng().random::<[u8; 16]>().to_vec();
                let credential_id = passkey
                    .create_passkey(RP_ID.to_string(), user_id, challenge)
                    .map_err(|e| CloudBackupError::Passkey(e.to_string()))?;

                // persist immediately so retries reuse the same passkey
                keychain
                    .save(CREDENTIAL_ID_KEY.to_string(), hex::encode(&credential_id))
                    .map_err(|e| CloudBackupError::Internal(format!("save credential: {e}")))?;
                keychain
                    .save(PRF_SALT_KEY.to_string(), hex::encode(prf_salt))
                    .map_err(|e| CloudBackupError::Internal(format!("save prf_salt: {e}")))?;

                (credential_id, prf_salt)
            }
        };

        // authenticate with PRF to get wrapping key
        let challenge: Vec<u8> = rand::rng().random::<[u8; 32]>().to_vec();
        let prf_output = passkey
            .authenticate_with_prf(RP_ID.to_string(), credential_id, prf_salt.to_vec(), challenge)
            .map_err(|e| CloudBackupError::Passkey(e.to_string()))?;

        let prf_key: [u8; 32] = prf_output
            .try_into()
            .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

        // encrypt and upload master key
        let encrypted_master =
            master_key_crypto::encrypt_master_key(&master_key, &prf_key, &prf_salt)
                .map_err(|e| CloudBackupError::Crypto(e.to_string()))?;
        let master_json = serde_json::to_vec(&encrypted_master)
            .map_err(|e| CloudBackupError::Internal(format!("serialize master: {e}")))?;

        let cloud = CloudStorage::global();
        cloud
            .upload_master_key_backup(master_json)
            .map_err(|e| CloudBackupError::Cloud(e.to_string()))?;

        // enumerate and encrypt all wallets
        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let mut wallet_record_ids = Vec::new();
        let db = Database::global();

        for network in Network::iter() {
            for mode in [
                crate::wallet::metadata::WalletMode::Main,
                crate::wallet::metadata::WalletMode::Decoy,
            ] {
                let wallets = db
                    .wallets
                    .get_all(network, mode)
                    .map_err(|e| CloudBackupError::Internal(format!("list wallets: {e}")))?;

                for metadata in wallets {
                    let entry = build_wallet_entry(&metadata, mode)?;
                    let encrypted = wallet_crypto::encrypt_wallet_entry(&entry, &critical_key)
                        .map_err(|e| CloudBackupError::Crypto(e.to_string()))?;

                    let record_id = wallet_record_id(metadata.id.as_ref());
                    let wallet_json = serde_json::to_vec(&encrypted).map_err(|e| {
                        CloudBackupError::Internal(format!("serialize wallet: {e}"))
                    })?;

                    cloud
                        .upload_wallet_backup(record_id.clone(), wallet_json)
                        .map_err(|e| CloudBackupError::Cloud(e.to_string()))?;

                    wallet_record_ids.push(record_id);
                }
            }
        }

        // upload manifest last as commit marker
        let manifest = BackupManifest {
            version: 1,
            created_at: jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
            wallet_record_ids,
        };
        let manifest_json = serde_json::to_vec(&manifest)
            .map_err(|e| CloudBackupError::Internal(format!("serialize manifest: {e}")))?;
        cloud.upload_manifest(manifest_json).map_err(|e| CloudBackupError::Cloud(e.to_string()))?;

        // mark enabled only after manifest succeeds
        let now = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        db.global_config
            .set_cloud_backup(&CloudBackup::Enabled { last_sync: Some(now) })
            .map_err(|e| CloudBackupError::Internal(format!("persist cloud backup state: {e}")))?;

        self.send(Message::EnableComplete);
        self.send(Message::StateChanged(CloudBackupState::Enabled));
        info!("Cloud backup enabled successfully");
        Ok(())
    }

    fn do_restore_from_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.send(Message::StateChanged(CloudBackupState::Restoring));

        let cloud = CloudStorage::global();
        let passkey = PasskeyAccess::global();

        // download encrypted master key to get prf_salt
        let master_json = cloud
            .download_master_key_backup()
            .map_err(|e| CloudBackupError::Cloud(e.to_string()))?;
        let encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup =
            serde_json::from_slice(&master_json)
                .map_err(|e| CloudBackupError::Internal(format!("deserialize master: {e}")))?;

        if encrypted_master.version != 1 {
            return Err(CloudBackupError::Internal(format!(
                "unsupported master key backup version: {}",
                encrypted_master.version
            )));
        }

        let prf_salt = encrypted_master.prf_salt;

        // discoverable credential assertion — no credential_id needed
        let challenge: Vec<u8> = rand::rng().random::<[u8; 32]>().to_vec();
        let discovered = passkey
            .discover_and_authenticate_with_prf(RP_ID.to_string(), prf_salt.to_vec(), challenge)
            .map_err(|e| CloudBackupError::Passkey(e.to_string()))?;

        let prf_key: [u8; 32] = discovered
            .prf_output
            .try_into()
            .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

        // decrypt master key
        let master_key = master_key_crypto::decrypt_master_key(&encrypted_master, &prf_key)
            .map_err(|e| CloudBackupError::Crypto(format!("master key decrypt: {e}")))?;

        // persist discovered credential to local keychain
        let keychain = Keychain::global();
        keychain
            .save(CREDENTIAL_ID_KEY.to_string(), hex::encode(&discovered.credential_id))
            .map_err(|e| CloudBackupError::Internal(format!("save credential_id: {e}")))?;
        keychain
            .save(PRF_SALT_KEY.to_string(), hex::encode(prf_salt))
            .map_err(|e| CloudBackupError::Internal(format!("save prf_salt: {e}")))?;

        // save master key to keychain and set encryption key
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        cspp.save_master_key(&master_key)
            .map_err(|e| CloudBackupError::Internal(format!("save master key: {e}")))?;

        let sensitive_key = Zeroizing::new(master_key.sensitive_data_key());
        crate::database::encrypted_backend::replace_encryption_key(*sensitive_key);

        // clear stale master key cache so reinitialize loads the correct key
        cove_cspp::reset_master_key_cache();

        // wipe existing encrypted databases (undecryptable with old key)
        wipe_local_data();

        // reinitialize database with the new encryption key
        Database::reinitialize()
            .map_err(|e| CloudBackupError::Internal(format!("reinitialize database: {e}")))?;

        // download manifest
        let manifest_json =
            cloud.download_manifest().map_err(|e| CloudBackupError::Cloud(e.to_string()))?;
        let manifest: BackupManifest = serde_json::from_slice(&manifest_json)
            .map_err(|e| CloudBackupError::Internal(format!("deserialize manifest: {e}")))?;

        if manifest.version != 1 {
            return Err(CloudBackupError::Internal(format!(
                "unsupported manifest version: {}",
                manifest.version
            )));
        }

        let total = manifest.wallet_record_ids.len() as u32;
        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let mut report = CloudBackupRestoreReport {
            wallets_restored: 0,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
        };

        let mut existing_fingerprints = crate::backup::import::collect_existing_fingerprints()
            .inspect_err(|e| warn!("Failed to collect fingerprints: {e}"))
            .unwrap_or_default();

        // download and restore each wallet
        for (i, record_id) in manifest.wallet_record_ids.iter().enumerate() {
            match restore_single_wallet(cloud, record_id, &critical_key, &mut existing_fingerprints)
            {
                Ok(()) => report.wallets_restored += 1,
                Err(e) => {
                    warn!("Failed to restore wallet {record_id}: {e}");
                    report.wallets_failed += 1;
                    report.failed_wallet_errors.push(e.to_string());
                }
            }

            self.send(Message::ProgressUpdated { completed: (i + 1) as u32, total });
        }

        // mark enabled
        let now = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let db = Database::global();
        db.global_config
            .set_cloud_backup(&CloudBackup::Enabled { last_sync: Some(now) })
            .map_err(|e| CloudBackupError::Internal(format!("persist cloud backup state: {e}")))?;

        self.send(Message::RestoreComplete(report));
        self.send(Message::StateChanged(CloudBackupState::Enabled));
        info!("Cloud backup restore complete");
        Ok(())
    }
}

fn restore_single_wallet(
    cloud: &CloudStorage,
    record_id: &str,
    critical_key: &[u8; 32],
    existing_fingerprints: &mut Vec<(
        crate::wallet::fingerprint::Fingerprint,
        Network,
        crate::wallet::metadata::WalletMode,
    )>,
) -> Result<(), CloudBackupError> {
    let wallet_json = cloud
        .download_wallet_backup(record_id.to_string())
        .map_err(|e| CloudBackupError::Cloud(format!("download {record_id}: {e}")))?;

    let encrypted: cove_cspp::backup_data::EncryptedWalletBackup =
        serde_json::from_slice(&wallet_json)
            .map_err(|e| CloudBackupError::Internal(format!("deserialize wallet: {e}")))?;

    if encrypted.version != 1 {
        return Err(CloudBackupError::Internal(format!(
            "unsupported wallet backup version: {}",
            encrypted.version
        )));
    }

    let entry = wallet_crypto::decrypt_wallet_backup(&encrypted, critical_key)
        .map_err(|e| CloudBackupError::Crypto(format!("decrypt wallet: {e}")))?;

    // convert WalletEntry to WalletMetadata + restore
    let metadata: crate::wallet::metadata::WalletMetadata =
        serde_json::from_value(entry.metadata.clone())
            .map_err(|e| CloudBackupError::Internal(format!("parse wallet metadata: {e}")))?;

    // duplicate detection
    if crate::backup::import::is_wallet_duplicate(&metadata, existing_fingerprints).unwrap_or(false)
    {
        info!("Skipping duplicate wallet {}", metadata.name);
        return Ok(());
    }

    // build a WalletBackup-like structure for reuse of import helpers
    let backup_model = crate::backup::model::WalletBackup {
        metadata: entry.metadata.clone(),
        secret: convert_cloud_secret(&entry.secret),
        descriptors: entry.descriptors.as_ref().map(|d| crate::backup::model::DescriptorPair {
            external: d.external.clone(),
            internal: d.internal.clone(),
        }),
        xpub: entry.xpub.clone(),
        labels_jsonl: None,
    };

    match &backup_model.secret {
        crate::backup::model::WalletSecret::Mnemonic(words) => {
            let mnemonic = bip39::Mnemonic::from_str(words)
                .map_err(|e| CloudBackupError::Internal(format!("invalid mnemonic: {e}")))?;

            crate::backup::import::restore_mnemonic_wallet(&metadata, mnemonic).map_err(
                |(e, _)| CloudBackupError::Internal(format!("restore mnemonic wallet: {e}")),
            )?;
        }
        _ => {
            crate::backup::import::restore_descriptor_wallet(&metadata, &backup_model).map_err(
                |(e, _)| CloudBackupError::Internal(format!("restore descriptor wallet: {e}")),
            )?;
        }
    }

    // track fingerprint for duplicate detection of subsequent wallets
    if let Some(fp) = &metadata.master_fingerprint {
        existing_fingerprints.push((**fp, metadata.network, metadata.wallet_mode));
    }

    Ok(())
}

fn convert_cloud_secret(
    secret: &cove_cspp::backup_data::WalletSecret,
) -> crate::backup::model::WalletSecret {
    match secret {
        WalletSecret::Mnemonic(m) => crate::backup::model::WalletSecret::Mnemonic(m.clone()),
        WalletSecret::TapSignerBackup(b) => {
            crate::backup::model::WalletSecret::TapSignerBackup(b.clone())
        }
        WalletSecret::Descriptor(_) | WalletSecret::WatchOnly => {
            crate::backup::model::WalletSecret::None
        }
    }
}

fn build_wallet_entry(
    metadata: &crate::wallet::metadata::WalletMetadata,
    mode: crate::wallet::metadata::WalletMode,
) -> Result<WalletEntry, CloudBackupError> {
    let keychain = Keychain::global();
    let id = &metadata.id;
    let name = &metadata.name;

    let secret = match metadata.wallet_type {
        WalletType::Hot => match keychain.get_wallet_key(id) {
            Ok(Some(mnemonic)) => WalletSecret::Mnemonic(mnemonic.to_string()),
            Ok(None) => {
                return Err(CloudBackupError::Internal(format!(
                    "hot wallet '{name}' has no mnemonic"
                )));
            }
            Err(e) => {
                return Err(CloudBackupError::Internal(format!(
                    "failed to get mnemonic for '{name}': {e}"
                )));
            }
        },
        WalletType::Cold => {
            let is_tap_signer =
                metadata.hardware_metadata.as_ref().is_some_and(|hw| hw.is_tap_signer());

            if is_tap_signer {
                match keychain.get_tap_signer_backup(id) {
                    Ok(Some(backup)) => WalletSecret::TapSignerBackup(backup),
                    Ok(None) => {
                        warn!("Tap signer wallet '{name}' has no backup, exporting without it");
                        WalletSecret::WatchOnly
                    }
                    Err(e) => {
                        return Err(CloudBackupError::Internal(format!(
                            "failed to read tap signer backup for '{name}': {e}"
                        )));
                    }
                }
            } else {
                WalletSecret::WatchOnly
            }
        }
        WalletType::XpubOnly | WalletType::WatchOnly => WalletSecret::WatchOnly,
    };

    let xpub = match keychain.get_wallet_xpub(id) {
        Ok(Some(x)) => Some(x.to_string()),
        Ok(None) => None,
        Err(e) => {
            return Err(CloudBackupError::Internal(format!(
                "failed to read xpub for '{name}': {e}"
            )));
        }
    };

    let descriptors = match keychain.get_public_descriptor(id) {
        Ok(Some((ext, int))) => {
            Some(DescriptorPair { external: ext.to_string(), internal: int.to_string() })
        }
        Ok(None) => None,
        Err(e) => {
            return Err(CloudBackupError::Internal(format!(
                "failed to read descriptors for '{name}': {e}"
            )));
        }
    };

    let metadata_value = serde_json::to_value(metadata)
        .map_err(|e| CloudBackupError::Internal(format!("serialize metadata: {e}")))?;

    let wallet_mode = match mode {
        crate::wallet::metadata::WalletMode::Main => WalletMode::Main,
        crate::wallet::metadata::WalletMode::Decoy => WalletMode::Decoy,
    };

    Ok(WalletEntry {
        wallet_id: id.to_string(),
        secret,
        metadata: metadata_value,
        descriptors,
        xpub,
        wallet_mode,
    })
}

/// Wipe all local encrypted databases (main db + per-wallet databases)
///
/// Used during restore (old databases are undecryptable with the new key)
/// and during "Start Fresh" flows. Shared across platforms via FFI
#[uniffi::export]
pub fn wipe_local_data() {
    let root = &*cove_common::consts::ROOT_DATA_DIR;
    let db_path = root.join("cove.db");

    if db_path.exists()
        && let Err(e) = std::fs::remove_file(&db_path)
    {
        error!("Failed to remove cove.db: {e}");
    }

    // also remove per-wallet databases
    let wallet_dir = &*cove_common::consts::WALLET_DATA_DIR;
    if wallet_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(wallet_dir)
    {
        error!("Failed to remove wallet data dir: {e}");
    }
}

#[derive(Debug, thiserror::Error)]
enum CloudBackupError {
    #[error("not supported: {0}")]
    NotSupported(String),

    #[error("passkey error: {0}")]
    Passkey(String),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("cloud storage error: {0}")]
    Cloud(String),

    #[error("internal error: {0}")]
    Internal(String),
}

use cove_cspp::CsppStore as _;
use std::str::FromStr as _;

#[cfg(test)]
mod tests {
    use super::*;
    use cove_cspp::backup_data::WalletSecret;

    #[test]
    fn convert_cloud_secret_mnemonic() {
        let secret = WalletSecret::Mnemonic("abandon".into());
        let result = convert_cloud_secret(&secret);
        assert!(
            matches!(result, crate::backup::model::WalletSecret::Mnemonic(ref m) if m == "abandon")
        );
    }

    #[test]
    fn convert_cloud_secret_tap_signer() {
        let secret = WalletSecret::TapSignerBackup(vec![1, 2, 3]);
        let result = convert_cloud_secret(&secret);
        assert!(
            matches!(result, crate::backup::model::WalletSecret::TapSignerBackup(ref b) if b == &[1, 2, 3])
        );
    }

    #[test]
    fn convert_cloud_secret_descriptor_to_none() {
        let secret = WalletSecret::Descriptor("wpkh(...)".into());
        let result = convert_cloud_secret(&secret);
        assert!(matches!(result, crate::backup::model::WalletSecret::None));
    }

    #[test]
    fn convert_cloud_secret_watch_only_to_none() {
        let result = convert_cloud_secret(&WalletSecret::WatchOnly);
        assert!(matches!(result, crate::backup::model::WalletSecret::None));
    }
}
