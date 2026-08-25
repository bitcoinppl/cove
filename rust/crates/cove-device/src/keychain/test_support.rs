use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use cove_types::WalletId;

use super::{Keychain, KeychainAccess, KeychainError};

#[derive(Debug, Default)]
pub(crate) struct MockKeychain(Mutex<HashMap<String, String>>);

impl MockKeychain {
    pub(crate) fn with_entries(entries: &[(&str, &str)]) -> Self {
        Self(Mutex::new(
            entries.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect(),
        ))
    }
}

impl KeychainAccess for MockKeychain {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        self.0.lock().unwrap().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.0.lock().unwrap().get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.0.lock().unwrap().remove(&key).is_some()
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.0.lock().unwrap().retain(|key, _| !is_wallet_item_key(key));
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FailingKeychain {
    entries: Arc<Mutex<HashMap<String, String>>>,
    failing_save_key: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Default)]
pub(crate) struct FailSecondSave(Mutex<(HashMap<String, String>, u8)>);

impl KeychainAccess for FailSecondSave {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        let mut state = self.0.lock().unwrap();
        state.1 += 1;
        if state.1 == 2 {
            return Err(KeychainError::Save);
        }

        state.0.insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.0.lock().unwrap().0.get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.0.lock().unwrap().0.remove(&key).is_some()
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.0.lock().unwrap().0.retain(|key, _| !is_wallet_item_key(key));
        Ok(())
    }
}

impl FailingKeychain {
    pub(crate) fn fail_save_for(&self, key: String) {
        *self.failing_save_key.lock().unwrap() = Some(key);
    }

    pub(crate) fn entry(&self, key: &str) -> Option<String> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        self.entries.lock().unwrap().keys().cloned().collect()
    }
}

impl KeychainAccess for FailingKeychain {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        if self.failing_save_key.lock().unwrap().as_ref() == Some(&key) {
            return Err(KeychainError::Save);
        }

        self.entries.lock().unwrap().insert(key, value);
        Ok(())
    }

    fn get(&self, key: String) -> Option<String> {
        self.entries.lock().unwrap().get(&key).cloned()
    }

    fn delete(&self, key: String) -> bool {
        self.entries.lock().unwrap().remove(&key).is_some()
    }

    fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.entries.lock().unwrap().retain(|key, _| !is_wallet_item_key(key));
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
