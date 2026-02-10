use cove_util::encryption::Cryptor;
use rand::Rng as _;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::keychain::{KeychainAccess, KeychainError};

const MASTER_KEY_NAME: &str = "cspp::v1::master_key";
const MASTER_KEY_ENCRYPTION_KEY_AND_NONCE: &str = "cspp::v1::master_key_encryption_key_and_nonce";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Generate a new random master key
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Store the master key encrypted in the keychain
    pub fn store(&self, keychain: &dyn KeychainAccess) -> Result<(), KeychainError> {
        let hex = hex::encode(self.0);
        let cryptor = Cryptor::new();

        let encrypted = cryptor
            .encrypt_to_string(&hex)
            .map_err(|error| KeychainError::Encrypt(error.to_string()))?;

        let encryption_key = cryptor.serialize_to_string();

        keychain.save(MASTER_KEY_NAME.into(), encrypted)?;
        keychain.save(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into(), encryption_key)?;

        Ok(())
    }

    /// Load the master key from the keychain, returns None if not found
    pub fn load(keychain: &dyn KeychainAccess) -> Result<Option<Self>, KeychainError> {
        let Some(encryption_secret) = keychain.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into())
        else {
            return Ok(None);
        };

        let Some(encrypted) = keychain.get(MASTER_KEY_NAME.into()) else {
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(&encryption_secret)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let hex = cryptor
            .decrypt_from_string(&encrypted)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err(|error| KeychainError::ParseSavedValue(error.to_string()))?
            .try_into()
            .map_err(|_| KeychainError::ParseSavedValue("master key not 32 bytes".into()))?;

        Ok(Some(Self(bytes)))
    }

    /// Load the master key from the keychain, or generate and store a new one
    pub fn load_or_generate(keychain: &dyn KeychainAccess) -> Result<Self, KeychainError> {
        if let Some(key) = Self::load(keychain)? {
            return Ok(key);
        }

        let key = Self::generate();
        key.store(keychain)?;
        Ok(key)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug)]
    struct MockKeychain(Mutex<HashMap<String, String>>);

    impl MockKeychain {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
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
    }

    #[test]
    fn generate_produces_32_bytes() {
        let key = MasterKey::generate();
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn store_and_load_roundtrip() {
        let keychain = MockKeychain::new();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();

        original.store(&keychain).unwrap();
        let loaded = MasterKey::load(&keychain).unwrap().unwrap();

        assert_eq!(*loaded.as_bytes(), original_bytes);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let keychain = MockKeychain::new();
        let loaded = MasterKey::load(&keychain).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn load_or_generate_creates_when_missing() {
        let keychain = MockKeychain::new();
        let first = MasterKey::load_or_generate(&keychain).unwrap();
        let first_bytes = *first.as_bytes();

        let second = MasterKey::load_or_generate(&keychain).unwrap();
        assert_eq!(*second.as_bytes(), first_bytes);
    }

    #[test]
    fn load_or_generate_reuses_existing() {
        let keychain = MockKeychain::new();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();
        original.store(&keychain).unwrap();

        let loaded = MasterKey::load_or_generate(&keychain).unwrap();
        assert_eq!(*loaded.as_bytes(), original_bytes);
    }
}
use cove_util::encryption::Cryptor;
use rand::Rng as _;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::keychain::{KeychainAccess, KeychainError};

const MASTER_KEY_NAME: &str = "cspp::v1::master_key";
const MASTER_KEY_ENCRYPTION_KEY_AND_NONCE: &str = "cspp::v1::master_key_encryption_key_and_nonce";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Generate a new random master key
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Store the master key encrypted in the keychain
    pub fn store(&self, keychain: &dyn KeychainAccess) -> Result<(), KeychainError> {
        let hex = hex::encode(self.0);
        let cryptor = Cryptor::new();

        let encrypted = cryptor
            .encrypt_to_string(&hex)
            .map_err(|error| KeychainError::Encrypt(error.to_string()))?;

        let encryption_key = cryptor.serialize_to_string();

        keychain.save(MASTER_KEY_NAME.into(), encrypted)?;
        keychain.save(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into(), encryption_key)?;

        Ok(())
    }

    /// Load the master key from the keychain, returns None if not found
    pub fn load(keychain: &dyn KeychainAccess) -> Result<Option<Self>, KeychainError> {
        let Some(encryption_secret) = keychain.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into())
        else {
            return Ok(None);
        };

        let Some(encrypted) = keychain.get(MASTER_KEY_NAME.into()) else {
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(&encryption_secret)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let hex = cryptor
            .decrypt_from_string(&encrypted)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err(|error| KeychainError::ParseSavedValue(error.to_string()))?
            .try_into()
            .map_err(|_| KeychainError::ParseSavedValue("master key not 32 bytes".into()))?;

        Ok(Some(Self(bytes)))
    }

    /// Load the master key from the keychain, or generate and store a new one
    pub fn load_or_generate(keychain: &dyn KeychainAccess) -> Result<Self, KeychainError> {
        if let Some(key) = Self::load(keychain)? {
            return Ok(key);
        }

        let key = Self::generate();
        key.store(keychain)?;
        Ok(key)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive the sensitive data encryption key (used for redb encryption)
    pub fn sensitive_data_key(&self) -> [u8; 32] {
        cove_util::key_derivation::derive_sensitive_data_key(&self.0)
    }

    /// Derive the critical data encryption key
    pub fn critical_data_key(&self) -> [u8; 32] {
        cove_util::key_derivation::derive_critical_data_key(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug)]
    struct MockKeychain(Mutex<HashMap<String, String>>);

    impl MockKeychain {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
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
    }

    #[test]
    fn generate_produces_32_bytes() {
        let key = MasterKey::generate();
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn store_and_load_roundtrip() {
        let keychain = MockKeychain::new();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();

        original.store(&keychain).unwrap();
        let loaded = MasterKey::load(&keychain).unwrap().unwrap();

        assert_eq!(*loaded.as_bytes(), original_bytes);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let keychain = MockKeychain::new();
        let loaded = MasterKey::load(&keychain).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn load_or_generate_creates_when_missing() {
        let keychain = MockKeychain::new();
        let first = MasterKey::load_or_generate(&keychain).unwrap();
        let first_bytes = *first.as_bytes();

        let second = MasterKey::load_or_generate(&keychain).unwrap();
        assert_eq!(*second.as_bytes(), first_bytes);
    }

    #[test]
    fn load_or_generate_reuses_existing() {
        let keychain = MockKeychain::new();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();
        original.store(&keychain).unwrap();

        let loaded = MasterKey::load_or_generate(&keychain).unwrap();
        assert_eq!(*loaded.as_bytes(), original_bytes);
    }

    #[test]
    fn sensitive_data_key_derivation() {
        let key = MasterKey::generate();
        let derived1 = key.sensitive_data_key();
        let derived2 = key.sensitive_data_key();
        assert_eq!(derived1, derived2);
    }

    #[test]
    fn critical_data_key_derivation() {
        let key = MasterKey::generate();
        let derived1 = key.critical_data_key();
        let derived2 = key.critical_data_key();
        assert_eq!(derived1, derived2);
    }
}
