//! Provider fakes controlled by call identity
//!
//! Every provider call gets a [`CallId`]: its kind and its 1-based position among calls of that
//! kind in this process. Tests script a fault for an exact `CallId`, and a [`Hold`] reports when
//! the call entered, waits for the test to release it, and reports when it finished. Nothing in
//! the harness waits by sleeping or yielding

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use cove_device::cloud_storage::{
    CloudAccessPolicy, CloudBackupUploadStatus, CloudStorageAccess, CloudStorageError,
    CloudStorageInventorySnapshot, CloudSyncHealth, RemoteBackupLocation,
};
use cove_device::keychain::{KeychainAccess, KeychainError};
use cove_device::passkey::{
    DiscoveredPasskeyResult, PasskeyCredentialPresence, PasskeyError, PasskeyFailureReason,
    PasskeyOperation, PasskeyProvider, PasskeyRegistrationResult, PasskeyRegistrationUser,
};
use parking_lot::Mutex;
use tokio::sync::watch;

use super::deadline::Deadline;
use super::process::{checkpoint, on_checkpoint};
use crate::manager::cloud_backup_manager::keychain::CSPP_NAMESPACE_ID_KEY;
use crate::manager::cloud_backup_manager::ops::test_support::{
    MockCloudDurableContents, MockCloudStorage, MockPasskeyProviderImpl,
};
use crate::test_support::MockKeychain;

/// A provider call's stable identity within one process
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CallId<Kind> {
    pub(crate) kind: Kind,
    /// 1-based position among calls of this kind
    pub(crate) nth: usize,
}

impl<Kind> CallId<Kind> {
    pub(crate) fn new(kind: Kind, nth: usize) -> Self {
        Self { kind, nth }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldStage {
    Armed,
    Entered,
    Released,
    Finished,
}

/// Holds one provider call and acknowledges each step of it
///
/// Every wait on a hold is bounded by the harness [`Deadline`], so a call that never starts, or a
/// hold the test never releases, fails the phase with the held call's name
#[derive(Debug, Clone)]
pub(crate) struct Hold(Arc<HoldInner>);

#[derive(Debug)]
struct HoldInner {
    stage: watch::Sender<HoldStage>,
    call: Mutex<String>,
}

impl Default for Hold {
    fn default() -> Self {
        Self(Arc::new(HoldInner {
            stage: watch::Sender::new(HoldStage::Armed),
            call: Mutex::new("an unscripted held call".into()),
        }))
    }
}

impl Hold {
    /// Resolves once the held call has started and is waiting for [`Hold::release`]
    pub(crate) async fn entered(&self) {
        self.wait_for(HoldStage::Entered, "to start").await;
    }

    pub(crate) fn release(&self) {
        self.0.stage.send_modify(|stage| *stage = HoldStage::Released);
    }

    /// Resolves once the released call has returned to its caller
    pub(crate) async fn finished(&self) {
        self.wait_for(HoldStage::Finished, "to return after release").await;
    }

    /// Name the held call in deadline failures
    fn label(&self, call: String) {
        *self.0.call.lock() = call;
    }

    async fn wait_for(&self, stage: HoldStage, step: &str) {
        let _deadline = Deadline::arm(format!("{} {step}", self.0.call.lock()));
        let rank = |stage: &HoldStage| *stage as u8;
        let mut receiver = self.0.stage.subscribe();
        receiver.wait_for(|current| rank(current) >= rank(&stage)).await.expect("hold dropped");
    }

    async fn enter_and_wait(&self) {
        HELD_CALLS.send_modify(|held| *held += 1);

        // a hold the test released before its call started lets that call through: overwriting
        // the release would strand the call with nothing left to release it
        self.0.stage.send_if_modified(|stage| {
            let armed = *stage == HoldStage::Armed;
            if armed {
                *stage = HoldStage::Entered;
            }
            armed
        });
        self.wait_for(HoldStage::Released, "to be released by the test").await;
        HELD_CALLS.send_modify(|held| *held -= 1);
    }

    fn finish(&self) {
        self.0.stage.send_modify(|stage| *stage = HoldStage::Finished);
    }
}

/// Fake calls of any kind waiting on a [`Hold`]
static HELD_CALLS: LazyLock<watch::Sender<usize>> = LazyLock::new(|| watch::Sender::new(0));

/// How many fake calls are waiting for the test to release them
pub(crate) fn held_calls() -> usize {
    *HELD_CALLS.borrow()
}

/// Resolves once some fake call is waiting for the test to release it
pub(crate) async fn until_a_call_is_held() {
    let mut held = HELD_CALLS.subscribe();
    let _ = held.wait_for(|held| *held > 0).await;
}

fn write_json(path: &PathBuf, value: &impl serde::Serialize) {
    // rename makes the write atomic, so a SIGKILL never leaves a torn file
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec(value).expect("encode fake state"))
        .expect("write fake state");
    std::fs::rename(&temp, path).expect("commit fake state");
}

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> T {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).expect("decode fake state"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(error) => panic!("read fake state {path:?}: {error}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CloudCall {
    UploadMasterKey,
    UploadWallet,
    DownloadMasterKey,
    DownloadWallet,
    DeleteWallet,
    DeleteNamespace,
    ListNamespaces,
    ListWalletFiles,
    ListWalletFilesSnapshot,
    IsBackupUploaded,
    SyncHealth,
}

/// What a scripted cloud call does instead of, or after, the provider contents
#[derive(Debug, Clone)]
pub(crate) enum CloudFault {
    /// Fail without touching the provider
    Fail(CloudStorageError),
    /// Wait for the test to release the call, then run it
    Hold(Hold),
    /// Run the call, then lose the response
    SucceedThenFail(CloudStorageError),
    /// Delete these wallet records from the namespace, then fail
    PartialDeleteThenFail { removed: Vec<String>, error: CloudStorageError },
    /// Answer a listing with these wallet file names instead of the provider contents
    Listing(Vec<String>),
    /// Reach this crash checkpoint before running the call
    KillBefore(&'static str),
    /// Run the call, persist the result, then reach this crash checkpoint
    KillAfter(&'static str),
}

#[derive(Debug, Default)]
struct CloudCalls {
    counts: HashMap<CloudCall, usize>,
    log: Vec<(CallId<CloudCall>, Option<String>)>,
    scripts: HashMap<CallId<CloudCall>, CloudFault>,
    in_flight: usize,
}

fn label_hold(id: CallId<CloudCall>, fault: &CloudFault) {
    if let CloudFault::Hold(hold) = fault {
        hold.label(format!("held cloud call {:?} #{}", id.kind, id.nth));
    }
}

/// File-backed fake cloud that survives process death
#[derive(Debug, Clone)]
pub(crate) struct HarnessCloud {
    pub(crate) inner: MockCloudStorage,
    file: Arc<PathBuf>,
    calls: Arc<Mutex<CloudCalls>>,
}

impl HarnessCloud {
    pub(crate) fn load(inner: MockCloudStorage, file: PathBuf) -> Self {
        inner.restore_durable_contents(read_json::<MockCloudDurableContents>(&file));
        let cloud = Self { inner, file: Arc::new(file), calls: Arc::default() };
        let flushed = cloud.clone();
        on_checkpoint(move || flushed.persist());
        cloud
    }

    pub(crate) fn persist(&self) {
        write_json(&self.file, &self.inner.durable_contents());
    }

    /// Script the call with this identity
    pub(crate) fn script(&self, id: CallId<CloudCall>, fault: CloudFault) {
        label_hold(id, &fault);
        self.calls.lock().scripts.insert(id, fault);
    }

    /// Script the next call of `kind`, counted from now
    pub(crate) fn script_next(&self, kind: CloudCall, fault: CloudFault) -> CallId<CloudCall> {
        let mut calls = self.calls.lock();
        let id = CallId::new(kind, calls.counts.get(&kind).copied().unwrap_or_default() + 1);
        label_hold(id, &fault);
        calls.scripts.insert(id, fault);
        id
    }

    /// Every call so far with the namespace it targeted
    pub(crate) fn calls(&self) -> Vec<(CallId<CloudCall>, Option<String>)> {
        self.calls.lock().log.clone()
    }

    pub(crate) fn count(&self, kind: CloudCall) -> usize {
        self.calls.lock().counts.get(&kind).copied().unwrap_or_default()
    }

    /// Calls that started and have not returned, excluding ones waiting on a [`Hold`]
    pub(crate) fn running_calls(&self) -> usize {
        self.calls.lock().in_flight
    }

    async fn begin(
        &self,
        kind: CloudCall,
        namespace: Option<&str>,
    ) -> Result<Option<CloudFault>, CloudStorageError> {
        let fault = {
            let mut calls = self.calls.lock();
            let nth = calls.counts.entry(kind).or_default();
            *nth += 1;
            let id = CallId::new(kind, *nth);
            calls.log.push((id, namespace.map(str::to_string)));
            calls.scripts.remove(&id)
        };

        match &fault {
            Some(CloudFault::Fail(error)) => return Err(error.clone()),
            Some(CloudFault::KillBefore(name)) => checkpoint(name),
            Some(CloudFault::Hold(hold)) => hold.enter_and_wait().await,
            _ => {}
        }

        // a held call counts as running only once released, so settling can wait on a hold
        self.calls.lock().in_flight += 1;
        Ok(fault)
    }

    /// Ends a call that may have changed the provider contents, saving them first
    fn finish<T>(
        &self,
        fault: Option<CloudFault>,
        result: Result<T, CloudStorageError>,
    ) -> Result<T, CloudStorageError> {
        self.persist();
        self.answer(fault, result)
    }

    /// Ends a call that only read the provider contents
    ///
    /// Rewriting the whole file after every download made a refresh of a large account slow
    /// enough to trip the wait limit under a loaded test run; a kill checkpoint still saves
    fn answer<T>(
        &self,
        fault: Option<CloudFault>,
        result: Result<T, CloudStorageError>,
    ) -> Result<T, CloudStorageError> {
        self.calls.lock().in_flight -= 1;

        match fault {
            Some(CloudFault::SucceedThenFail(error)) if result.is_ok() => Err(error),
            Some(CloudFault::KillAfter(name)) => {
                checkpoint(name);
                result
            }

            Some(CloudFault::Hold(hold)) => {
                hold.finish();
                result
            }

            _ => result,
        }
    }

    fn remove_records(&self, namespace: &str, record_ids: &[String]) {
        let mut contents = self.inner.durable_contents();
        contents.wallet_backups.retain(|((ns, record), _)| {
            !(ns == namespace && record_ids.iter().any(|id| id == record))
        });

        if let Some(files) = contents.wallet_files.get_mut(namespace) {
            files.retain(|file| {
                cove_cspp::backup_data::wallet_record_id_from_filename(file)
                    .is_none_or(|record| !record_ids.iter().any(|id| id == record))
            });
        }

        self.inner.restore_durable_contents(contents);
    }

    fn listing_override(fault: &Option<CloudFault>) -> Option<Vec<String>> {
        match fault {
            Some(CloudFault::Listing(names)) => Some(names.clone()),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl CloudStorageAccess for HarnessCloud {
    async fn upload_master_key_backup(
        &self,
        namespace: String,
        location: RemoteBackupLocation,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let fault = self.begin(CloudCall::UploadMasterKey, Some(&namespace)).await?;
        let result = self.inner.upload_master_key_backup(namespace, location, data, policy).await;
        self.finish(fault, result)
    }

    async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        location: RemoteBackupLocation,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let fault = self.begin(CloudCall::UploadWallet, Some(&namespace)).await?;
        let result =
            self.inner.upload_wallet_backup(namespace, record_id, location, data, policy).await;
        self.finish(fault, result)
    }

    async fn download_master_key_backup(
        &self,
        namespace: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        let fault = self.begin(CloudCall::DownloadMasterKey, Some(&namespace)).await?;
        let result = self.inner.download_master_key_backup(namespace, locations, policy).await;
        self.answer(fault, result)
    }

    async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        let fault = self.begin(CloudCall::DownloadWallet, Some(&namespace)).await?;
        let result =
            self.inner.download_wallet_backup(namespace, record_id, locations, policy).await;
        self.answer(fault, result)
    }

    async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let fault = self.begin(CloudCall::DeleteWallet, Some(&namespace)).await?;
        let result = self.inner.delete_wallet_backup(namespace, record_id, locations, policy).await;
        self.finish(fault, result)
    }

    async fn delete_namespace(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        let fault = self.begin(CloudCall::DeleteNamespace, Some(&namespace)).await?;
        if let Some(CloudFault::PartialDeleteThenFail { removed, error }) = &fault {
            self.remove_records(&namespace, removed);
            return self.finish(None, Err(error.clone()));
        }

        let result = self.inner.delete_namespace(namespace, policy).await;
        self.finish(fault, result)
    }

    async fn list_namespaces(
        &self,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        let fault = self.begin(CloudCall::ListNamespaces, None).await?;
        let result = self.inner.list_namespaces(policy).await;
        self.answer(fault, result)
    }

    async fn list_wallet_files(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        let fault = self.begin(CloudCall::ListWalletFiles, Some(&namespace)).await?;
        let result = match Self::listing_override(&fault) {
            Some(names) => Ok(names),
            None => self.inner.list_wallet_files(namespace, policy).await,
        };

        self.answer(fault, result)
    }

    async fn list_wallet_files_snapshot(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<CloudStorageInventorySnapshot, CloudStorageError> {
        let fault = self.begin(CloudCall::ListWalletFilesSnapshot, Some(&namespace)).await?;
        let result = match Self::listing_override(&fault) {
            Some(names) => Ok(CloudStorageInventorySnapshot { names, is_complete: true }),
            None => self.inner.list_wallet_files_snapshot(namespace, policy).await,
        };

        self.answer(fault, result)
    }

    async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<CloudBackupUploadStatus, CloudStorageError> {
        let fault = self.begin(CloudCall::IsBackupUploaded, Some(&namespace)).await?;
        let result = self.inner.is_backup_uploaded(namespace, record_id, locations, policy).await;
        self.answer(fault, result)
    }

    async fn overall_sync_health(&self, policy: CloudAccessPolicy) -> CloudSyncHealth {
        match self.begin(CloudCall::SyncHealth, None).await {
            Ok(fault) => {
                let health = self.inner.overall_sync_health(policy).await;
                self.answer(fault, Ok(health)).expect("sync health has no error path")
            }

            // a scripted failure has no error channel here, so report the provider unavailable
            Err(error) => CloudSyncHealth::Failed(error.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PasskeyCall {
    Create,
    Authenticate,
    Discover,
}

/// How the platform passkey provider answers interactive requests
#[derive(Debug, Clone, Default)]
pub(crate) enum PasskeyMode {
    /// Answer with the scripted mock results
    #[default]
    Succeeds,
    /// The user dismissed the sheet; today's error is `UserCancelled`
    NotCompleted,
    /// The provider failed after presenting
    ProviderFailure,
    /// Wait for the test to release the request, then answer with the mock results
    Slow(Hold),
    /// Never answer
    #[expect(dead_code, reason = "reserved for bounded passkey request tests")]
    NeverCompletes,
    /// Any interactive request is a test failure, reported by [`HarnessPasskey::interactive_calls`]
    FailOnAnyInteractiveCall,
}

#[derive(Debug, Default)]
struct PasskeyCalls {
    counts: HashMap<PasskeyCall, usize>,
    log: Vec<CallId<PasskeyCall>>,
    forbidden: Vec<CallId<PasskeyCall>>,
    running: usize,
}

/// Passkey provider fake; the mock's results answer `Succeeds` and `Slow` requests
#[derive(Debug, Clone)]
pub(crate) struct HarnessPasskey {
    pub(crate) inner: MockPasskeyProviderImpl,
    mode: Arc<Mutex<PasskeyMode>>,
    calls: Arc<Mutex<PasskeyCalls>>,
}

impl HarnessPasskey {
    pub(crate) fn new(inner: MockPasskeyProviderImpl) -> Self {
        Self { inner, mode: Arc::default(), calls: Arc::default() }
    }

    pub(crate) fn set_mode(&self, mode: PasskeyMode) {
        *self.mode.lock() = mode;
    }

    pub(crate) fn interactive_calls(&self) -> Vec<CallId<PasskeyCall>> {
        self.calls.lock().log.clone()
    }

    /// Requests answering on a blocking thread, excluding held and never-answered ones
    pub(crate) fn running_calls(&self) -> usize {
        self.calls.lock().running
    }

    /// Interactive calls made while the mode was `FailOnAnyInteractiveCall`
    pub(crate) fn forbidden_calls(&self) -> Vec<CallId<PasskeyCall>> {
        self.calls.lock().forbidden.clone()
    }

    fn interactive<T>(
        &self,
        kind: PasskeyCall,
        operation: PasskeyOperation,
        answer: impl FnOnce() -> Result<T, PasskeyError>,
    ) -> Result<T, PasskeyError> {
        let id = {
            let mut calls = self.calls.lock();
            let nth = calls.counts.entry(kind).or_default();
            *nth += 1;
            let id = CallId::new(kind, *nth);
            calls.log.push(id);
            id
        };

        let mode = self.mode.lock().clone();

        // passkey requests run on blocking threads, so waiting here never stalls the runtime; a
        // held or never-answered request counts as settled, like a held cloud call
        match &mode {
            PasskeyMode::Slow(hold) => {
                hold.label(format!("held passkey call {:?} #{}", id.kind, id.nth));
                futures::executor::block_on(hold.enter_and_wait());
            }

            PasskeyMode::NeverCompletes => loop {
                std::thread::park();
            },

            _ => {}
        }

        // the runtime can go idle while this blocking thread answers, so settling waits on it
        self.calls.lock().running += 1;
        let result = match mode {
            PasskeyMode::Succeeds => answer(),
            PasskeyMode::NotCompleted => Err(PasskeyError::UserCancelled),
            PasskeyMode::ProviderFailure => Err(PasskeyError::RequestFailed {
                operation,
                reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
            }),

            PasskeyMode::Slow(hold) => {
                let result = answer();
                hold.finish();
                result
            }

            PasskeyMode::NeverCompletes => unreachable!("never-answered requests park above"),
            PasskeyMode::FailOnAnyInteractiveCall => {
                self.calls.lock().forbidden.push(id);
                Err(PasskeyError::UserCancelled)
            }
        };

        self.calls.lock().running -= 1;
        result
    }
}

impl PasskeyProvider for HarnessPasskey {
    fn create_passkey(
        &self,
        rp_id: String,
        challenge: Vec<u8>,
        user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError> {
        self.interactive(PasskeyCall::Create, PasskeyOperation::Registration, || {
            self.inner.create_passkey(rp_id, challenge, user)
        })
    }

    fn authenticate_with_prf(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.interactive(PasskeyCall::Authenticate, PasskeyOperation::AuthenticateAssertion, || {
            self.inner.authenticate_with_prf(rp_id, credential_id, prf_salt, challenge)
        })
    }

    fn discover_and_authenticate_with_prf(
        &self,
        rp_id: String,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError> {
        self.interactive(PasskeyCall::Discover, PasskeyOperation::DiscoverAssertion, || {
            self.inner.discover_and_authenticate_with_prf(rp_id, prf_salt, challenge)
        })
    }

    fn is_prf_supported(&self) -> bool {
        self.inner.is_prf_supported()
    }

    fn check_passkey_presence(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
    ) -> PasskeyCredentialPresence {
        self.inner.check_passkey_presence(rp_id, credential_id)
    }
}

/// File-backed keychain that survives process death
#[derive(Debug, Clone)]
pub(crate) struct HarnessKeychain {
    pub(crate) inner: MockKeychain,
    file: Arc<PathBuf>,
    /// How long each delete takes, like a platform keychain under load
    slow_deletes: Arc<Mutex<Option<Duration>>>,
    /// Holds the next delete of the namespace item until the test releases it
    namespace_delete_hold: Arc<Mutex<Option<Hold>>>,
}

impl HarnessKeychain {
    pub(crate) fn load(inner: MockKeychain, file: PathBuf) -> Self {
        inner.restore_entries(read_json(&file));
        let keychain = Self {
            inner,
            file: Arc::new(file),
            slow_deletes: Arc::default(),
            namespace_delete_hold: Arc::default(),
        };
        let flushed = keychain.clone();
        on_checkpoint(move || flushed.persist());
        keychain
    }

    pub(crate) fn persist(&self) {
        write_json(&self.file, &self.inner.entries_snapshot());
    }

    /// Makes every later delete take `duration` on its own thread, without a hold the test
    /// releases, so the only way to wait for it is the owner's stage checkpoint
    pub(crate) fn slow_deletes(&self, duration: Duration) {
        *self.slow_deletes.lock() = Some(duration);
    }

    /// Holds the next delete of the namespace item, which only clearing local state makes
    pub(crate) fn hold_namespace_delete(&self, hold: Hold) {
        hold.label("held keychain namespace delete".into());
        *self.namespace_delete_hold.lock() = Some(hold);
    }
}

impl KeychainAccess for HarnessKeychain {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        let result = self.inner.save(key, value);
        self.persist();
        result
    }

    fn get(&self, key: String) -> Result<Option<String>, KeychainError> {
        self.inner.get(key)
    }

    fn delete(&self, key: String) -> bool {
        if let Some(duration) = *self.slow_deletes.lock() {
            std::thread::sleep(duration);
        }

        let hold = (key == CSPP_NAMESPACE_ID_KEY)
            .then(|| self.namespace_delete_hold.lock().take())
            .flatten();
        if let Some(hold) = hold {
            futures::executor::block_on(hold.enter_and_wait());
            let deleted = self.inner.delete(key);
            self.persist();
            hold.finish();
            return deleted;
        }

        let deleted = self.inner.delete(key);
        self.persist();
        deleted
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        let result = self.inner.delete_all_wallet_items();
        self.persist();
        result
    }
}
