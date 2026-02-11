use cove_util::encryption::Cryptor;

use crate::error::CsppError;
use crate::master_key::MasterKey;
use crate::store::CsppStore;

const MASTER_KEY_NAME: &str = "cspp::v1::master_key";
const MASTER_KEY_ENCRYPTION_KEY_AND_NONCE: &str = "cspp::v1::master_key_encryption_key_and_nonce";

pub struct Cspp<S: CsppStore>(S);

impl<S: CsppStore> Cspp<S> {
    pub fn new(store: S) -> Self {
        Self(store)
    }

    /// Saves the master key encrypted via the underlying store
    pub fn save_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let hex = hex::encode(master_key.as_bytes());
        let cryptor = Cryptor::new();

        let encrypted =
            cryptor.encrypt_to_string(&hex).map_err(|e| CsppError::Encrypt(e.to_string()))?;

        let encryption_key = cryptor.serialize_to_string();

        self.0
            .save(MASTER_KEY_NAME.into(), encrypted)
            .map_err(|e| CsppError::Save(e.to_string()))?;

        self.0
            .save(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into(), encryption_key)
            .map_err(|e| CsppError::Save(e.to_string()))?;

        Ok(())
    }

    /// Loads the master key from the store, returns None if not found
    pub fn get_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let Some(encryption_secret) = self.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into()) else {
            return Ok(None);
        };

        let Some(encrypted) = self.0.get(MASTER_KEY_NAME.into()) else {
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(&encryption_secret)
            .map_err(|e| CsppError::Decrypt(e.to_string()))?;

        let hex = cryptor
            .decrypt_from_string(&encrypted)
            .map_err(|e| CsppError::Decrypt(e.to_string()))?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err(|e| CsppError::InvalidData(e.to_string()))?
            .try_into()
            .map_err(|_| CsppError::InvalidData("master key not 32 bytes".into()))?;

        Ok(Some(MasterKey::from_bytes(bytes)))
    }

    /// Deletes the master key from the store
    pub fn delete_master_key(&self) -> bool {
        self.0.delete(MASTER_KEY_NAME.into());
        self.0.delete(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into())
    }

    /// Loads the master key from the store, or generates and saves a new one
    pub fn get_or_create_master_key(&self) -> Result<MasterKey, CsppError> {
        if let Some(key) = self.get_master_key()? {
            return Ok(key);
        }

        let key = MasterKey::generate();
        self.save_master_key(&key)?;
        Ok(key)
    }

    /// Load-or-create master key, then derive the sensitive data encryption key
    pub fn sensitive_data_key(&self) -> Result<[u8; 32], CsppError> {
        let master_key = self.get_or_create_master_key()?;
        Ok(master_key.sensitive_data_key())
    }

    /// Load-or-create master key, then derive the critical data encryption key
    pub fn critical_data_key(&self) -> Result<[u8; 32], CsppError> {
        let master_key = self.get_or_create_master_key()?;
        Ok(master_key.critical_data_key())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug)]
    struct MockStore(Mutex<HashMap<String, String>>);

    impl MockStore {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }
    }

    impl CsppStore for MockStore {
        type Error = String;

        fn save(&self, key: String, value: String) -> Result<(), String> {
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

    fn mock_cspp() -> Cspp<MockStore> {
        Cspp::new(MockStore::new())
    }

    #[test]
    fn master_key_store_and_load_roundtrip() {
        let cspp = mock_cspp();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();

        cspp.save_master_key(&original).unwrap();
        let loaded = cspp.get_master_key().unwrap().unwrap();

        assert_eq!(*loaded.as_bytes(), original_bytes);
    }

    #[test]
    fn master_key_load_returns_none_when_missing() {
        let cspp = mock_cspp();
        let loaded = cspp.get_master_key().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn get_or_create_creates_when_missing() {
        let cspp = mock_cspp();
        let first = cspp.get_or_create_master_key().unwrap();
        let first_bytes = *first.as_bytes();

        let second = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*second.as_bytes(), first_bytes);
    }

    #[test]
    fn get_or_create_reuses_existing() {
        let cspp = mock_cspp();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();
        cspp.save_master_key(&original).unwrap();

        let loaded = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*loaded.as_bytes(), original_bytes);
    }

    #[test]
    fn sensitive_data_key_is_deterministic() {
        let cspp = mock_cspp();
        let key1 = cspp.sensitive_data_key().unwrap();
        let key2 = cspp.sensitive_data_key().unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn critical_data_key_is_deterministic() {
        let cspp = mock_cspp();
        let key1 = cspp.critical_data_key().unwrap();
        let key2 = cspp.critical_data_key().unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn sensitive_and_critical_keys_differ() {
        let cspp = mock_cspp();
        let sensitive = cspp.sensitive_data_key().unwrap();
        let critical = cspp.critical_data_key().unwrap();
        assert_ne!(sensitive, critical);
    }
}
