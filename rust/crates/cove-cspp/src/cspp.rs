use std::sync::{Arc, LazyLock, Mutex};

use arc_swap::ArcSwapOption;
use cove_util::encryption::Cryptor;
use zeroize::Zeroizing;

use crate::error::CsppError;
use crate::master_key::MasterKey;
use crate::store::CsppStore;

const MASTER_KEY_NAME: &str = "cspp::v1::master_key";
const MASTER_KEY_ENCRYPTION_KEY_AND_NONCE: &str = "cspp::v1::master_key_encryption_key_and_nonce";

static INIT_LOCK: Mutex<()> = Mutex::new(());
static MASTER_KEY_CACHE: LazyLock<ArcSwapOption<Zeroizing<[u8; 32]>>> =
    LazyLock::new(|| ArcSwapOption::new(None));

pub struct Cspp<S: CsppStore>(S);

impl<S: CsppStore> Cspp<S> {
    pub fn new(store: S) -> Self {
        Self(store)
    }

    /// Loads the master key from the store, or generates and saves a new one
    ///
    /// Uses double-checked locking to prevent a TOCTOU race where two threads
    /// could both observe no key and generate different master keys
    pub fn get_or_create_master_key(&self) -> Result<MasterKey, CsppError> {
        // fast path (lock-free): return cached key
        if let Some(bytes) = MASTER_KEY_CACHE.load().as_deref() {
            return Ok(MasterKey::from_bytes(**bytes));
        }

        // slow path: acquire init lock for double-checked initialization
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // re-check cache after acquiring lock
        if let Some(bytes) = MASTER_KEY_CACHE.load().as_deref() {
            return Ok(MasterKey::from_bytes(**bytes));
        }

        // try loading from store
        if let Some(key) = self.get_master_key()? {
            MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*key.as_bytes()))));
            return Ok(key);
        }

        // generate and save new key
        let key = MasterKey::generate();
        self.save_master_key(&key)?;
        MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*key.as_bytes()))));

        Ok(key)
    }

    /// Deletes the master key from the store
    pub fn delete_master_key(&self) -> bool {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MASTER_KEY_CACHE.store(None);

        // delete encrypted data before its encryption key (reverse of save order)
        // so a partial failure never leaves orphaned data without a decryption key,
        // and code that checks for the key's existence won't see stale encrypted
        // data after the decryption key has already been removed
        self.0.delete(MASTER_KEY_NAME.into());
        self.0.delete(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into())
    }

    /// Saves the master key encrypted via the underlying store
    ///
    /// The master key is encrypted with a random [`Cryptor`] before storage. The
    /// keychain already provides at-rest encryption, but this extra layer prevents
    /// the plaintext key from being accidentally exposed if other code enumerates
    /// keychain entries — it must be explicitly decrypted to be read
    fn save_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let hex = hex::encode(master_key.as_bytes());
        let cryptor = Cryptor::new();

        let encrypted =
            cryptor.encrypt_to_string(&hex).map_err(|e| CsppError::Encrypt(e.to_string()))?;

        let encryption_key = cryptor.serialize_to_string();

        self.0
            .save(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into(), encryption_key)
            .map_err(|e| CsppError::Save(e.to_string()))?;

        self.0
            .save(MASTER_KEY_NAME.into(), encrypted)
            .map_err(|e| CsppError::Save(e.to_string()))?;

        Ok(())
    }

    /// Loads the master key, returns None if not found
    fn get_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
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
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    // serializes tests that touch the global MASTER_KEY_CACHE
    static CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    struct MockStore(Mutex<HashMap<String, String>>);

    impl MockStore {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }
    }

    impl Cspp<MockStore> {
        fn reset_cache() {
            MASTER_KEY_CACHE.store(None);
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
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

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
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();
        let first = cspp.get_or_create_master_key().unwrap();
        let first_bytes = *first.as_bytes();

        let second = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*second.as_bytes(), first_bytes);
    }

    #[test]
    fn get_or_create_reuses_existing() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();
        let original = MasterKey::generate();
        let original_bytes = *original.as_bytes();
        cspp.save_master_key(&original).unwrap();

        let loaded = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*loaded.as_bytes(), original_bytes);
    }
}
