use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use cove_types::WalletId;

use super::{Keychain, KeychainAccess, KeychainError};

#[derive(Debug, Default)]
struct MockKeychainState {
    entries: HashMap<String, String>,
    failing_save_key: Option<String>,
    failing_save_attempt: Option<u8>,
    save_attempts: u8,
}

/// In-memory keychain whose state is shared between clones so tests can inspect it after use
#[derive(Clone, Debug, Default)]
pub(crate) struct MockKeychain(Arc<Mutex<MockKeychainState>>);

impl MockKeychain {
    pub(crate) fn with_entries(entries: &[(&str, &str)]) -> Self {
        let keychain = Self::default();
        keychain.0.lock().unwrap().entries =
            entries.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect();
        keychain
    }

    /// Fails the nth save call, counting from one
    pub(crate) fn failing_save_attempt(attempt: u8) -> Self {
        let keychain = Self::default();
        keychain.0.lock().unwrap().failing_save_attempt = Some(attempt);
        keychain
    }

    pub(crate) fn fail_save_for(&self, key: String) {
        self.0.lock().unwrap().failing_save_key = Some(key);
    }

    pub(crate) fn entry(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().entries.get(key).cloned()
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        self.0.lock().unwrap().entries.keys().cloned().collect()
    }
}

impl KeychainAccess for MockKeychain {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        let mut state = self.0.lock().unwrap();
        state.save_attempts += 1;
        if state.failing_save_attempt == Some(state.save_attempts) {
            return Err(KeychainError::Save);
        }
        if state.failing_save_key.as_ref() == Some(&key) {
            return Err(KeychainError::Save);
        }

        state.entries.insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.0.lock().unwrap().entries.get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.0.lock().unwrap().entries.remove(&key).is_some()
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.0.lock().unwrap().entries.retain(|key, _| !is_wallet_item_key(key));
        Ok(())
    }
}

fn is_wallet_item_key(key: &str) -> bool {
    [
        "::wallet_mnemonic",
        "::wallet_mnemonic_encryption_key_and_nonce",
        "::wallet_xpub",
        "::wallet_public_descriptor",
        "::tap_signer_backup",
        "::wallet_tap_signer_encryption_key_and_nonce_key_name",
    ]
    .iter()
    .any(|suffix| key.ends_with(suffix))
}

pub(crate) fn keychain(access: impl KeychainAccess) -> Keychain {
    Keychain::from_access(Arc::new(access))
}

pub(crate) fn wallet_id() -> WalletId {
    WalletId::preview_new()
}
