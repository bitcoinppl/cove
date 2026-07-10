use cove_util::ResultExt as _;
use serde::{Deserialize, Serialize};
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
const STAGED_MASTER_KEY_NAME: &str = "cspp::v1::staged_master_key";
const STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE: &str =
    "cspp::v1::staged_master_key_encryption_key_and_nonce";
const MASTER_KEY_PROMOTION_JOURNAL: &str = "cspp::v1::master_key_promotion_journal";
const MASTER_KEY_PROMOTION_JOURNAL_VERSION: u32 = 1;

static INIT_LOCK: Mutex<()> = Mutex::new(());
static MASTER_KEY_CACHE: LazyLock<ArcSwapOption<Zeroizing<[u8; 32]>>> =
    LazyLock::new(|| ArcSwapOption::new(None));

pub struct Cspp<S: CsppStore>(S);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterKeyPromotionActiveState {
    Prior,
    Staged,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterKeyPromotionStatus {
    None,
    Staged,
    Pending(MasterKeyPromotionActiveState),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredMasterKeyEntries {
    encryption_key_and_nonce: Option<String>,
    encrypted_master_key: Option<String>,
}

impl StoredMasterKeyEntries {
    fn complete(encryption_key_and_nonce: String, encrypted_master_key: String) -> Self {
        Self {
            encryption_key_and_nonce: Some(encryption_key_and_nonce),
            encrypted_master_key: Some(encrypted_master_key),
        }
    }

    fn is_absent(&self) -> bool {
        self.encryption_key_and_nonce.is_none() && self.encrypted_master_key.is_none()
    }

    fn is_complete(&self) -> bool {
        self.encryption_key_and_nonce.is_some() && self.encrypted_master_key.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MasterKeyPromotionJournal {
    version: u32,
    prior: StoredMasterKeyEntries,
    staged: StoredMasterKeyEntries,
}

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

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "master key promotion must be resolved before creating an active key".into(),
            ));
        }

        // generate and save new key
        warn!("Master key not found in keychain, generating new key");
        let key = MasterKey::generate();
        self.save_master_key_locked(&key)?;

        Ok(key)
    }

    /// Saves the master key encrypted via the underlying store
    ///
    /// The master key is encrypted with a random [`Cryptor`] before storage. The
    /// keychain already provides at-rest encryption, but this extra layer prevents
    /// the plaintext key from being accidentally exposed if other code enumerates
    /// keychain entries — it must be explicitly decrypted to be read
    pub fn save_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "master key promotion must be resolved before replacing the active key".into(),
            ));
        }

        self.save_master_key_locked(master_key)
    }

    /// Saves a fresh master key in an isolated slot without changing active storage or cache
    pub fn save_staged_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "cannot replace a staged master key while promotion is pending".into(),
            ));
        }

        let existing = self.read_staged_entries();
        if existing.is_complete() {
            let existing_key = match Self::decrypt_entries(&existing) {
                Ok(existing_key) => existing_key,
                Err(error) => {
                    self.restore_staged_entries(&StoredMasterKeyEntries::default())?;

                    return Err(CsppError::InvalidData(format!(
                        "invalid staged master key was discarded: {error}"
                    )));
                }
            };
            if existing_key.as_bytes() == master_key.as_bytes() {
                return Ok(());
            }

            return Err(CsppError::InvalidData(
                "a different staged master key already exists".into(),
            ));
        }

        if !existing.is_absent() {
            self.restore_staged_entries(&StoredMasterKeyEntries::default())?;

            return Err(CsppError::InvalidData(
                "incomplete staged master key was discarded".into(),
            ));
        }

        let staged = Self::encrypt_master_key(master_key)?;
        if let Err(error) = self.restore_staged_entries(&staged) {
            let rollback = self.restore_staged_entries(&existing);
            if let Err(rollback_error) = rollback {
                return Err(CsppError::Save(format!(
                    "{error}; unable to restore prior staged entries: {rollback_error}"
                )));
            }

            return Err(error);
        }

        Ok(())
    }

    /// Loads the isolated staged master key without consulting or changing active cache
    pub fn load_staged_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let entries = self.read_staged_entries();
        if entries.is_absent() {
            return Ok(None);
        }

        if !entries.is_complete() {
            return Err(CsppError::InvalidData("staged master key entries are incomplete".into()));
        }

        Self::decrypt_entries(&entries).map(Some)
    }

    /// Discards an unpromoted staged master key
    pub fn discard_staged_master_key(&self) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "use promotion rollback while a master key promotion is pending".into(),
            ));
        }

        self.restore_staged_entries(&StoredMasterKeyEntries::default())
    }

    /// Installs the staged key as active while retaining an exact rollback journal
    ///
    /// This operation is idempotent. A caller must separately commit or roll back
    /// the promotion after its own durable state has been finalized
    pub fn promote_staged_master_key(&self) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let journal = match self.load_promotion_journal()? {
            Some(journal) => journal,
            None => {
                let staged = self.read_staged_entries();
                if !staged.is_complete() {
                    return Err(CsppError::InvalidData(
                        "no complete staged master key is available for promotion".into(),
                    ));
                }
                Self::decrypt_entries(&staged)?;

                let journal = MasterKeyPromotionJournal {
                    version: MASTER_KEY_PROMOTION_JOURNAL_VERSION,
                    prior: self.read_active_entries(),
                    staged,
                };
                self.save_promotion_journal(&journal)?;
                journal
            }
        };

        Self::decrypt_entries(&journal.staged)?;
        self.restore_staged_entries(&journal.staged)?;
        if let Err(error) = self.restore_active_entries(&journal.staged) {
            Self::clear_cached_master_key();

            return Err(error);
        }

        let installed = self.read_active_entries();
        if installed != journal.staged {
            Self::clear_cached_master_key();

            return Err(CsppError::Save(
                "active master key does not match the durable promotion journal".into(),
            ));
        }

        let master_key = Self::decrypt_entries(&installed)?;
        Self::update_cached_master_key(&master_key);

        Ok(())
    }

    /// Commits an installed promotion and removes its staged and rollback material
    pub fn commit_master_key_promotion(&self) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let Some(journal) = self.load_promotion_journal()? else {
            if self.read_staged_entries().is_absent() {
                return Ok(());
            }

            return Err(CsppError::InvalidData(
                "cannot commit a staged master key that was not promoted".into(),
            ));
        };

        let active = self.read_active_entries();
        if active != journal.staged {
            Self::clear_cached_master_key();

            return Err(CsppError::InvalidData(
                "cannot commit before the staged master key is fully active".into(),
            ));
        }

        let master_key = Self::decrypt_entries(&active)?;
        self.restore_staged_entries(&StoredMasterKeyEntries::default())?;
        self.delete_entry_exact(MASTER_KEY_PROMOTION_JOURNAL)?;
        Self::update_cached_master_key(&master_key);

        Ok(())
    }

    /// Reactivates the prior master key while retaining staged promotion material for retry
    pub fn restore_prior_master_key_for_retry(&self) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let Some(journal) = self.load_promotion_journal()? else {
            if matches!(self.master_key_promotion_status()?, MasterKeyPromotionStatus::Staged) {
                return Ok(());
            }

            return Err(CsppError::InvalidData(
                "no master key promotion is available to restore for retry".into(),
            ));
        };

        if let Err(error) = self.restore_active_entries(&journal.prior) {
            Self::clear_cached_master_key();

            return Err(error);
        }

        if self.read_active_entries() != journal.prior {
            Self::clear_cached_master_key();

            return Err(CsppError::Save(
                "active master key retry rollback did not restore the durable snapshot".into(),
            ));
        }

        Self::refresh_cache_from_entries(&journal.prior);

        Ok(())
    }

    /// Restores the exact active entries captured before promotion
    pub fn rollback_master_key_promotion(&self) -> Result<(), CsppError> {
        let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let Some(journal) = self.load_promotion_journal()? else {
            return self.restore_staged_entries(&StoredMasterKeyEntries::default());
        };

        if let Err(error) = self.restore_active_entries(&journal.prior) {
            Self::clear_cached_master_key();

            return Err(error);
        }

        if self.read_active_entries() != journal.prior {
            Self::clear_cached_master_key();

            return Err(CsppError::Save(
                "active master key rollback did not restore the durable snapshot".into(),
            ));
        }

        Self::refresh_cache_from_entries(&journal.prior);
        self.restore_staged_entries(&StoredMasterKeyEntries::default())?;
        self.delete_entry_exact(MASTER_KEY_PROMOTION_JOURNAL)?;

        Ok(())
    }

    /// Reports durable staging and promotion state without mutating it
    pub fn master_key_promotion_status(&self) -> Result<MasterKeyPromotionStatus, CsppError> {
        let Some(journal) = self.load_promotion_journal()? else {
            let staged = self.read_staged_entries();
            if staged.is_absent() {
                return Ok(MasterKeyPromotionStatus::None);
            }
            if !staged.is_complete() {
                return Err(CsppError::InvalidData(
                    "staged master key entries are incomplete".into(),
                ));
            }
            Self::decrypt_entries(&staged)?;

            return Ok(MasterKeyPromotionStatus::Staged);
        };

        Self::decrypt_entries(&journal.staged)?;
        let active = self.read_active_entries();
        let active_state = if active == journal.staged {
            MasterKeyPromotionActiveState::Staged
        } else if active == journal.prior {
            MasterKeyPromotionActiveState::Prior
        } else {
            MasterKeyPromotionActiveState::Incomplete
        };

        Ok(MasterKeyPromotionStatus::Pending(active_state))
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
        let master_key_deleted = self.delete_key_if_present(MASTER_KEY_NAME);
        let encryption_key_deleted =
            self.delete_key_if_present(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE);
        let staged_master_key_deleted = self.delete_key_if_present(STAGED_MASTER_KEY_NAME);
        let staged_encryption_key_deleted =
            self.delete_key_if_present(STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE);
        let journal_deleted = self.delete_key_if_present(MASTER_KEY_PROMOTION_JOURNAL);
        Self::clear_cached_master_key();
        master_key_deleted
            && encryption_key_deleted
            && staged_master_key_deleted
            && staged_encryption_key_deleted
            && journal_deleted
    }

    /// Checks whether the master key exists in the store without decrypting it
    pub fn has_master_key(&self) -> bool {
        self.0.get(MASTER_KEY_NAME.into()).is_some()
            && self.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into()).is_some()
    }

    /// Loads the master key, returns None if not found
    fn get_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let entries = self.read_active_entries();

        info!(
            encryption_key_found = entries.encryption_key_and_nonce.is_some(),
            master_key_found = entries.encrypted_master_key.is_some(),
            "Keychain master key lookup"
        );

        if entries.is_absent() {
            return Ok(None);
        }

        if !entries.is_complete() {
            return Ok(None);
        }

        Self::decrypt_entries(&entries).map(Some)
    }

    fn save_master_key_locked(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let prior = self.read_active_entries();
        let replacement = Self::encrypt_master_key(master_key)?;

        if let Err(error) = self.restore_active_entries(&replacement) {
            if let Err(rollback_error) = self.restore_active_entries(&prior) {
                Self::clear_cached_master_key();

                return Err(CsppError::Save(format!(
                    "{error}; unable to restore prior active entries: {rollback_error}"
                )));
            }

            Self::refresh_cache_from_entries(&prior);

            return Err(error);
        }

        let installed = self.read_active_entries();
        if installed != replacement {
            let rollback = self.restore_active_entries(&prior);
            Self::refresh_cache_from_entries(&prior);
            rollback?;

            return Err(CsppError::Save("active master key write could not be verified".into()));
        }

        Self::decrypt_entries(&installed)?;
        Self::update_cached_master_key(master_key);

        Ok(())
    }

    fn encrypt_master_key(master_key: &MasterKey) -> Result<StoredMasterKeyEntries, CsppError> {
        let hex = hex::encode(master_key.as_bytes());
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(&hex).map_err_str(CsppError::Encrypt)?;
        let encryption_key = cryptor.serialize_to_string();

        Ok(StoredMasterKeyEntries::complete(encryption_key, encrypted))
    }

    fn decrypt_entries(entries: &StoredMasterKeyEntries) -> Result<MasterKey, CsppError> {
        let (Some(encryption_secret), Some(encrypted)) =
            (entries.encryption_key_and_nonce.as_ref(), entries.encrypted_master_key.as_ref())
        else {
            return Err(CsppError::InvalidData("master key entries are incomplete".into()));
        };

        let cryptor =
            Cryptor::try_from_string(encryption_secret).map_err_str(CsppError::Decrypt)?;

        let hex = cryptor.decrypt_from_string(encrypted).map_err_str(CsppError::Decrypt)?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err_str(CsppError::InvalidData)?
            .try_into()
            .map_err(|_| CsppError::InvalidData("master key not 32 bytes".into()))?;

        Ok(MasterKey::from_bytes(bytes))
    }

    fn read_active_entries(&self) -> StoredMasterKeyEntries {
        self.read_entries(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, MASTER_KEY_NAME)
    }

    fn read_staged_entries(&self) -> StoredMasterKeyEntries {
        self.read_entries(STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, STAGED_MASTER_KEY_NAME)
    }

    fn read_entries(&self, encryption_key: &str, master_key: &str) -> StoredMasterKeyEntries {
        StoredMasterKeyEntries {
            encryption_key_and_nonce: self.0.get(encryption_key.to_owned()),
            encrypted_master_key: self.0.get(master_key.to_owned()),
        }
    }

    fn restore_active_entries(&self, entries: &StoredMasterKeyEntries) -> Result<(), CsppError> {
        self.restore_entries(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, MASTER_KEY_NAME, entries)
    }

    fn restore_staged_entries(&self, entries: &StoredMasterKeyEntries) -> Result<(), CsppError> {
        self.restore_entries(
            STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE,
            STAGED_MASTER_KEY_NAME,
            entries,
        )
    }

    fn restore_entries(
        &self,
        encryption_key_name: &str,
        master_key_name: &str,
        entries: &StoredMasterKeyEntries,
    ) -> Result<(), CsppError> {
        self.restore_entry(encryption_key_name, entries.encryption_key_and_nonce.as_deref())?;
        self.restore_entry(master_key_name, entries.encrypted_master_key.as_deref())?;

        let restored = self.read_entries(encryption_key_name, master_key_name);
        if &restored != entries {
            return Err(CsppError::Save(format!(
                "unable to verify restored master key entries for {master_key_name}"
            )));
        }

        Ok(())
    }

    fn restore_entry(&self, key: &str, value: Option<&str>) -> Result<(), CsppError> {
        match value {
            Some(value) => {
                self.0.save(key.to_owned(), value.to_owned()).map_err_str(CsppError::Save)
            }
            None => self.delete_entry_exact(key),
        }
    }

    fn delete_entry_exact(&self, key: &str) -> Result<(), CsppError> {
        if self.0.get(key.to_owned()).is_none() {
            return Ok(());
        }

        self.0.delete(key.to_owned());
        if self.0.get(key.to_owned()).is_some() {
            return Err(CsppError::Save(format!("unable to delete {key}")));
        }

        Ok(())
    }

    fn save_promotion_journal(&self, journal: &MasterKeyPromotionJournal) -> Result<(), CsppError> {
        let serialized = serde_json::to_string(journal).map_err_str(CsppError::Serialization)?;

        self.0.save(MASTER_KEY_PROMOTION_JOURNAL.into(), serialized).map_err_str(CsppError::Save)
    }

    fn load_promotion_journal(&self) -> Result<Option<MasterKeyPromotionJournal>, CsppError> {
        let Some(serialized) = self.0.get(MASTER_KEY_PROMOTION_JOURNAL.into()) else {
            return Ok(None);
        };
        let journal: MasterKeyPromotionJournal =
            serde_json::from_str(&serialized).map_err_str(CsppError::Deserialization)?;
        if journal.version != MASTER_KEY_PROMOTION_JOURNAL_VERSION {
            return Err(CsppError::InvalidData(format!(
                "unsupported master key promotion journal version {}",
                journal.version
            )));
        }
        if !journal.staged.is_complete() {
            return Err(CsppError::InvalidData(
                "promotion journal contains an incomplete staged master key".into(),
            ));
        }

        Ok(Some(journal))
    }

    fn refresh_cache_from_entries(entries: &StoredMasterKeyEntries) {
        match Self::decrypt_entries(entries) {
            Ok(master_key) => Self::update_cached_master_key(&master_key),
            Err(_) => Self::clear_cached_master_key(),
        }
    }

    fn update_cached_master_key(master_key: &MasterKey) {
        MASTER_KEY_CACHE.store(Some(Arc::new(Zeroizing::new(*master_key.as_bytes()))));
    }

    fn delete_key_if_present(&self, key: &str) -> bool {
        self.0.get(key.to_owned()).is_none() || self.0.delete(key.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    // serializes tests that touch the global MASTER_KEY_CACHE
    static CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    struct MockStore(Mutex<HashMap<String, String>>);

    impl MockStore {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }

        fn with_entries(entries: Vec<(&str, &str)>) -> Self {
            Self(Mutex::new(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect(),
            ))
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

    #[derive(Clone, Debug)]
    struct FailNthStore(Arc<Mutex<FailNthStoreState>>);

    #[derive(Debug, Default)]
    struct FailNthStoreState {
        entries: HashMap<String, String>,
        mutation_count: usize,
        fail_at: Option<usize>,
    }

    impl FailNthStore {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(FailNthStoreState::default())))
        }

        fn with_entries(entries: Vec<(&str, &str)>) -> Self {
            Self(Arc::new(Mutex::new(FailNthStoreState {
                entries: entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect(),
                ..Default::default()
            })))
        }

        fn fail_nth_mutation(&self, nth: usize) {
            let mut state = self.0.lock().unwrap();
            state.fail_at = Some(state.mutation_count + nth);
        }

        fn should_fail(state: &mut FailNthStoreState) -> bool {
            state.mutation_count += 1;
            let should_fail = state.fail_at == Some(state.mutation_count);
            if should_fail {
                state.fail_at = None;
            }

            should_fail
        }
    }

    impl CsppStore for FailNthStore {
        type Error = String;

        fn save(&self, key: String, value: String) -> Result<(), Self::Error> {
            let mut state = self.0.lock().unwrap();
            if Self::should_fail(&mut state) {
                return Err(format!("failed mutation {}", state.mutation_count));
            }

            state.entries.insert(key, value);

            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().unwrap().entries.get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            let mut state = self.0.lock().unwrap();
            if Self::should_fail(&mut state) {
                return false;
            }

            state.entries.remove(&key).is_some()
        }
    }

    fn mock_cspp() -> Cspp<MockStore> {
        Cspp::new(MockStore::new())
    }

    fn deterministic_key(byte: u8) -> MasterKey {
        MasterKey::from_bytes([byte; 32])
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

    #[test]
    fn delete_master_key_treats_empty_store_as_success() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();

        assert!(cspp.delete_master_key());
    }

    #[test]
    fn delete_master_key_removes_partial_master_key_entry() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = Cspp::new(MockStore::with_entries(vec![(MASTER_KEY_NAME, "encrypted")]));

        assert!(cspp.delete_master_key());
        assert!(cspp.0.get(MASTER_KEY_NAME.into()).is_none());
    }

    #[test]
    fn delete_master_key_removes_partial_encryption_key_entry() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = Cspp::new(MockStore::with_entries(vec![(
            MASTER_KEY_ENCRYPTION_KEY_AND_NONCE,
            "encryption-key",
        )]));

        assert!(cspp.delete_master_key());
        assert!(cspp.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into()).is_none());
    }

    #[test]
    fn staged_master_key_is_isolated_and_same_key_save_is_idempotent() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let cspp = Cspp::new(FailNthStore::new());
        let active = deterministic_key(1);
        let staged = deterministic_key(2);
        cspp.save_master_key(&active).unwrap();
        let active_entries = cspp.read_active_entries();

        cspp.save_staged_master_key(&staged).unwrap();
        let staged_entries = cspp.read_staged_entries();
        cspp.save_staged_master_key(&staged).unwrap();

        assert_eq!(cspp.read_active_entries(), active_entries);
        assert_eq!(cspp.read_staged_entries(), staged_entries);
        assert_eq!(cspp.load_staged_master_key().unwrap().unwrap().as_bytes(), staged.as_bytes());
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), active.as_bytes());
        assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    }

    #[test]
    fn staged_master_key_rejects_replacement_and_cleans_partial_entries() {
        let cspp = Cspp::new(FailNthStore::new());
        let staged = deterministic_key(3);
        cspp.save_staged_master_key(&staged).unwrap();
        let staged_entries = cspp.read_staged_entries();

        assert!(cspp.save_staged_master_key(&deterministic_key(4)).is_err());
        assert_eq!(cspp.read_staged_entries(), staged_entries);

        cspp.discard_staged_master_key().unwrap();
        cspp.0.save(STAGED_MASTER_KEY_NAME.into(), "partial".into()).unwrap();

        assert!(cspp.save_staged_master_key(&staged).is_err());
        assert!(cspp.read_staged_entries().is_absent());
    }

    #[test]
    fn active_and_staged_saves_restore_exact_entries_after_failure() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(5);
        cspp.save_master_key(&prior).unwrap();
        let prior_entries = cspp.read_active_entries();

        store.fail_nth_mutation(2);
        assert!(cspp.save_master_key(&deterministic_key(6)).is_err());
        assert_eq!(cspp.read_active_entries(), prior_entries);
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), prior.as_bytes());

        store.fail_nth_mutation(2);
        assert!(cspp.save_staged_master_key(&deterministic_key(7)).is_err());
        assert!(cspp.read_staged_entries().is_absent());
    }

    #[test]
    fn promotion_is_idempotent_and_rollback_restores_exact_prior_entries() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(8);
        let staged = deterministic_key(9);
        cspp.save_master_key(&prior).unwrap();
        let prior_entries = cspp.read_active_entries();
        cspp.save_staged_master_key(&staged).unwrap();

        cspp.promote_staged_master_key().unwrap();
        let promoted_entries = cspp.read_active_entries();
        cspp.promote_staged_master_key().unwrap();

        assert_eq!(cspp.read_active_entries(), promoted_entries);
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), staged.as_bytes());

        recreated.rollback_master_key_promotion().unwrap();
        recreated.rollback_master_key_promotion().unwrap();
        assert_eq!(recreated.read_active_entries(), prior_entries);
        assert!(recreated.read_staged_entries().is_absent());
        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::None
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), prior.as_bytes());
    }

    #[test]
    fn retry_restore_reactivates_prior_without_discarding_staged_promotion() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(21);
        let staged = deterministic_key(22);
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&staged).unwrap();
        cspp.promote_staged_master_key().unwrap();

        cspp.restore_prior_master_key_for_retry().unwrap();

        assert_eq!(
            cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
            prior.as_bytes()
        );
        assert_eq!(cspp.load_staged_master_key().unwrap().unwrap().as_bytes(), staged.as_bytes());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        recreated.promote_staged_master_key().unwrap();
        assert_eq!(
            recreated.load_master_key_from_store().unwrap().unwrap().as_bytes(),
            staged.as_bytes()
        );
        recreated.commit_master_key_promotion().unwrap();
        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::None
        );
    }

    #[test]
    fn interrupted_retry_restore_clears_cache_and_can_resume() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(23);
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&deterministic_key(24)).unwrap();
        cspp.promote_staged_master_key().unwrap();

        store.fail_nth_mutation(2);
        assert!(cspp.restore_prior_master_key_for_retry().is_err());
        assert!(MASTER_KEY_CACHE.load().is_none());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Incomplete)
        );

        cspp.restore_prior_master_key_for_retry().unwrap();

        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), prior.as_bytes());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
        );
    }

    #[test]
    fn rollback_restores_missing_and_partial_prior_state_exactly() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let empty_store = FailNthStore::new();
        let empty_cspp = Cspp::new(empty_store);
        empty_cspp.save_staged_master_key(&deterministic_key(10)).unwrap();
        empty_cspp.promote_staged_master_key().unwrap();
        empty_cspp.rollback_master_key_promotion().unwrap();

        assert!(empty_cspp.read_active_entries().is_absent());
        assert!(MASTER_KEY_CACHE.load().is_none());

        let partial_store = FailNthStore::with_entries(vec![(MASTER_KEY_NAME, "prior-raw")]);
        let partial_cspp = Cspp::new(partial_store);
        let partial_prior = partial_cspp.read_active_entries();
        partial_cspp.save_staged_master_key(&deterministic_key(11)).unwrap();
        partial_cspp.promote_staged_master_key().unwrap();
        partial_cspp.rollback_master_key_promotion().unwrap();

        assert_eq!(partial_cspp.read_active_entries(), partial_prior);
        assert!(MASTER_KEY_CACHE.load().is_none());
    }

    #[test]
    fn interrupted_promotion_is_reported_and_can_resume_after_recreation() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(12);
        let staged = deterministic_key(13);
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&staged).unwrap();

        store.fail_nth_mutation(2);
        assert!(cspp.promote_staged_master_key().is_err());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
        );

        store.fail_nth_mutation(4);
        assert!(cspp.promote_staged_master_key().is_err());
        assert!(MASTER_KEY_CACHE.load().is_none());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Incomplete)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        recreated.promote_staged_master_key().unwrap();

        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
        );
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), staged.as_bytes());
    }

    #[test]
    fn interrupted_rollback_can_resume_after_recreation() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(14);
        cspp.save_master_key(&prior).unwrap();
        let prior_entries = cspp.read_active_entries();
        cspp.save_staged_master_key(&deterministic_key(15)).unwrap();
        cspp.promote_staged_master_key().unwrap();

        store.fail_nth_mutation(2);
        assert!(cspp.rollback_master_key_promotion().is_err());
        assert!(MASTER_KEY_CACHE.load().is_none());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Incomplete)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        recreated.rollback_master_key_promotion().unwrap();

        assert_eq!(recreated.read_active_entries(), prior_entries);
        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::None
        );
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), prior.as_bytes());
    }

    #[test]
    fn interrupted_commit_can_resume_after_recreation() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let staged = deterministic_key(16);
        cspp.save_staged_master_key(&staged).unwrap();
        cspp.promote_staged_master_key().unwrap();

        store.fail_nth_mutation(2);
        assert!(cspp.commit_master_key_promotion().is_err());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        recreated.commit_master_key_promotion().unwrap();
        recreated.commit_master_key_promotion().unwrap();

        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::None
        );
        assert!(recreated.read_staged_entries().is_absent());
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), staged.as_bytes());
    }

    #[test]
    fn active_save_is_rejected_while_promotion_is_pending() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let cspp = Cspp::new(FailNthStore::new());
        let staged = deterministic_key(17);
        cspp.save_staged_master_key(&staged).unwrap();
        cspp.promote_staged_master_key().unwrap();
        let promoted = cspp.read_active_entries();

        assert!(cspp.save_master_key(&deterministic_key(18)).is_err());
        assert_eq!(cspp.read_active_entries(), promoted);
    }

    #[test]
    fn delete_master_key_purges_staging_and_promotion_state() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let cspp = Cspp::new(FailNthStore::new());
        cspp.save_master_key(&deterministic_key(19)).unwrap();
        cspp.save_staged_master_key(&deterministic_key(20)).unwrap();
        cspp.promote_staged_master_key().unwrap();

        assert!(cspp.delete_master_key());
        assert!(cspp.read_active_entries().is_absent());
        assert!(cspp.read_staged_entries().is_absent());
        assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
        assert!(MASTER_KEY_CACHE.load().is_none());
    }
}
