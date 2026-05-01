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

pub struct Cspp<S: CsppStore>(S);

impl<S: CsppStore> Cspp<S> {
    pub fn new(store: S) -> Self {
        Self(store)
    }

    /// Clears the process-local cache without modifying persisted key material
    ///
    /// Used by bootstrap and debug reset flows that need to drop in-memory state
    /// across runtime transitions
    pub fn clear_cached_master_key() {
        MASTER_KEY_CACHE.store(None);
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
            Self::update_cached_master_key(&key);
            return Ok(key);
        }

        // generate and save new key
        warn!("Master key not found in keychain, generating new key");
        let key = MasterKey::generate();
        self.save_master_key(&key)?;

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
        Self::update_cached_master_key(master_key);

        Ok(())
    }

    /// Loads the master key directly from the store, bypassing the in-memory cache
    ///
    /// Used by verification to detect keychain corruption even if the cache
    /// was populated earlier in the session
    pub fn load_master_key_from_store(&self) -> Result<Option<MasterKey>, CsppError> {
        self.get_master_key()
    }

    /// Deletes the master key and its encryption key from the store
    ///
    /// Used by debug reset to fully clear CSPP state
    pub fn delete_master_key(&self) -> bool {
        let master_key_deleted = self.0.delete(MASTER_KEY_NAME.into());
        let encryption_key_deleted = self.0.delete(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into());
        Self::clear_cached_master_key();
        master_key_deleted && encryption_key_deleted
    }

    /// Checks whether the master key exists in the store without decrypting it
    pub fn has_master_key(&self) -> bool {
        self.0.get(MASTER_KEY_NAME.into()).is_some()
            && self.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into()).is_some()
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

    fn update_cached_master_key(master_key: &MasterKey) {
        MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*master_key.as_bytes()))));
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
            Self::clear_cached_master_key();
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

        // reset cache via runtime-reset API
        Cspp::<MockStore>::clear_cached_master_key();

        // verify cache is cleared
        assert!(MASTER_KEY_CACHE.load().is_none());

        // next call should reload from store (same key since store still has it)
        let reloaded = cspp.get_or_create_master_key().unwrap();
        assert_eq!(*reloaded.as_bytes(), first_bytes);
    }

    #[test]
    fn save_master_key_refreshes_warm_cache() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();
        let original = cspp.get_or_create_master_key().unwrap();
        let replacement = MasterKey::generate();

        assert_ne!(original.as_bytes(), replacement.as_bytes());

        cspp.save_master_key(&replacement).unwrap();

        let loaded = cspp.get_or_create_master_key().unwrap();
        assert_eq!(loaded.as_bytes(), replacement.as_bytes());
    }

    #[test]
    fn delete_master_key_clears_warm_cache() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();
        let original = cspp.get_or_create_master_key().unwrap();
        cspp.delete_master_key();

        assert!(MASTER_KEY_CACHE.load().is_none());

        let regenerated = cspp.get_or_create_master_key().unwrap();
        assert_ne!(regenerated.as_bytes(), original.as_bytes());
    }
}
