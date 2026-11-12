use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cove_device::keychain::{Keychain, KeychainAccess, KeychainError};
use parking_lot::Mutex;

pub(crate) mod fake_node;
pub(crate) mod hardware_signing;
pub(crate) mod log_capture;
pub(crate) mod malformed_psbt;
pub(crate) mod payjoin;
pub(crate) mod tap_signer_emulator;
pub(crate) mod utxo_locks;
pub(crate) mod wallet_target;

use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};

/// The shortest deadline a test gives a condition to become true
///
/// CI and local machines run several heavy suites at once: simulator and emulator UI tests,
/// builds and other Rust suites. At load averages of 100 to 470, waits that finish in well under
/// a second on an idle machine have missed 1 to 10 second deadlines. A longer deadline on a wait
/// for something to become true cannot fail a correct test, it only makes a broken one fail later
///
/// Never use it for checks that something stays true or stays false for a while, or where the
/// deadline is itself the assertion, such as proving something happens before a retry timer fires
pub(crate) const LOADED_HOST_WAIT_FLOOR: Duration = Duration::from_secs(30);

/// The deadline for a wait until something becomes true
///
/// `expected` is how long the wait may take on an idle machine. It documents the caller's
/// expectation and only takes effect when it is longer than [`LOADED_HOST_WAIT_FLOOR`]
pub(crate) fn loaded_host_wait(expected: Duration) -> Duration {
    expected.max(LOADED_HOST_WAIT_FLOOR)
}

/// A wall-clock deadline for a hand-rolled poll until something becomes true
///
/// It reads the wall clock, not tokio's, because some of these polls run in paused-time tests
/// while the work they wait for runs in real time on the global runtime. A paused clock skips
/// ahead on every sleep, so a tokio deadline there only bounds the number of polls
pub(crate) struct LoadedHostDeadline(std::time::Instant);

impl LoadedHostDeadline {
    /// A deadline [`loaded_host_wait`] of `expected` from now
    pub(crate) fn after(expected: Duration) -> Self {
        Self(std::time::Instant::now() + loaded_host_wait(expected))
    }

    pub(crate) fn has_passed(&self) -> bool {
        std::time::Instant::now() >= self.0
    }
}

static FAIL_KEYCHAIN_DELETES: AtomicBool = AtomicBool::new(false);
type AfterSaveHook = Arc<dyn Fn(&str) + Send + Sync>;

pub(crate) const WALLET_MNEMONIC_KEY_SUFFIX: &str = "::wallet_mnemonic";
pub(crate) const WALLET_MNEMONIC_CRYPTOR_KEY_SUFFIX: &str =
    "::wallet_mnemonic_encryption_key_and_nonce";
pub(crate) const WALLET_XPUB_KEY_SUFFIX: &str = "::wallet_xpub";

pub(crate) const WALLET_KEYCHAIN_KEY_SUFFIXES: [&str; 6] = [
    WALLET_MNEMONIC_KEY_SUFFIX,
    WALLET_MNEMONIC_CRYPTOR_KEY_SUFFIX,
    WALLET_XPUB_KEY_SUFFIX,
    "::wallet_public_descriptor",
    "::tap_signer_backup",
    "::wallet_tap_signer_encryption_key_and_nonce_key_name",
];

/// The raw stored wallet keychain values keyed by suffix
///
/// Comparing raw values makes a rewrite of an adopted item visible even when the
/// decrypted value would still compare equal
pub(crate) fn raw_wallet_keychain_entries(
    wallet_id: &WalletId,
) -> BTreeMap<&'static str, Option<String>> {
    let keychain = shared_mock_keychain();

    WALLET_KEYCHAIN_KEY_SUFFIXES
        .iter()
        .map(|suffix| (*suffix, keychain.get_entry(&format!("{wallet_id}{suffix}"))))
        .collect()
}

/// In-memory keychain shared by every test module
///
/// The process-global [`Keychain`] can only be installed once, and which test
/// module's init wins that race is nondeterministic. Sharing one clonable
/// instance keeps entry inspection and failure injection working no matter
/// which module installs it
#[derive(Default, Clone)]
pub(crate) struct MockKeychain {
    entries: Arc<Mutex<HashMap<String, String>>>,
    fail_save_at: Arc<Mutex<Option<usize>>>,
    fail_delete_at: Arc<Mutex<Option<usize>>>,
    save_count: Arc<Mutex<usize>>,
    delete_count: Arc<Mutex<usize>>,
    after_save: Arc<Mutex<Option<AfterSaveHook>>>,
    unreadable_keys: Arc<Mutex<HashSet<String>>>,
    recorded_reads: Arc<Mutex<Option<Vec<String>>>>,
}

// the after-save hook is a closure, which has no Debug impl
impl std::fmt::Debug for MockKeychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockKeychain")
            .field("entries", &self.entries)
            .field("fail_save_at", &self.fail_save_at)
            .field("fail_delete_at", &self.fail_delete_at)
            .field("save_count", &self.save_count)
            .field("delete_count", &self.delete_count)
            .field("unreadable_keys", &self.unreadable_keys)
            .field("recorded_reads", &self.recorded_reads)
            .finish_non_exhaustive()
    }
}

impl MockKeychain {
    pub(crate) fn reset(&self) {
        self.entries.lock().clear();
        *self.fail_save_at.lock() = None;
        *self.fail_delete_at.lock() = None;
        *self.save_count.lock() = 0;
        *self.delete_count.lock() = 0;
        *self.after_save.lock() = None;
        self.unreadable_keys.lock().clear();
        *self.recorded_reads.lock() = None;
    }

    /// Starts recording the key of every read, dropping any earlier recording
    pub(crate) fn record_reads(&self) {
        *self.recorded_reads.lock() = Some(Vec::new());
    }

    /// Stops recording reads and returns the keys read since [`Self::record_reads`]
    pub(crate) fn take_recorded_reads(&self) -> Vec<String> {
        self.recorded_reads.lock().take().unwrap_or_default()
    }

    /// Makes reads of `key` fail the way a locked platform keychain does
    pub(crate) fn fail_reads_for(&self, key: impl Into<String>) {
        self.unreadable_keys.lock().insert(key.into());
    }

    pub(crate) fn clear_read_failures(&self) {
        self.unreadable_keys.lock().clear();
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

    pub(crate) fn set_after_save(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
        *self.after_save.lock() = Some(Arc::new(hook));
    }
}

impl KeychainAccess for MockKeychain {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        {
            let mut save_count = self.save_count.lock();
            *save_count += 1;
            if Some(*save_count) == *self.fail_save_at.lock() {
                return Err(KeychainError::Save);
            }
        }

        self.entries.lock().insert(key.clone(), value);

        let hook = self.after_save.lock().clone();
        if let Some(hook) = hook {
            hook(&key);
        }

        Ok(())
    }

    fn get(&self, key: String) -> Result<Option<String>, KeychainError> {
        if let Some(reads) = self.recorded_reads.lock().as_mut() {
            reads.push(key.clone());
        }

        if self.unreadable_keys.lock().contains(&key) {
            return Err(KeychainError::Read(format!("{key} is unreadable")));
        }

        Ok(self.entries.lock().get(&key).cloned())
    }

    fn delete(&self, key: String) -> bool {
        if FAIL_KEYCHAIN_DELETES.load(Ordering::Relaxed) {
            return false;
        }

        let mut delete_count = self.delete_count.lock();
        *delete_count += 1;
        if Some(*delete_count) == *self.fail_delete_at.lock() {
            return false;
        }

        self.entries.lock().remove(&key).is_some()
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.entries.lock().retain(|key, _| {
            !WALLET_KEYCHAIN_KEY_SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
        });
        Ok(())
    }
}

pub(crate) fn hot_wallet_metadata(name: &str) -> WalletMetadata {
    let mut metadata = WalletMetadata::preview_new();
    metadata.name = name.to_string();
    metadata.wallet_type = WalletType::Hot;
    metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(
        bdk_wallet::bitcoin::bip32::Fingerprint::from_str("817e7be0").unwrap(),
    )));

    metadata
}

/// The single [`MockKeychain`] instance behind the process-global keychain
pub(crate) fn shared_mock_keychain() -> &'static MockKeychain {
    static KEYCHAIN: OnceLock<MockKeychain> = OnceLock::new();

    KEYCHAIN.get_or_init(MockKeychain::default)
}

/// Installs the shared [`MockKeychain`] as the process-global keychain
pub(crate) fn init_test_keychain() {
    static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let _ = Keychain::new(Box::new(shared_mock_keychain().clone()));
    });
}

/// Time zone of the process-global test device
///
/// A zone west of UTC makes evening confirmations fall on the next UTC day, so date code
/// that skips the local conversion shows up as an off-by-one-day mismatch
pub(crate) const TEST_DEVICE_TIME_ZONE: &str = "America/Chicago";

#[derive(Debug)]
struct TestDevice;

impl cove_device::device::DeviceAccess for TestDevice {
    fn timezone(&self) -> String {
        TEST_DEVICE_TIME_ZONE.to_string()
    }
}

/// Installs a process-global device that reports [`TEST_DEVICE_TIME_ZONE`]
pub(crate) fn init_test_device() {
    static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let _ = cove_device::device::Device::new(Box::new(TestDevice));
    });
}

/// Makes every keychain delete fail while set, for error-path tests
pub(crate) fn set_fail_keychain_deletes(fail: bool) {
    FAIL_KEYCHAIN_DELETES.store(fail, Ordering::Relaxed);
}

/// Serializes tests that mutate process-wide database or global config state
pub(crate) fn global_state_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    LOCK.get_or_init(tokio::sync::Mutex::default)
}

/// Starts a process-long Tokio runtime for tests that need the shared cove_tokio handle
pub(crate) fn ensure_tokio_runtime() {
    static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        std::thread::Builder::new()
            .name("cove-test-tokio".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("create cove test tokio runtime");

                runtime.block_on(async move {
                    cove_tokio::init();
                    sender.send(()).expect("signal cove test tokio runtime");
                    std::future::pending::<()>().await;
                });
            })
            .expect("spawn cove test tokio runtime thread");

        receiver.recv().expect("wait for cove test tokio runtime");
    });
}
