use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use act_zero::call;
use bip39::Mnemonic;
use cove_cspp::backup_data::{
    MASTER_KEY_RECORD_ID, WalletEntry, WalletMode as CloudWalletMode, WalletSecret,
    wallet_filename_from_record_id,
};
use cove_device::cloud_storage::{
    CloudAccessPolicy, CloudStorage, CloudStorageAccess, CloudStorageError, CloudSyncHealth,
};
use cove_device::keychain::{Keychain, KeychainAccess};
use cove_device::passkey::{
    DiscoveredPasskeyResult, PasskeyAccess, PasskeyCredentialPresence, PasskeyError,
    PasskeyFailureReason, PasskeyOperation, PasskeyProvider, PasskeyRegistrationPlatform,
    PasskeyRegistrationResult, PasskeyRegistrationUser,
};
use parking_lot::Mutex;
use sha2::Digest as _;
use strum::IntoEnumIterator as _;

use super::*;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, CloudBlobFailedState, CloudBlobFailureIssue, CloudBlobUploadingState,
    PersistedBackupSyncState, PersistedBackupVerificationState, PersistedCloudBackupState,
    PersistedCloudBlobState, PersistedCloudBlobSyncState, PersistedConfiguredCloudBackup,
    PersistedDisablingCloudBackup, PersistedPasskeyState,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupStore, PendingEnableSession, actors::restore::RestoreOperation,
    pending::PendingUploadVerificationStatus,
};
use crate::manager::connectivity_manager::CONNECTIVITY_MANAGER;
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
type MockPasskeyCreateResult = Arc<Mutex<Option<Result<PasskeyRegistrationResult, PasskeyError>>>>;
type MockPasskeyPresenceResults = Arc<Mutex<VecDeque<PasskeyCredentialPresence>>>;
#[derive(Debug, Clone, Default)]
pub(crate) struct MockKeychain {
    entries: Arc<Mutex<HashMap<String, String>>>,
    fail_save_at: Arc<Mutex<Option<usize>>>,
    fail_delete_at: Arc<Mutex<Option<usize>>>,
    save_count: Arc<Mutex<usize>>,
    delete_count: Arc<Mutex<usize>>,
}

impl MockKeychain {
    fn reset(&self) {
        self.entries.lock().clear();
        *self.fail_save_at.lock() = None;
        *self.fail_delete_at.lock() = None;
        *self.save_count.lock() = 0;
        *self.delete_count.lock() = 0;
    }

    pub(crate) fn set_entries(&self, entries: Vec<(&str, &str)>) {
        *self.entries.lock() =
            entries.into_iter().map(|(key, value)| (key.into(), value.into())).collect();
    }

    pub(crate) fn get_entry(&self, key: &str) -> Option<String> {
        self.entries.lock().get(key).cloned()
    }

    pub(crate) fn fail_save_at(&self, save_attempt: usize) {
        *self.save_count.lock() = 0;
        *self.fail_save_at.lock() = Some(save_attempt);
    }

    pub(crate) fn fail_delete_at(&self, delete_attempt: usize) {
        *self.delete_count.lock() = 0;
        *self.fail_delete_at.lock() = Some(delete_attempt);
    }
}

impl KeychainAccess for MockKeychain {
    fn save(&self, key: String, value: String) -> Result<(), cove_device::keychain::KeychainError> {
        let mut save_count = self.save_count.lock();
        *save_count += 1;
        if Some(*save_count) == *self.fail_save_at.lock() {
            return Err(cove_device::keychain::KeychainError::Save);
        }

        self.entries.lock().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.entries.lock().get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        let mut delete_count = self.delete_count.lock();
        *delete_count += 1;
        if Some(*delete_count) == *self.fail_delete_at.lock() {
            return false;
        }

        self.entries.lock().remove(&key).is_some()
    }
}

#[derive(Debug, Default)]
struct MockCloudState {
    wallet_files: HashMap<String, Vec<String>>,
    master_key_backups: HashMap<String, Vec<u8>>,
    master_key_download_errors: HashMap<String, CloudStorageError>,
    wallet_backups: HashMap<(String, String), Vec<u8>>,
    wallet_backup_download_overrides: HashMap<(String, String), Vec<u8>>,
    wallet_backup_download_errors: HashMap<(String, String), CloudStorageError>,
    next_list_wallet_files_error: Option<CloudStorageError>,
    list_wallet_files_error: Option<CloudStorageError>,
    list_wallet_files_namespace_errors: HashMap<String, CloudStorageError>,
    list_wallet_files_non_interactive_error: Option<CloudStorageError>,
    upload_master_key_error: Option<CloudStorageError>,
    next_upload_wallet_backup_error: Option<CloudStorageError>,
    upload_wallet_backup_error: Option<CloudStorageError>,
    delete_namespace_error: Option<CloudStorageError>,
    list_namespaces_error: Option<CloudStorageError>,
    reflect_uploaded_wallets_in_listing: bool,
    uploaded_wallet_backups: Vec<(String, String)>,
    deleted_namespace_policies: Vec<CloudAccessPolicy>,
    delete_namespace_attempts: usize,
    list_wallet_files_attempts: usize,
    list_wallet_files_attempts_by_namespace: HashMap<String, usize>,
    wallet_backup_upload_attempts: usize,
    dirty_wallet_on_next_upload: Option<WalletId>,
    changed_wallet_on_next_upload: Option<WalletId>,
    dirty_wallet_on_next_backup_check: Option<WalletId>,
    disabling_on_next_upload: Option<PersistedDisablingCloudBackup>,
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

    pub(crate) fn fail_wallet_backup_download_offline(
        &self,
        namespace: String,
        record_id: String,
        message: &str,
    ) {
        self.state
            .lock()
            .wallet_backup_download_errors
            .insert((namespace, record_id), CloudStorageError::Offline(message.into()));
    }

    pub(crate) fn fail_list_wallet_files(&self, message: &str) {
        self.state.lock().list_wallet_files_error =
            Some(CloudStorageError::DownloadFailed(message.into()));
    }

    pub(crate) fn fail_list_wallet_files_for_namespace(
        &self,
        namespace: String,
        error: CloudStorageError,
    ) {
        self.state.lock().list_wallet_files_namespace_errors.insert(namespace, error);
    }

    pub(crate) fn fail_next_list_wallet_files_offline(&self, message: &str) {
        self.state.lock().next_list_wallet_files_error =
            Some(CloudStorageError::Offline(message.into()));
    }

    pub(crate) fn fail_list_wallet_files_non_interactive(&self, message: &str) {
        self.state.lock().list_wallet_files_non_interactive_error =
            Some(CloudStorageError::DownloadFailed(message.into()));
    }

    pub(crate) fn fail_list_namespaces(&self, message: &str) {
        self.state.lock().list_namespaces_error =
            Some(CloudStorageError::DownloadFailed(message.into()));
    }

    pub(crate) fn clear_list_wallet_files_non_interactive_failure(&self) {
        self.state.lock().list_wallet_files_non_interactive_error = None;
    }

    pub(crate) fn fail_master_key_upload(&self, message: &str) {
        self.state.lock().upload_master_key_error =
            Some(CloudStorageError::UploadFailed(message.into()));
    }

    pub(crate) fn fail_master_key_download_offline(&self, namespace: String, message: &str) {
        self.state
            .lock()
            .master_key_download_errors
            .insert(namespace, CloudStorageError::Offline(message.into()));
    }

    pub(crate) fn fail_master_key_download_authorization_required(
        &self,
        namespace: String,
        message: &str,
    ) {
        self.state
            .lock()
            .master_key_download_errors
            .insert(namespace, CloudStorageError::AuthorizationRequired(message.into()));
    }

    pub(crate) fn fail_wallet_backup_upload(&self, message: &str) {
        self.state.lock().upload_wallet_backup_error =
            Some(CloudStorageError::UploadFailed(message.into()));
    }

    pub(crate) fn fail_wallet_backup_upload_quota_exceeded(&self) {
        self.state.lock().upload_wallet_backup_error = Some(CloudStorageError::QuotaExceeded);
    }

    pub(crate) fn fail_next_wallet_backup_upload_offline(&self, message: &str) {
        self.state.lock().next_upload_wallet_backup_error =
            Some(CloudStorageError::Offline(message.into()));
    }

    pub(crate) fn clear_wallet_backup_upload_failure(&self) {
        let mut state = self.state.lock();
        state.next_upload_wallet_backup_error = None;
        state.upload_wallet_backup_error = None;
    }

    pub(crate) fn fail_delete_namespace(&self, message: &str) {
        self.state.lock().delete_namespace_error =
            Some(CloudStorageError::DownloadFailed(message.into()));
    }

    pub(crate) fn fail_delete_namespace_not_found(&self, message: &str) {
        self.state.lock().delete_namespace_error =
            Some(CloudStorageError::NotFound(message.into()));
    }

    pub(crate) fn set_reflect_uploaded_wallets_in_listing(&self, enabled: bool) {
        self.state.lock().reflect_uploaded_wallets_in_listing = enabled;
    }

    pub(crate) fn uploaded_wallet_backup_count(&self) -> usize {
        self.state.lock().uploaded_wallet_backups.len()
    }

    pub(crate) fn has_master_key_backup(&self, namespace: &str) -> bool {
        self.state.lock().master_key_backups.contains_key(namespace)
    }

    pub(crate) fn has_namespace(&self, namespace: &str) -> bool {
        let state = self.state.lock();
        state.master_key_backups.contains_key(namespace)
            || state.wallet_files.contains_key(namespace)
            || state
                .wallet_backups
                .keys()
                .any(|(backup_namespace, _)| backup_namespace == namespace)
    }

    pub(crate) fn deleted_namespace_policies(&self) -> Vec<CloudAccessPolicy> {
        self.state.lock().deleted_namespace_policies.clone()
    }

    pub(crate) fn delete_namespace_attempt_count(&self) -> usize {
        self.state.lock().delete_namespace_attempts
    }

    pub(crate) fn wallet_backup_upload_attempt_count(&self) -> usize {
        self.state.lock().wallet_backup_upload_attempts
    }

    pub(crate) fn list_wallet_files_attempt_count(&self) -> usize {
        self.state.lock().list_wallet_files_attempts
    }

    pub(crate) fn list_wallet_files_attempt_count_for_namespace(&self, namespace: &str) -> usize {
        self.state
            .lock()
            .list_wallet_files_attempts_by_namespace
            .get(namespace)
            .copied()
            .unwrap_or_default()
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

    pub(crate) fn persist_disabling_on_next_upload(
        &self,
        disabling: PersistedDisablingCloudBackup,
    ) {
        self.state.lock().disabling_on_next_upload = Some(disabling);
    }
}

#[async_trait::async_trait]
impl CloudStorageAccess for MockCloudStorage {
    async fn upload_master_key_backup(
        &self,
        namespace: String,
        _location: cove_device::cloud_storage::RemoteBackupLocation,
        data: Vec<u8>,
        _policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        if let Some(error) = self.state.lock().upload_master_key_error.clone() {
            return Err(error);
        }

        self.state.lock().master_key_backups.insert(namespace, data);
        Ok(())
    }

    async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        _location: cove_device::cloud_storage::RemoteBackupLocation,
        data: Vec<u8>,
        _policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let (dirty_wallet, changed_wallet, disabling) = {
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
            let disabling = state.disabling_on_next_upload.take();
            state.wallet_backups.insert((namespace.clone(), record_id.clone()), data);
            state.uploaded_wallet_backups.push((namespace, record_id));
            (dirty_wallet, changed_wallet, disabling)
        };
        if let Some(wallet_id) = dirty_wallet {
            persist_dirty_blob_state(wallet_id);
        }
        if let Some(wallet_id) = changed_wallet {
            mutate_wallet_and_persist_dirty(wallet_id);
        }
        if let Some(disabling) = disabling {
            Database::global()
                .cloud_backup_state
                .set(&PersistedCloudBackupState::Disabling(disabling))
                .unwrap();
        }
        Ok(())
    }

    async fn download_master_key_backup(
        &self,
        namespace: String,
        _locations: Vec<cove_device::cloud_storage::RemoteBackupLocation>,
        _policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        if let Some(error) = self.state.lock().master_key_download_errors.get(&namespace).cloned() {
            return Err(error);
        }

        self.state
            .lock()
            .master_key_backups
            .get(&namespace)
            .cloned()
            .ok_or(CloudStorageError::NotFound(namespace))
    }

    async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        _locations: Vec<cove_device::cloud_storage::RemoteBackupLocation>,
        _policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        let dirty_wallet = self.state.lock().dirty_wallet_on_next_backup_check.take();
        if let Some(wallet_id) = dirty_wallet {
            persist_dirty_blob_state(wallet_id);
        }

        let override_key = (namespace.clone(), record_id.clone());
        if let Some(error) =
            self.state.lock().wallet_backup_download_errors.get(&override_key).cloned()
        {
            return Err(error);
        }

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

    async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        _locations: Vec<cove_device::cloud_storage::RemoteBackupLocation>,
        _policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let mut state = self.state.lock();
        if record_id == MASTER_KEY_RECORD_ID {
            state.master_key_backups.remove(&namespace);
            return Ok(());
        }

        state.wallet_backups.remove(&(namespace.clone(), record_id.clone()));
        state.uploaded_wallet_backups.retain(|(uploaded_namespace, uploaded_record_id)| {
            uploaded_namespace != &namespace || uploaded_record_id != &record_id
        });
        Ok(())
    }

    async fn delete_namespace(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let mut state = self.state.lock();
        state.delete_namespace_attempts += 1;
        if let Some(error) = state.delete_namespace_error.clone() {
            return Err(error);
        }

        state.deleted_namespace_policies.push(policy);
        state.master_key_backups.remove(&namespace);
        state.master_key_download_errors.remove(&namespace);
        state.wallet_files.remove(&namespace);
        state.wallet_backups.retain(|(backup_namespace, _), _| backup_namespace != &namespace);
        state
            .wallet_backup_download_overrides
            .retain(|(backup_namespace, _), _| backup_namespace != &namespace);
        state
            .wallet_backup_download_errors
            .retain(|(backup_namespace, _), _| backup_namespace != &namespace);
        state
            .uploaded_wallet_backups
            .retain(|(uploaded_namespace, _)| uploaded_namespace != &namespace);
        Ok(())
    }

    async fn list_namespaces(
        &self,
        _policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        let state = self.state.lock();
        if let Some(error) = state.list_namespaces_error.clone() {
            return Err(error);
        }

        let mut namespaces: std::collections::HashSet<String> =
            state.wallet_files.keys().cloned().collect();
        namespaces.extend(state.master_key_backups.keys().cloned());
        namespaces.extend(state.wallet_backups.keys().map(|(namespace, _)| namespace.clone()));

        Ok(namespaces.into_iter().collect())
    }

    async fn list_wallet_files(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        let mut state = self.state.lock();
        state.list_wallet_files_attempts += 1;
        *state.list_wallet_files_attempts_by_namespace.entry(namespace.clone()).or_default() += 1;
        if let Some(error) = state.next_list_wallet_files_error.take() {
            return Err(error);
        }

        let error = if policy == CloudAccessPolicy::Silent {
            state.list_wallet_files_non_interactive_error.clone()
        } else {
            state.list_wallet_files_error.clone()
        };
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(error) = state.list_wallet_files_namespace_errors.get(&namespace).cloned() {
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

    async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
        _locations: Vec<cove_device::cloud_storage::RemoteBackupLocation>,
        _policy: CloudAccessPolicy,
    ) -> Result<bool, CloudStorageError> {
        let state = self.state.lock();
        if record_id == MASTER_KEY_RECORD_ID {
            return Ok(state.master_key_backups.contains_key(&namespace));
        }

        Ok(state.wallet_backups.contains_key(&(namespace, record_id)))
    }

    async fn overall_sync_health(&self, _policy: CloudAccessPolicy) -> CloudSyncHealth {
        CloudSyncHealth::AllUploaded
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MockPasskeyProviderImpl {
    discover_results: Arc<Mutex<VecDeque<MockDiscoverResult>>>,
    create_result: MockPasskeyCreateResult,
    authenticate_result: MockPasskeyActionResult,
    create_count: Arc<Mutex<usize>>,
    authenticate_count: Arc<Mutex<usize>>,
    discover_count: Arc<Mutex<usize>>,
    presence_results: MockPasskeyPresenceResults,
    authenticated_credential_ids: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Default for MockPasskeyProviderImpl {
    fn default() -> Self {
        Self {
            discover_results: Arc::new(Mutex::new(VecDeque::new())),
            create_result: Arc::new(Mutex::new(None)),
            authenticate_result: Arc::new(Mutex::new(None)),
            create_count: Arc::new(Mutex::new(0)),
            authenticate_count: Arc::new(Mutex::new(0)),
            discover_count: Arc::new(Mutex::new(0)),
            presence_results: Arc::new(Mutex::new(VecDeque::new())),
            authenticated_credential_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockPasskeyProviderImpl {
    pub(crate) fn reset(&self) {
        self.discover_results.lock().clear();
        *self.create_result.lock() = None;
        *self.authenticate_result.lock() = None;
        *self.create_count.lock() = 0;
        *self.authenticate_count.lock() = 0;
        *self.discover_count.lock() = 0;
        self.presence_results.lock().clear();
        self.authenticated_credential_ids.lock().clear();
    }

    pub(crate) fn set_discover_result(
        &self,
        result: Result<DiscoveredPasskeyResult, PasskeyError>,
    ) {
        let mut results = self.discover_results.lock();
        results.clear();
        results.push_back(result.map(|value| (value.prf_output, value.credential_id)));
    }

    pub(crate) fn push_discover_result(
        &self,
        result: Result<DiscoveredPasskeyResult, PasskeyError>,
    ) {
        self.discover_results
            .lock()
            .push_back(result.map(|value| (value.prf_output, value.credential_id)));
    }

    pub(crate) fn set_create_result(&self, result: Result<Vec<u8>, PasskeyError>) {
        *self.create_result.lock() = Some(result.map(|credential_id| PasskeyRegistrationResult {
            credential_id,
            provider_aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
            registered_platform: PasskeyRegistrationPlatform::Android,
        }));
    }

    pub(crate) fn set_authenticate_result(&self, result: Result<Vec<u8>, PasskeyError>) {
        *self.authenticate_result.lock() = Some(result);
    }

    pub(crate) fn authenticate_count(&self) -> usize {
        *self.authenticate_count.lock()
    }

    pub(crate) fn create_count(&self) -> usize {
        *self.create_count.lock()
    }

    pub(crate) fn discover_count(&self) -> usize {
        *self.discover_count.lock()
    }

    pub(crate) fn authenticated_credential_ids(&self) -> Vec<Vec<u8>> {
        self.authenticated_credential_ids.lock().clone()
    }
}

impl PasskeyProvider for MockPasskeyProviderImpl {
    fn create_passkey(
        &self,
        _rp_id: String,
        _challenge: Vec<u8>,
        _user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError> {
        *self.create_count.lock() += 1;
        self.create_result.lock().take().unwrap_or_else(|| {
            Err(PasskeyError::RequestFailed {
                operation: PasskeyOperation::Registration,
                reason: PasskeyFailureReason::Unknown {
                    diagnostic_message: "unexpected create_passkey call".into(),
                },
            })
        })
    }

    fn authenticate_with_prf(
        &self,
        _rp_id: String,
        credential_id: Vec<u8>,
        _prf_salt: Vec<u8>,
        _challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        *self.authenticate_count.lock() += 1;
        self.authenticated_credential_ids.lock().push(credential_id);
        self.authenticate_result.lock().take().unwrap_or_else(|| {
            Err(PasskeyError::RequestFailed {
                operation: PasskeyOperation::AuthenticateAssertion,
                reason: PasskeyFailureReason::Unknown {
                    diagnostic_message: "unexpected authenticate_with_prf call".into(),
                },
            })
        })
    }

    fn discover_and_authenticate_with_prf(
        &self,
        _rp_id: String,
        _prf_salt: Vec<u8>,
        _challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError> {
        *self.discover_count.lock() += 1;
        self.discover_results
            .lock()
            .pop_front()
            .unwrap_or(Err(PasskeyError::NoCredentialFound))
            .map(|(prf_output, credential_id)| DiscoveredPasskeyResult {
                prf_output,
                credential_id,
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
        self.presence_results.lock().pop_front().unwrap_or(PasskeyCredentialPresence::Present)
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
        CONNECTIVITY_MANAGER.set_connection_state(true);
    }
}

pub(crate) fn ensure_cloud_backup_test_tokio_runtime() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("cloud-backup-test-tokio".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("create cloud backup test tokio runtime");

                let drive_runtime = tokio::runtime::Runtime::block_on;
                drive_runtime(&runtime, async move {
                    cove_tokio::init();
                    sender.send(()).expect("signal cloud backup test tokio runtime");
                    std::future::pending::<()>().await;
                });
            })
            .expect("spawn cloud backup test tokio runtime thread");
        receiver.recv().expect("wait for cloud backup test tokio runtime");
    });
}

pub(crate) fn test_globals() -> &'static TestGlobals {
    static GLOBALS: OnceLock<TestGlobals> = OnceLock::new();
    GLOBALS.get_or_init(|| {
        crate::database::test_support::init_test_database();
        ensure_cloud_backup_test_tokio_runtime();
        TestGlobals::init()
    })
}

pub(crate) struct SyncTestLock(&'static tokio::sync::Mutex<()>);

impl SyncTestLock {
    pub(crate) fn lock(&self) -> tokio::sync::MutexGuard<'static, ()> {
        self.0.blocking_lock()
    }
}

pub(crate) fn test_lock() -> &'static SyncTestLock {
    static LOCK: OnceLock<SyncTestLock> = OnceLock::new();
    LOCK.get_or_init(|| SyncTestLock(shared_test_lock()))
}

pub(crate) fn async_test_lock() -> &'static tokio::sync::Mutex<()> {
    shared_test_lock()
}

fn shared_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(tokio::sync::Mutex::default)
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
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
    let changed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            wallet_id,
            record_id,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
        ))
        .unwrap();
}

fn mutate_wallet_and_persist_dirty(wallet_id: WalletId) {
    let mut wallet = CloudBackupStore::global()
        .all_wallets()
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
    persist_failed_blob_state_with_issue(wallet_id, retryable, None);
}

pub(crate) fn persist_failed_blob_state_with_issue(
    wallet_id: WalletId,
    retryable: bool,
    issue: Option<CloudBlobFailureIssue>,
) {
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
    let failed_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            wallet_id,
            record_id,
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: Some("rev-1".into()),
                retryable,
                error: "failed".into(),
                issue,
                failed_at,
            }),
        ))
        .unwrap();
}

pub(crate) fn persist_uploading_blob_state(wallet_id: WalletId, started_at: u64) {
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            wallet_id,
            record_id,
            PersistedCloudBlobState::Uploading(CloudBlobUploadingState {
                revision_hash: "rev-1".into(),
                started_at,
            }),
        ))
        .unwrap();
}

pub(crate) fn reset_cloud_backup_test_state(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
) {
    reset_cloud_backup_test_state_with_hook(manager, globals, || {});
}

pub(crate) fn reset_cloud_backup_test_state_with_hook(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
    before_reconnect: impl FnOnce(),
) {
    ensure_cloud_backup_test_tokio_runtime();
    wait_for_cleanup_idle_for_test(manager);
    globals.reset();
    clear_local_wallets();
    let reset_manager = manager.clone();
    std::thread::spawn(move || reset_manager.debug_reset_cloud_backup_state())
        .join()
        .expect("reset cloud backup test state thread");
    let supervisor = manager.supervisor.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let _task = cove_tokio::task::spawn(async move {
        let result = call!(supervisor.clear_upload_runtime_state()).await;
        sender.send(result).expect("send clear upload runtime state result");
    });
    receiver
        .recv()
        .expect("receive clear upload runtime state result")
        .expect("clear upload runtime state");
    before_reconnect();
    CONNECTIVITY_MANAGER.set_connection_state(true);
}

fn wait_for_cleanup_idle_for_test(manager: &RustCloudBackupManager) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        let supervisor = manager.supervisor.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let _task = cove_tokio::task::spawn(async move {
            let result = call!(supervisor.cleanup_idle_for_test()).await;
            sender.send(result).expect("send cleanup idle result");
        });
        let idle =
            receiver.recv().expect("receive cleanup idle result").expect("check cleanup idle");
        if idle {
            return;
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    panic!("cleanup worker should become idle before resetting cloud backup test state");
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

pub(crate) async fn take_pending_enable_session_for_test(
    manager: &RustCloudBackupManager,
) -> Option<PendingEnableSession> {
    call!(manager.supervisor.take_pending_enable_session_for_test())
        .await
        .expect("take pending enable session")
}

pub(crate) async fn replace_pending_enable_session_for_test(
    manager: &RustCloudBackupManager,
    session: PendingEnableSession,
) {
    call!(manager.supervisor.replace_pending_enable_session_for_test(session))
        .await
        .expect("replace pending enable session");
}

pub(crate) async fn has_awaiting_saved_passkey_confirmation_for_test(
    manager: &RustCloudBackupManager,
) -> bool {
    call!(manager.supervisor.has_awaiting_saved_passkey_confirmation_for_test())
        .await
        .expect("check saved passkey confirmation session")
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
    CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(Some(wallet_count)),
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
    CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(Some(wallet_count)),
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();
}

pub(crate) fn persisted_enabled_cloud_backup_state(
    wallet_count: Option<u32>,
) -> PersistedCloudBackupState {
    PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
        passkey: PersistedPasskeyState::Available,
        verification: PersistedBackupVerificationState::NotVerified {
            requested_at: None,
            dismissed_at: None,
        },
        sync: PersistedBackupSyncState { last_sync: None, wallet_count },
        pending_verification_completion: None,
    })
}

pub(crate) fn persisted_passkey_missing_cloud_backup_state(
    wallet_count: Option<u32>,
) -> PersistedCloudBackupState {
    PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
        passkey: PersistedPasskeyState::Missing,
        verification: PersistedBackupVerificationState::NotVerified {
            requested_at: None,
            dismissed_at: None,
        },
        sync: PersistedBackupSyncState { last_sync: None, wallet_count },
        pending_verification_completion: None,
    })
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

pub(crate) async fn encrypted_wallet_backup_bytes(
    metadata: &WalletMetadata,
    master_key: &cove_cspp::master_key::MasterKey,
    revision_hash: &str,
    version: u32,
) -> Vec<u8> {
    let mut prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        metadata,
        metadata.wallet_mode,
    )
    .await
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
    CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();

    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(None),
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
    call!(manager.supervisor.clear_upload_runtime_state())
        .await
        .expect("clear upload runtime state");
}

pub(crate) async fn resume_wallet_uploads_from_persisted_state_for_test_async(
    manager: &RustCloudBackupManager,
) {
    manager.sync_persisted_state();
    call!(manager.supervisor.resume_wallet_uploads_from_persisted_state())
        .await
        .expect("resume wallet uploads from persisted state");
}

pub(crate) async fn verify_pending_uploads_once_for_test_async(
    manager: &RustCloudBackupManager,
) -> bool {
    !matches!(manager.verify_pending_uploads_once().await, PendingUploadVerificationStatus::Idle)
}

pub(crate) async fn run_wallet_upload_for_test_async(
    manager: &RustCloudBackupManager,
    wallet_id: WalletId,
) {
    call!(manager.supervisor.run_wallet_upload_inline_for_test(wallet_id))
        .await
        .expect("run wallet upload");
}

pub(crate) async fn new_restore_operation_for_test(
    manager: &RustCloudBackupManager,
) -> RestoreOperation {
    call!(manager.supervisor.new_restore_operation()).await.expect("create restore operation")
}
