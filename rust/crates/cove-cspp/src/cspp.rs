use cove_util::ResultExt as _;
use std::sync::{Arc, LazyLock, Mutex};
use tracing::{info, warn};

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

/// Clear the cached master key so the next `get_or_create_master_key()` reloads from the store
pub fn reset_master_key_cache() {
    MASTER_KEY_CACHE.store(None);
}

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
            info!("Master key loaded from keychain");
            MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*key.as_bytes()))));
            return Ok(key);
        }

        // generate and save new key
        warn!("Master key not found in keychain, generating new key");
        let key = MasterKey::generate();
        self.save_master_key(&key)?;
        MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*key.as_bytes()))));

        Ok(key)
    }

    /// Saves the master key encrypted via the underlying store
    ///
    /// The master key is encrypted with a random [`Cryptor`] before storage. The
    /// keychain already provides at-rest encryption, but this extra layer prevents
    /// the plaintext key from being accidentally exposed if other code enumerates
    /// keychain entries — it must be explicitly decrypted to be read
    pub fn save_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let hex = hex::encode(master_key.as_bytes());
        let mut cryptor = Cryptor::new();

        let encrypted = cryptor.encrypt_to_string(&hex).map_err_str(CsppError::Encrypt)?;

        let encryption_key = cryptor.serialize_to_string();

        self.0
            .save(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into(), encryption_key)
            .map_err_str(CsppError::Save)?;

        self.0.save(MASTER_KEY_NAME.into(), encrypted).map_err_str(CsppError::Save)?;

        Ok(())
    }

    /// Loads the master key, returns None if not found
    fn get_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let has_encryption_key = self.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into());
        let has_master_key = self.0.get(MASTER_KEY_NAME.into());

        info!(
            encryption_key_found = has_encryption_key.is_some(),
            master_key_found = has_master_key.is_some(),
            "Keychain master key lookup"
        );

        let Some(encryption_secret) = has_encryption_key else {
            return Ok(None);
        };

        let Some(encrypted) = has_master_key else {
            return Ok(None);
        };

        let cryptor =
            Cryptor::try_from_string(&encryption_secret).map_err_str(CsppError::Decrypt)?;

        let hex = cryptor.decrypt_from_string(&encrypted).map_err_str(CsppError::Decrypt)?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err_str(CsppError::InvalidData)?
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

    #[test]
    fn master_key_cache_cleared_on_reset() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();

        // populate cache
        let first = cspp.get_or_create_master_key().unwrap();
        let first_bytes = *first.as_bytes();

        // verify cache is populated
        assert!(MASTER_KEY_CACHE.load().is_some());

        // reset cache via public API
        reset_master_key_cache();

        // verify cache is cleared
        assert!(MASTER_KEY_CACHE.load().is_none());

        // next call should reload from store (same key since store still has it)
        let reloaded = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*reloaded.as_bytes(), first_bytes);
    }
}
