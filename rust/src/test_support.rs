use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use cove_device::keychain::{Keychain, KeychainAccess, KeychainError};
use parking_lot::Mutex;

static FAIL_KEYCHAIN_DELETES: AtomicBool = AtomicBool::new(false);

/// In-memory keychain shared by every test module
///
/// The process-global [`Keychain`] can only be installed once, and which test
/// module's init wins that race is nondeterministic. Sharing one clonable
/// instance keeps entry inspection and failure injection working no matter
/// which module installs it
#[derive(Debug, Default, Clone)]
pub(crate) struct MockKeychain {
    entries: Arc<Mutex<HashMap<String, String>>>,
    fail_save_at: Arc<Mutex<Option<usize>>>,
    fail_delete_at: Arc<Mutex<Option<usize>>>,
    save_count: Arc<Mutex<usize>>,
    delete_count: Arc<Mutex<usize>>,
}

impl MockKeychain {
    pub(crate) fn reset(&self) {
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
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        let mut save_count = self.save_count.lock();
        *save_count += 1;
        if Some(*save_count) == *self.fail_save_at.lock() {
            return Err(KeychainError::Save);
        }

        self.entries.lock().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.entries.lock().get(&key).cloned()
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
        let suffixes = [
            "::wallet_mnemonic",
            "::wallet_mnemonic_encryption_key_and_nonce",
            "::wallet_xpub",
            "::wallet_public_descriptor",
            "::tap_signer_backup",
            "::wallet_tap_signer_encryption_key_and_nonce_key_name",
        ];
        self.entries.lock().retain(|key, _| !suffixes.iter().any(|suffix| key.ends_with(suffix)));
        Ok(())
    }
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
