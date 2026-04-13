use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use act_zero::call;
use bip39::Mnemonic;
use cove_cspp::CsppStore;
use cove_cspp::backup_data::{
    WalletEntry, WalletMode as CloudWalletMode, WalletSecret, wallet_filename_from_record_id,
};
use cove_device::cloud_storage::{CloudStorage, CloudStorageAccess, CloudStorageError};
use cove_device::keychain::{CSPP_NAMESPACE_ID_KEY, Keychain, KeychainAccess};
use cove_device::passkey::{
    DiscoveredPasskeyResult, PasskeyAccess, PasskeyCredentialPresence, PasskeyError,
    PasskeyProvider,
};
use parking_lot::Mutex;
use sha2::Digest as _;
use strum::IntoEnumIterator as _;

use super::*;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, CloudBlobFailedState, CloudBlobUploadingState, CloudUploadKind,
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::mnemonic::MnemonicExt as _;
use crate::network::Network;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletMode, WalletType};

#[derive(Debug, Default)]
pub(crate) struct MockStore {
    pub(crate) entries: Mutex<HashMap<String, String>>,
    pub(crate) save_count: Mutex<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct MockStoreHandle(pub(crate) Arc<MockStore>);

impl cove_cspp::CsppStore for MockStoreHandle {
    type Error = String;

    fn save(&self, key: String, value: String) -> Result<(), Self::Error> {
        *self.0.save_count.lock() += 1;
        self.0.entries.lock().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.0.entries.lock().get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.0.entries.lock().remove(&key).is_some()
    }
}

type MockDiscoverResult = Result<(Vec<u8>, Vec<u8>), PasskeyError>;
type MockPasskeyActionResult = Arc<Mutex<Option<Result<Vec<u8>, PasskeyError>>>>;

#[derive(Debug, Clone, Default)]
pub(crate) struct MockKeychain {
    entries: Arc<Mutex<HashMap<String, String>>>,
}

impl MockKeychain {
    fn reset(&self) {
        self.entries.lock().clear();
    }
}

impl KeychainAccess for MockKeychain {
    fn save(&self, key: String, value: String) -> Result<(), cove_device::keychain::KeychainError> {
        self.entries.lock().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.entries.lock().get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.entries.lock().remove(&key).is_some()
    }
}

#[derive(Debug, Default)]
struct MockCloudState {
    wallet_files: HashMap<String, Vec<String>>,
    master_key_backups: HashMap<String, Vec<u8>>,
    wallet_backups: HashMap<(String, String), Vec<u8>>,
    wallet_backup_download_overrides: HashMap<(String, String), Vec<u8>>,
    list_wallet_files_error: Option<CloudStorageError>,
    upload_master_key_error: Option<CloudStorageError>,
    next_upload_wallet_backup_error: Option<CloudStorageError>,
    upload_wallet_backup_error: Option<CloudStorageError>,
    reflect_uploaded_wallets_in_listing: bool,
    uploaded_wallet_backups: Vec<(String, String)>,
    wallet_backup_upload_attempts: usize,
    dirty_wallet_on_next_upload: Option<WalletId>,
    changed_wallet_on_next_upload: Option<WalletId>,
    dirty_wallet_on_next_backup_check: Option<WalletId>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MockCloudStorage {
    state: Arc<Mutex<MockCloudState>>,
}

impl MockCloudStorage {
    pub(crate) fn reset(&self) {
        *self.state.lock() = MockCloudState::default();
    }

    pub(crate) fn set_wallet_files(&self, namespace: String, wallet_files: Vec<String>) {
        self.state.lock().wallet_files.insert(namespace, wallet_files);
    }

    pub(crate) fn set_master_key_backup(&self, namespace: String, backup: Vec<u8>) {
        self.state.lock().master_key_backups.insert(namespace, backup);
    }

    pub(crate) fn set_wallet_backup(&self, namespace: String, record_id: String, backup: Vec<u8>) {
        self.state.lock().wallet_backups.insert((namespace, record_id), backup);
    }

    pub(crate) fn set_wallet_backup_download_override(
        &self,
        namespace: String,
        record_id: String,
        backup: Vec<u8>,
    ) {
        self.state.lock().wallet_backup_download_overrides.insert((namespace, record_id), backup);
    }

    pub(crate) fn fail_list_wallet_files(&self, message: &str) {
        self.state.lock().list_wallet_files_error =
            Some(CloudStorageError::DownloadFailed(message.into()));
    }

    pub(crate) fn clear_list_wallet_files_failure(&self) {
        self.state.lock().list_wallet_files_error = None;
    }

    pub(crate) fn fail_master_key_upload(&self, message: &str) {
        self.state.lock().upload_master_key_error =
            Some(CloudStorageError::UploadFailed(message.into()));
    }

    pub(crate) fn fail_wallet_backup_upload(&self, message: &str) {
        self.state.lock().upload_wallet_backup_error =
            Some(CloudStorageError::UploadFailed(message.into()));
    }

    pub(crate) fn fail_wallet_backup_upload_quota_exceeded(&self) {
        self.state.lock().upload_wallet_backup_error = Some(CloudStorageError::QuotaExceeded);
    }

    pub(crate) fn fail_next_wallet_backup_upload(&self, message: &str) {
        self.state.lock().next_upload_wallet_backup_error =
            Some(CloudStorageError::UploadFailed(message.into()));
    }

    pub(crate) fn clear_wallet_backup_upload_failure(&self) {
        let mut state = self.state.lock();
        state.next_upload_wallet_backup_error = None;
        state.upload_wallet_backup_error = None;
    }

    pub(crate) fn set_reflect_uploaded_wallets_in_listing(&self, enabled: bool) {
        self.state.lock().reflect_uploaded_wallets_in_listing = enabled;
    }

    pub(crate) fn uploaded_wallet_backup_count(&self) -> usize {
        self.state.lock().uploaded_wallet_backups.len()
    }

    pub(crate) fn wallet_backup_upload_attempt_count(&self) -> usize {
        self.state.lock().wallet_backup_upload_attempts
    }

    pub(crate) fn dirty_wallet_on_next_upload(&self, wallet_id: WalletId) {
        self.state.lock().dirty_wallet_on_next_upload = Some(wallet_id);
    }

    pub(crate) fn change_wallet_on_next_upload(&self, wallet_id: WalletId) {
        self.state.lock().changed_wallet_on_next_upload = Some(wallet_id);
    }

    pub(crate) fn dirty_wallet_on_next_backup_check(&self, wallet_id: WalletId) {
        self.state.lock().dirty_wallet_on_next_backup_check = Some(wallet_id);
    }
}

impl CloudStorageAccess for MockCloudStorage {
    fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        if let Some(error) = self.state.lock().upload_master_key_error.clone() {
            return Err(error);
        }

        self.state.lock().master_key_backups.insert(namespace, data);
        Ok(())
    }

    fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        let (dirty_wallet, changed_wallet) = {
            let mut state = self.state.lock();
            state.wallet_backup_upload_attempts += 1;
            if let Some(error) = state.next_upload_wallet_backup_error.take() {
                return Err(error);
            }

            if let Some(error) = state.upload_wallet_backup_error.clone() {
                return Err(error);
            }

            let dirty_wallet = state.dirty_wallet_on_next_upload.take();
            let changed_wallet = state.changed_wallet_on_next_upload.take();
            state.wallet_backups.insert((namespace.clone(), record_id.clone()), data);
            state.uploaded_wallet_backups.push((namespace, record_id));
            (dirty_wallet, changed_wallet)
        };
        if let Some(wallet_id) = dirty_wallet {
            persist_dirty_blob_state(wallet_id);
        }
        if let Some(wallet_id) = changed_wallet {
            mutate_wallet_and_persist_dirty(wallet_id);
        }
        Ok(())
    }

    fn download_master_key_backup(&self, namespace: String) -> Result<Vec<u8>, CloudStorageError> {
        self.state
            .lock()
            .master_key_backups
            .get(&namespace)
            .cloned()
            .ok_or(CloudStorageError::NotFound(namespace))
    }

    fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<Vec<u8>, CloudStorageError> {
        let dirty_wallet = self.state.lock().dirty_wallet_on_next_backup_check.take();
        if let Some(wallet_id) = dirty_wallet {
            persist_dirty_blob_state(wallet_id);
        }

        let override_key = (namespace.clone(), record_id.clone());
        if let Some(backup) =
            self.state.lock().wallet_backup_download_overrides.get(&override_key).cloned()
        {
            return Ok(backup);
        }

        self.state
            .lock()
            .wallet_backups
            .get(&(namespace.clone(), record_id.clone()))
            .cloned()
            .ok_or(CloudStorageError::NotFound(format!("{namespace}/{record_id}")))
    }

    fn delete_wallet_backup(
        &self,
        _namespace: String,
        _record_id: String,
    ) -> Result<(), CloudStorageError> {
        Ok(())
    }

    fn list_namespaces(&self) -> Result<Vec<String>, CloudStorageError> {
        Ok(self.state.lock().wallet_files.keys().cloned().collect())
    }

    fn list_wallet_files(&self, namespace: String) -> Result<Vec<String>, CloudStorageError> {
        let state = self.state.lock();
        if let Some(error) = state.list_wallet_files_error.clone() {
            return Err(error);
        }
        let mut wallet_files = state.wallet_files.get(&namespace).cloned().unwrap_or_default();

        if state.reflect_uploaded_wallets_in_listing {
            for (uploaded_namespace, record_id) in &state.uploaded_wallet_backups {
                if uploaded_namespace == &namespace {
                    let filename = wallet_filename_from_record_id(record_id);
                    if !wallet_files.contains(&filename) {
                        wallet_files.push(filename);
                    }
                }
            }
        }

        Ok(wallet_files)
    }

    fn is_backup_uploaded(
        &self,
        _namespace: String,
        _record_id: String,
    ) -> Result<bool, CloudStorageError> {
        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MockPasskeyProviderImpl {
    discover_result: Arc<Mutex<MockDiscoverResult>>,
    create_result: MockPasskeyActionResult,
    authenticate_result: MockPasskeyActionResult,
}

impl Default for MockPasskeyProviderImpl {
    fn default() -> Self {
        Self {
            discover_result: Arc::new(Mutex::new(Err(PasskeyError::NoCredentialFound))),
            create_result: Arc::new(Mutex::new(None)),
            authenticate_result: Arc::new(Mutex::new(None)),
        }
    }
}

impl MockPasskeyProviderImpl {
    pub(crate) fn reset(&self) {
        *self.discover_result.lock() = Err(PasskeyError::NoCredentialFound);
        *self.create_result.lock() = None;
        *self.authenticate_result.lock() = None;
    }

    pub(crate) fn set_discover_result(
        &self,
        result: Result<DiscoveredPasskeyResult, PasskeyError>,
    ) {
        *self.discover_result.lock() = result.map(|value| (value.prf_output, value.credential_id));
    }

    pub(crate) fn set_create_result(&self, result: Result<Vec<u8>, PasskeyError>) {
        *self.create_result.lock() = Some(result);
    }

    pub(crate) fn set_authenticate_result(&self, result: Result<Vec<u8>, PasskeyError>) {
        *self.authenticate_result.lock() = Some(result);
    }
}

impl PasskeyProvider for MockPasskeyProviderImpl {
    fn create_passkey(
        &self,
        _rp_id: String,
        _user_id: Vec<u8>,
        _challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.create_result.lock().take().unwrap_or_else(|| {
            Err(PasskeyError::CreationFailed("unexpected create_passkey call".into()))
        })
    }

    fn authenticate_with_prf(
        &self,
        _rp_id: String,
        _credential_id: Vec<u8>,
        _prf_salt: Vec<u8>,
        _challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.authenticate_result.lock().take().unwrap_or_else(|| {
            Err(PasskeyError::AuthenticationFailed("unexpected authenticate_with_prf call".into()))
        })
    }

    fn discover_and_authenticate_with_prf(
        &self,
        _rp_id: String,
        _prf_salt: Vec<u8>,
        _challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError> {
        self.discover_result.lock().clone().map(|(prf_output, credential_id)| {
            DiscoveredPasskeyResult { prf_output, credential_id }
        })
    }

    fn is_prf_supported(&self) -> bool {
        true
    }

    fn check_passkey_presence(
        &self,
        _rp_id: String,
        _credential_id: Vec<u8>,
    ) -> PasskeyCredentialPresence {
        PasskeyCredentialPresence::Present
    }
}

pub(crate) struct TestGlobals {
    pub(crate) keychain: MockKeychain,
    pub(crate) cloud: MockCloudStorage,
    pub(crate) passkey: MockPasskeyProviderImpl,
}

impl TestGlobals {
    fn init() -> Self {
        let keychain = MockKeychain::default();
        let cloud = MockCloudStorage::default();
        let passkey = MockPasskeyProviderImpl::default();

        let _ = Keychain::new(Box::new(keychain.clone()));
        let _ = CloudStorage::new(Box::new(cloud.clone()));
        let _ = PasskeyAccess::new(Box::new(passkey.clone()));

        Self { keychain, cloud, passkey }
    }

    pub(crate) fn reset(&self) {
        self.keychain.reset();
        self.cloud.reset();
        self.passkey.reset();
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();
    }
}

pub(crate) fn init_test_runtime() {
    super::super::ensure_cloud_backup_test_tokio_runtime();
}

pub(crate) fn test_globals() -> &'static TestGlobals {
    static GLOBALS: OnceLock<TestGlobals> = OnceLock::new();
    init_test_runtime();
    GLOBALS.get_or_init(TestGlobals::init)
}

pub(crate) fn test_lock() -> &'static parking_lot::Mutex<()> {
    super::super::cloud_backup_test_lock()
}

fn clear_local_wallets() {
    let wallets = Database::global().wallets();
    for network in Network::iter() {
        for mode in WalletMode::iter() {
            wallets.save_all_wallets(network, mode, Vec::new()).unwrap();
        }
    }
}

pub(crate) fn persist_dirty_blob_state(wallet_id: WalletId) {
    let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
    let changed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id,
            wallet_id: Some(wallet_id),
            record_id,
            state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
        })
        .unwrap();
}

fn mutate_wallet_and_persist_dirty(wallet_id: WalletId) {
    let mut wallet = all_local_wallets(&Database::global())
        .unwrap()
        .into_iter()
        .find(|wallet| wallet.id == wallet_id)
        .unwrap();
    wallet.name.push_str(" updated");
    Database::global()
        .wallets()
        .save_all_wallets(wallet.network, wallet.wallet_mode, vec![wallet.clone()])
        .unwrap();
    persist_dirty_blob_state(wallet.id);
}

pub(crate) fn persist_failed_blob_state(wallet_id: WalletId, retryable: bool) {
    let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
    let failed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id,
            wallet_id: Some(wallet_id),
            record_id,
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: Some("rev-1".into()),
                retryable,
                error: "failed".into(),
                failed_at,
            }),
        })
        .unwrap();
}

pub(crate) fn persist_uploading_blob_state(wallet_id: WalletId, started_at: u64) {
    let namespace_id = Keychain::global().get(CSPP_NAMESPACE_ID_KEY.into()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState {
            kind: CloudUploadKind::BackupBlob,
            namespace_id,
            wallet_id: Some(wallet_id),
            record_id,
            state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState {
                revision_hash: "rev-1".into(),
                started_at,
            }),
        })
        .unwrap();
}

pub(crate) fn reset_cloud_backup_test_state(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
) {
    init_test_runtime();
    globals.reset();
    clear_local_wallets();
    let reset_manager = manager.clone();
    std::thread::spawn(move || reset_manager.debug_reset_cloud_backup_state())
        .join()
        .expect("reset cloud backup test state thread");
    manager.clear_wallet_upload_debouncers_for_test();
}

pub(crate) async fn wait_for_test_condition(
    timeout: Duration,
    message: &str,
    mut condition: impl FnMut() -> bool,
) {
    tokio::time::timeout(timeout, async {
        loop {
            if condition() {
                break;
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect(message);
}

pub(crate) async fn assert_test_condition_stays_true(
    duration: Duration,
    message: &str,
    mut condition: impl FnMut() -> bool,
) {
    let deadline = tokio::time::Instant::now() + duration;
    while tokio::time::Instant::now() < deadline {
        assert!(condition(), "{message}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) fn configure_enabled_cloud_backup(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
    wallet_count: u32,
) {
    reset_cloud_backup_test_state(manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let keychain = Keychain::global();
    keychain.save(CSPP_NAMESPACE_ID_KEY.into(), namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Enabled,
                wallet_count: Some(wallet_count),
                ..PersistedCloudBackupState::default()
            },
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();
}

pub(crate) fn enable_cloud_backup_without_reset(
    manager: &RustCloudBackupManager,
    wallet_count: u32,
) {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let keychain = Keychain::global();
    keychain.save(CSPP_NAMESPACE_ID_KEY.into(), namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Enabled,
                wallet_count: Some(wallet_count),
                ..PersistedCloudBackupState::default()
            },
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();
}

pub(crate) fn xpub_only_wallet_metadata() -> WalletMetadata {
    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::XpubOnly;
    metadata
}

pub(crate) fn sample_xpub(metadata: &WalletMetadata) -> String {
    let mnemonic = Mnemonic::parse(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    mnemonic.xpub(metadata.network.into()).to_string()
}

pub(crate) fn persist_xpub_wallets(wallets: Vec<WalletMetadata>) {
    for wallet in &wallets {
        let xpub = sample_xpub(wallet);
        Keychain::global().save_wallet_xpub(&wallet.id, xpub.parse().unwrap()).unwrap();
    }

    let mut wallets_by_scope = HashMap::new();
    for wallet in wallets {
        wallets_by_scope
            .entry((wallet.network, wallet.wallet_mode))
            .or_insert_with(Vec::new)
            .push(wallet);
    }

    for ((network, wallet_mode), wallets) in wallets_by_scope {
        Database::global().wallets().save_all_wallets(network, wallet_mode, wallets).unwrap();
    }
}

pub(crate) fn encrypted_wallet_backup_bytes(
    metadata: &WalletMetadata,
    master_key: &cove_cspp::master_key::MasterKey,
    revision_hash: &str,
    version: u32,
) -> Vec<u8> {
    let mut prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        metadata,
        metadata.wallet_mode,
    )
    .unwrap();
    prepared.entry.content_revision_hash = revision_hash.to_string();

    let critical_key = zeroize::Zeroizing::new(master_key.critical_data_key());
    let mut encrypted =
        cove_cspp::wallet_crypto::encrypt_wallet_entry(&prepared.entry, &critical_key).unwrap();
    encrypted.version = version;
    serde_json::to_vec(&encrypted).unwrap()
}

pub(crate) fn wallet_entry_with_labels(
    metadata: &WalletMetadata,
    labels_jsonl: Option<&str>,
) -> WalletEntry {
    let labels_count = labels_jsonl
        .map(|jsonl| jsonl.lines().filter(|line| !line.trim().is_empty()).count() as u32)
        .unwrap_or_default();
    let labels_zstd_jsonl =
        labels_jsonl.map(|jsonl| crate::backup::crypto::compress(jsonl.as_bytes()).unwrap());
    let labels_hash = labels_jsonl
        .filter(|jsonl| !jsonl.is_empty())
        .map(|jsonl| hex::encode(sha2::Sha256::digest(jsonl.as_bytes())));
    let labels_uncompressed_size =
        labels_jsonl.map(|jsonl| jsonl.len().try_into().unwrap_or(u32::MAX));

    WalletEntry {
        wallet_id: metadata.id.to_string(),
        secret: WalletSecret::WatchOnly,
        metadata: serde_json::to_value(metadata).unwrap(),
        descriptors: None,
        xpub: Some(sample_xpub(metadata)),
        wallet_mode: CloudWalletMode::Main,
        labels_zstd_jsonl,
        labels_count,
        labels_hash,
        labels_uncompressed_size,
        content_revision_hash: "test-content-hash".to_string(),
        updated_at: 42,
    }
}

pub(crate) fn encrypted_wallet_backup_bytes_for_entry(
    entry: &WalletEntry,
    master_key: &cove_cspp::master_key::MasterKey,
    version: u32,
) -> Vec<u8> {
    let critical_key = zeroize::Zeroizing::new(master_key.critical_data_key());
    let mut encrypted =
        cove_cspp::wallet_crypto::encrypt_wallet_entry(entry, &critical_key).unwrap();
    encrypted.version = version;
    serde_json::to_vec(&encrypted).unwrap()
}

pub(crate) fn sample_labels_jsonl() -> &'static str {
    r#"{"type":"tx","ref":"d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290","label":"last txn received","origin":"pkh([73c5da0a/44h/0h/0h])"}"#
}

pub(crate) fn prepare_deep_verify_with_unsynced_wallet(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
) -> crate::wallet::metadata::WalletMetadata {
    reset_cloud_backup_test_state(manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let prf_key = [7u8; 32];
    let prf_salt = [9u8; 32];
    let credential_id = vec![1, 2, 3, 4];
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();

    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.cloud.set_reflect_uploaded_wallets_in_listing(false);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id,
    }));

    let keychain = Keychain::global();
    keychain.save(CSPP_NAMESPACE_ID_KEY.into(), namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Enabled,
                ..PersistedCloudBackupState::default()
            },
            "set cloud backup enabled for test",
        )
        .unwrap();

    let mut metadata = crate::wallet::metadata::WalletMetadata::preview_new();
    metadata.wallet_type = crate::wallet::metadata::WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    metadata
}
pub(crate) async fn clear_wallet_upload_runtime_for_test_async(manager: &RustCloudBackupManager) {
    call!(manager.runtime.clear_upload_runtime_state()).await.expect("clear upload runtime state");
}

pub(crate) async fn run_wallet_upload_for_test_async(
    manager: &RustCloudBackupManager,
    wallet_id: WalletId,
) {
    call!(manager.runtime.run_wallet_upload_inline_for_test(wallet_id))
        .await
        .expect("run wallet upload");
}

pub(crate) fn new_restore_operation_for_test(
    manager: &RustCloudBackupManager,
) -> super::super::runtime_actor::RestoreOperation {
    let runtime = manager.runtime.clone();
    std::thread::spawn(move || {
        cove_tokio::task::block_on(call!(runtime.new_restore_operation()))
            .expect("create restore operation")
    })
    .join()
    .expect("restore operation thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkey_create_result_is_consumed_after_first_use() {
        let provider = MockPasskeyProviderImpl::default();
        provider.set_create_result(Ok(vec![1, 2, 3]));

        assert_eq!(
            provider
                .create_passkey("rp".into(), vec![1], vec![2])
                .expect("configured create result"),
            vec![1, 2, 3]
        );
        assert!(matches!(
            provider.create_passkey("rp".into(), vec![1], vec![2]),
            Err(PasskeyError::CreationFailed(message)) if message == "unexpected create_passkey call"
        ));
    }

    #[test]
    fn passkey_authenticate_result_is_consumed_after_first_use() {
        let provider = MockPasskeyProviderImpl::default();
        provider.set_authenticate_result(Ok(vec![4, 5, 6]));

        assert_eq!(
            provider
                .authenticate_with_prf("rp".into(), vec![1], vec![2], vec![3])
                .expect("configured authenticate result"),
            vec![4, 5, 6]
        );
        assert!(matches!(
            provider.authenticate_with_prf("rp".into(), vec![1], vec![2], vec![3]),
            Err(PasskeyError::AuthenticationFailed(message))
                if message == "unexpected authenticate_with_prf call"
        ));
    }
}
