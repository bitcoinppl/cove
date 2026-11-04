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

/// Every store entry CSPP owns, in deletion order
const ALL_ENTRY_NAMES: [&str; 5] = [
    MASTER_KEY_NAME,
    MASTER_KEY_ENCRYPTION_KEY_AND_NONCE,
    STAGED_MASTER_KEY_NAME,
    STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE,
    MASTER_KEY_PROMOTION_JOURNAL,
];

static INIT_LOCK: Mutex<()> = Mutex::new(());
static MASTER_KEY_CACHE: LazyLock<ArcSwapOption<Zeroizing<[u8; 32]>>> =
    LazyLock::new(|| ArcSwapOption::new(None));

pub struct Cspp<S: CsppStore>(S);

/// Snapshot of the active master-key entries in the CSPP store
///
/// The storage keys and serialized values remain private to CSPP so callers can
/// restore a failed cross-store operation without depending on its key layout
#[derive(Clone)]
pub struct ActiveMasterKeySnapshot(StoredMasterKeyEntries);

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

/// Privacy-preserving comparison of promotion material with caller-owned namespaces
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasterKeyPromotionEvidence {
    pub status: MasterKeyPromotionStatus,
    pub staged_matches_expected: bool,
    pub prior_matches_expected: bool,
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

/// Persisted master-key state, read under `INIT_LOCK` so one decision sees one snapshot
///
/// Every variant comes from successful store reads. A read failure is returned as an error
/// instead, so it can never be mistaken for `Absent` and trigger key creation
enum MasterKeyState {
    /// Neither active entry nor a promotion journal exists
    Absent,

    /// The active entries decrypt to a master key
    Active(MasterKey),

    /// Only one of the active entries exists and no promotion journal can repair it
    Incomplete,

    /// A promotion journal exists and the active entries are not complete
    PromotionPending,
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

    /// Captures the active master-key entries for a later rollback
    ///
    /// # Errors
    ///
    /// Returns an error when the active entries cannot be read, since restoring an unread
    /// snapshot would delete entries that still exist
    pub fn capture_active_master_key_snapshot(&self) -> Result<ActiveMasterKeySnapshot, CsppError> {
        let _guard = lock_init();

        self.read_active_entries().map(ActiveMasterKeySnapshot)
    }

    /// Restores active master-key entries captured before a failed operation
    pub fn restore_active_master_key_snapshot(
        &self,
        snapshot: &ActiveMasterKeySnapshot,
    ) -> Result<(), CsppError> {
        let _guard = lock_init();

        if let Err(error) = self.restore_active_entries(&snapshot.0) {
            Self::clear_cached_master_key();

            return Err(error);
        }

        Self::refresh_cache_from_entries(&snapshot.0);

        Ok(())
    }

    /// Loads the master key from the store, or generates and saves a new one
    ///
    /// A new key is generated only when the store positively reports that no active entry and
    /// no promotion journal exist. Read failures, incomplete entries, and pending promotions
    /// return errors so an existing key is never replaced
    ///
    /// Uses double-checked locking to prevent a TOCTOU race where two threads
    /// could both observe no key and generate different master keys
    pub fn get_or_create_master_key(&self) -> Result<MasterKey, CsppError> {
        // fast path (lock-free): return cached key
        if let Some(bytes) = MASTER_KEY_CACHE.load().as_deref() {
            return Ok(MasterKey::from_bytes(**bytes));
        }

        // slow path: acquire init lock for double-checked initialization
        let _guard = lock_init();

        // re-check cache after acquiring lock
        if let Some(bytes) = MASTER_KEY_CACHE.load().as_deref() {
            return Ok(MasterKey::from_bytes(**bytes));
        }

        match self.load_master_key_state()? {
            MasterKeyState::Active(key) => {
                info!("Master key loaded from keychain");
                Self::update_cached_master_key(&key);

                Ok(key)
            }
            MasterKeyState::PromotionPending => Err(CsppError::InvalidData(
                "master key promotion must be resolved before creating an active key".into(),
            )),
            MasterKeyState::Incomplete => {
                Err(CsppError::InvalidData("active master key entries are incomplete".into()))
            }
            MasterKeyState::Absent => {
                warn!("Master key not found in keychain, generating new key");
                let key = MasterKey::generate();
                self.save_master_key_locked(&key)?;

                Ok(key)
            }
        }
    }

    /// Saves the master key encrypted via the underlying store
    ///
    /// The master key is encrypted with a random [`Cryptor`] before storage. The
    /// keychain already provides at-rest encryption, but this extra layer prevents
    /// the plaintext key from being accidentally exposed if other code enumerates
    /// keychain entries — it must be explicitly decrypted to be read
    pub fn save_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let _guard = lock_init();

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "master key promotion must be resolved before replacing the active key".into(),
            ));
        }

        self.save_master_key_locked(master_key)
    }

    /// Saves a fresh master key in an isolated slot without changing active storage or cache
    pub fn save_staged_master_key(&self, master_key: &MasterKey) -> Result<(), CsppError> {
        let _guard = lock_init();

        if self.load_promotion_journal()?.is_some() {
            return Err(CsppError::InvalidData(
                "cannot replace a staged master key while promotion is pending".into(),
            ));
        }

        let existing = self.read_staged_entries()?;
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
            if let Err(rollback_error) = self.restore_staged_entries(&existing) {
                return Err(error.with_failed_rollback(
                    rollback_error,
                    "unable to restore prior staged entries",
                ));
            }

            return Err(error);
        }

        Ok(())
    }

    /// Loads the staged master key without consulting or changing active cache
    ///
    /// While a promotion is pending, the durable promotion journal is authoritative
    /// and the isolated staging slot is only a repairable copy
    pub fn load_staged_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let _guard = lock_init();

        self.load_staged_master_key_locked()
    }

    fn load_staged_master_key_locked(&self) -> Result<Option<MasterKey>, CsppError> {
        let entries = match self.load_promotion_journal()? {
            Some(journal) => journal.staged,
            None => self.read_staged_entries()?,
        };

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
        let _guard = lock_init();

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
        let _guard = lock_init();

        let journal = match self.load_promotion_journal()? {
            Some(journal) => journal,
            None => {
                let staged = self.read_staged_entries()?;
                if !staged.is_complete() {
                    return Err(CsppError::InvalidData(
                        "no complete staged master key is available for promotion".into(),
                    ));
                }
                Self::decrypt_entries(&staged)?;

                // the journal's prior entries are what rollback restores, so they must come
                // from a successful read rather than an assumed-absent slot
                let journal = MasterKeyPromotionJournal {
                    version: MASTER_KEY_PROMOTION_JOURNAL_VERSION,
                    prior: self.read_active_entries()?,
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

        let installed = self.read_active_entries()?;
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
        let _guard = lock_init();

        let Some(journal) = self.load_promotion_journal()? else {
            if self.read_staged_entries()?.is_absent() {
                return Ok(());
            }

            return Err(CsppError::InvalidData(
                "cannot commit a staged master key that was not promoted".into(),
            ));
        };

        let active = self.read_active_entries()?;
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
        let _guard = lock_init();

        let Some(journal) = self.load_promotion_journal()? else {
            if matches!(
                self.master_key_promotion_status_locked()?,
                MasterKeyPromotionStatus::Staged
            ) {
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

        if self.read_active_entries()? != journal.prior {
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
        let _guard = lock_init();

        let Some(journal) = self.load_promotion_journal()? else {
            return self.restore_staged_entries(&StoredMasterKeyEntries::default());
        };

        if let Err(error) = self.restore_active_entries(&journal.prior) {
            Self::clear_cached_master_key();

            return Err(error);
        }

        if self.read_active_entries()? != journal.prior {
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
        let _guard = lock_init();

        self.master_key_promotion_status_locked()
    }

    fn master_key_promotion_status_locked(&self) -> Result<MasterKeyPromotionStatus, CsppError> {
        let Some(journal) = self.load_promotion_journal()? else {
            let staged = self.read_staged_entries()?;
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
        let active = self.read_active_entries()?;
        let active_state = if active == journal.staged {
            MasterKeyPromotionActiveState::Staged
        } else if active == journal.prior {
            MasterKeyPromotionActiveState::Prior
        } else {
            MasterKeyPromotionActiveState::Incomplete
        };

        Ok(MasterKeyPromotionStatus::Pending(active_state))
    }

    /// Compares promotion material without returning either stored namespace
    pub fn master_key_promotion_evidence(
        &self,
        expected_staged_namespace: &str,
        expected_prior_namespace: Option<&str>,
    ) -> Result<MasterKeyPromotionEvidence, CsppError> {
        let _guard = lock_init();
        let status = self.master_key_promotion_status_locked()?;
        let staged_matches_expected = self
            .load_staged_master_key_locked()?
            .is_some_and(|key| key.namespace_id() == expected_staged_namespace);
        let prior_entries = match self.load_promotion_journal()? {
            Some(journal) => journal.prior,
            None => self.read_active_entries()?,
        };

        let prior_matches_expected = match expected_prior_namespace {
            Some(expected) => {
                prior_entries.is_complete()
                    && Self::decrypt_entries(&prior_entries)?.namespace_id() == expected
            }
            None => prior_entries.is_absent(),
        };

        Ok(MasterKeyPromotionEvidence { status, staged_matches_expected, prior_matches_expected })
    }

    /// Loads the master key directly from the store, bypassing the in-memory cache
    ///
    /// Used by verification to detect keychain corruption even if the cache
    /// was populated earlier in the session. Returns `None` only when the store reports the
    /// active entries as absent or incomplete; a read failure is an error
    pub fn load_master_key_from_store(&self) -> Result<Option<MasterKey>, CsppError> {
        let _guard = lock_init();

        self.get_master_key()
    }

    /// Deletes the master key and its encryption key from the store
    ///
    /// Used by debug reset to fully clear CSPP state. Returns false when any entry could not be
    /// read or deleted
    pub fn delete_master_key(&self) -> bool {
        let _guard = lock_init();

        // read every entry before deleting any, so an unreadable entry never leaves its
        // counterpart deleted and the key unusable
        let mut present = Vec::with_capacity(ALL_ENTRY_NAMES.len());
        for key in ALL_ENTRY_NAMES {
            match self.read_entry(key) {
                Ok(Some(_)) => present.push(key),
                Ok(None) => {}
                Err(_) => return false,
            }
        }

        Self::clear_cached_master_key();

        // attempt every delete even after a failure so retries have less left to remove
        let failed_deletes =
            present.into_iter().filter(|key| !self.0.delete((*key).to_owned())).count();

        failed_deletes == 0
    }

    /// Checks whether both active master-key entries exist without decrypting them
    ///
    /// # Errors
    ///
    /// Returns an error when either entry cannot be read
    pub fn has_master_key(&self) -> Result<bool, CsppError> {
        let _guard = lock_init();

        Ok(self.read_active_entries()?.is_complete())
    }

    /// Reads active, journal, and decrypted key state as one typed snapshot
    fn load_master_key_state(&self) -> Result<MasterKeyState, CsppError> {
        let active = self.read_active_entries()?;
        if active.is_complete() {
            return Self::decrypt_entries(&active).map(MasterKeyState::Active);
        }

        if self.load_promotion_journal()?.is_some() {
            return Ok(MasterKeyState::PromotionPending);
        }

        if active.is_absent() {
            return Ok(MasterKeyState::Absent);
        }

        Ok(MasterKeyState::Incomplete)
    }

    /// Loads the master key, returning `None` when the active entries are absent or incomplete
    fn get_master_key(&self) -> Result<Option<MasterKey>, CsppError> {
        let entries = self.read_active_entries()?;

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
        // rollback restores these exact entries, so an unread prior must stop the write
        let prior = self.read_active_entries()?;
        let replacement = Self::encrypt_master_key(master_key)?;

        if let Err(error) = self.restore_active_entries(&replacement) {
            if let Err(rollback_error) = self.restore_active_entries(&prior) {
                Self::clear_cached_master_key();

                return Err(error.with_failed_rollback(
                    rollback_error,
                    "unable to restore prior active entries",
                ));
            }

            Self::refresh_cache_from_entries(&prior);

            return Err(error);
        }

        let installed = self.read_active_entries()?;
        if installed != replacement {
            self.restore_active_entries(&prior)?;
            Self::refresh_cache_from_entries(&prior);

            return Err(CsppError::Save("active master key write could not be verified".into()));
        }

        Self::decrypt_entries(&installed)?;
        Self::update_cached_master_key(master_key);

        Ok(())
    }

    fn encrypt_master_key(master_key: &MasterKey) -> Result<StoredMasterKeyEntries, CsppError> {
        let hex = Zeroizing::new(hex::encode(master_key.as_bytes()));
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(&hex).map_err_str(CsppError::Encrypt)?;
        let encryption_key = cryptor.serialize_to_string();

        Ok(StoredMasterKeyEntries::complete(encryption_key.as_str().to_owned(), encrypted))
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

        let decoded =
            Zeroizing::new(hex::decode(hex.as_str()).map_err_str(CsppError::InvalidData)?);
        let bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| CsppError::InvalidData("master key not 32 bytes".into()))?;

        Ok(MasterKey::from_bytes(bytes))
    }

    fn read_active_entries(&self) -> Result<StoredMasterKeyEntries, CsppError> {
        self.read_entries(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, MASTER_KEY_NAME)
    }

    fn read_staged_entries(&self) -> Result<StoredMasterKeyEntries, CsppError> {
        self.read_entries(STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, STAGED_MASTER_KEY_NAME)
    }

    fn read_entries(
        &self,
        encryption_key: &str,
        master_key: &str,
    ) -> Result<StoredMasterKeyEntries, CsppError> {
        Ok(StoredMasterKeyEntries {
            encryption_key_and_nonce: self.read_entry(encryption_key)?,
            encrypted_master_key: self.read_entry(master_key)?,
        })
    }

    fn read_entry(&self, key: &str) -> Result<Option<String>, CsppError> {
        self.0.get(key.to_owned()).map_err_prefix(key, CsppError::Read)
    }

    /// Writes the active entries, clearing the cache first
    ///
    /// Callers repopulate the cache only after verifying the result, so any failure, including
    /// an unreadable read-back after the write, leaves the cache empty instead of stale
    fn restore_active_entries(&self, entries: &StoredMasterKeyEntries) -> Result<(), CsppError> {
        Self::clear_cached_master_key();

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
        // read both slots before changing either, so an unreadable slot never leaves its
        // counterpart rewritten or deleted and the pair half-changed
        self.read_entries(encryption_key_name, master_key_name)?;

        self.restore_entry(encryption_key_name, entries.encryption_key_and_nonce.as_deref())?;
        self.restore_entry(master_key_name, entries.encrypted_master_key.as_deref())?;

        let restored = self.read_entries(encryption_key_name, master_key_name)?;
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
        if self.read_entry(key)?.is_none() {
            return Ok(());
        }

        self.0.delete(key.to_owned());
        if self.read_entry(key)?.is_some() {
            return Err(CsppError::Save(format!("unable to delete {key}")));
        }

        Ok(())
    }

    fn save_promotion_journal(&self, journal: &MasterKeyPromotionJournal) -> Result<(), CsppError> {
        let serialized = serde_json::to_string(journal).map_err_str(CsppError::Serialization)?;

        self.0.save(MASTER_KEY_PROMOTION_JOURNAL.into(), serialized).map_err_str(CsppError::Save)
    }

    fn load_promotion_journal(&self) -> Result<Option<MasterKeyPromotionJournal>, CsppError> {
        let Some(serialized) = self.read_entry(MASTER_KEY_PROMOTION_JOURNAL)? else {
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
}

fn lock_init() -> std::sync::MutexGuard<'static, ()> {
    INIT_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
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

        fn get(&self, key: String) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(&key).cloned())
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
        unreadable_keys: HashSet<String>,
        unreadable_after_save: Option<String>,
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

        /// Makes reads of `key` fail like a locked platform keychain until cleared
        fn fail_reads_of(&self, key: &str) {
            self.0.lock().unwrap().unreadable_keys.insert(key.to_owned());
        }

        fn clear_read_failures(&self) {
            let mut state = self.0.lock().unwrap();
            state.unreadable_keys.clear();
            state.unreadable_after_save = None;
        }

        /// Makes `key` unreadable once the next save of any entry completes
        fn fail_reads_after_next_save(&self, key: &str) {
            self.0.lock().unwrap().unreadable_after_save = Some(key.to_owned());
        }

        fn snapshot(&self) -> HashMap<String, String> {
            self.0.lock().unwrap().entries.clone()
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
            if let Some(unreadable) = state.unreadable_after_save.take() {
                state.unreadable_keys.insert(unreadable);
            }

            Ok(())
        }

        fn get(&self, key: String) -> Result<Option<String>, Self::Error> {
            let state = self.0.lock().unwrap();
            if state.unreadable_keys.contains(&key) {
                return Err(format!("{key} is unreadable"));
            }

            Ok(state.entries.get(&key).cloned())
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
    fn active_master_key_snapshot_restores_store_and_cache() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<MockStore>::reset_cache();

        let cspp = mock_cspp();
        let original = MasterKey::generate();
        let replacement = MasterKey::generate();
        cspp.save_master_key(&original).unwrap();
        let snapshot = cspp.capture_active_master_key_snapshot().unwrap();

        cspp.save_master_key(&replacement).unwrap();
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), replacement.as_bytes());

        cspp.restore_active_master_key_snapshot(&snapshot).unwrap();
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), original.as_bytes());
        assert_eq!(
            cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
            original.as_bytes()
        );
    }

    #[test]
    fn failed_active_master_key_snapshot_restore_clears_cache() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        cspp.save_master_key(&MasterKey::generate()).unwrap();
        let snapshot = cspp.capture_active_master_key_snapshot().unwrap();
        cspp.save_master_key(&MasterKey::generate()).unwrap();
        assert!(MASTER_KEY_CACHE.load().is_some());
        store.fail_nth_mutation(1);

        assert!(cspp.restore_active_master_key_snapshot(&snapshot).is_err());
        assert!(MASTER_KEY_CACHE.load().is_none());
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
        assert!(cspp.0.get(MASTER_KEY_NAME.into()).unwrap().is_none());
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
        assert!(cspp.0.get(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE.into()).unwrap().is_none());
    }

    #[test]
    fn staged_master_key_is_isolated_and_same_key_save_is_idempotent() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let cspp = Cspp::new(FailNthStore::new());
        let active = deterministic_key(1);
        let staged = deterministic_key(2);
        cspp.save_master_key(&active).unwrap();
        let active_entries = cspp.read_active_entries().unwrap();

        cspp.save_staged_master_key(&staged).unwrap();
        let staged_entries = cspp.read_staged_entries().unwrap();
        cspp.save_staged_master_key(&staged).unwrap();

        assert_eq!(cspp.read_active_entries().unwrap(), active_entries);
        assert_eq!(cspp.read_staged_entries().unwrap(), staged_entries);
        assert_eq!(cspp.load_staged_master_key().unwrap().unwrap().as_bytes(), staged.as_bytes());
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), active.as_bytes());
        assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    }

    #[test]
    fn staged_master_key_rejects_replacement_and_cleans_partial_entries() {
        let cspp = Cspp::new(FailNthStore::new());
        let staged = deterministic_key(3);
        cspp.save_staged_master_key(&staged).unwrap();
        let staged_entries = cspp.read_staged_entries().unwrap();

        assert!(cspp.save_staged_master_key(&deterministic_key(4)).is_err());
        assert_eq!(cspp.read_staged_entries().unwrap(), staged_entries);

        cspp.discard_staged_master_key().unwrap();
        cspp.0.save(STAGED_MASTER_KEY_NAME.into(), "partial".into()).unwrap();

        assert!(cspp.save_staged_master_key(&staged).is_err());
        assert!(cspp.read_staged_entries().unwrap().is_absent());
    }

    #[test]
    fn active_and_staged_saves_restore_exact_entries_after_failure() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(5);
        cspp.save_master_key(&prior).unwrap();
        let prior_entries = cspp.read_active_entries().unwrap();

        store.fail_nth_mutation(2);
        assert!(cspp.save_master_key(&deterministic_key(6)).is_err());
        assert_eq!(cspp.read_active_entries().unwrap(), prior_entries);
        assert_eq!(cspp.get_or_create_master_key().unwrap().as_bytes(), prior.as_bytes());

        store.fail_nth_mutation(2);
        assert!(cspp.save_staged_master_key(&deterministic_key(7)).is_err());
        assert!(cspp.read_staged_entries().unwrap().is_absent());
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
        let prior_entries = cspp.read_active_entries().unwrap();
        cspp.save_staged_master_key(&staged).unwrap();

        cspp.promote_staged_master_key().unwrap();
        let promoted_entries = cspp.read_active_entries().unwrap();
        cspp.promote_staged_master_key().unwrap();

        assert_eq!(cspp.read_active_entries().unwrap(), promoted_entries);
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), staged.as_bytes());

        recreated.rollback_master_key_promotion().unwrap();
        recreated.rollback_master_key_promotion().unwrap();
        assert_eq!(recreated.read_active_entries().unwrap(), prior_entries);
        assert!(recreated.read_staged_entries().unwrap().is_absent());
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

        assert!(empty_cspp.read_active_entries().unwrap().is_absent());
        assert!(MASTER_KEY_CACHE.load().is_none());

        let partial_store = FailNthStore::with_entries(vec![(MASTER_KEY_NAME, "prior-raw")]);
        let partial_cspp = Cspp::new(partial_store);
        let partial_prior = partial_cspp.read_active_entries().unwrap();
        partial_cspp.save_staged_master_key(&deterministic_key(11)).unwrap();
        partial_cspp.promote_staged_master_key().unwrap();
        partial_cspp.rollback_master_key_promotion().unwrap();

        assert_eq!(partial_cspp.read_active_entries().unwrap(), partial_prior);
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
        let prior_entries = cspp.read_active_entries().unwrap();
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

        assert_eq!(recreated.read_active_entries().unwrap(), prior_entries);
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
        assert!(recreated.read_staged_entries().unwrap().is_absent());
        assert_eq!(recreated.get_or_create_master_key().unwrap().as_bytes(), staged.as_bytes());
    }

    #[test]
    fn promotion_journal_owns_staged_key_after_interrupted_staging_clear() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(25);
        let staged = deterministic_key(26);
        let staged_namespace = staged.namespace_id();
        let prior_namespace = prior.namespace_id();
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&staged).unwrap();
        cspp.promote_staged_master_key().unwrap();

        store.fail_nth_mutation(3);
        assert!(cspp.commit_master_key_promotion().is_err());
        assert!(cspp.read_staged_entries().unwrap().is_absent());
        assert_eq!(
            cspp.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
        );

        Cspp::<FailNthStore>::clear_cached_master_key();
        let recreated = Cspp::new(store);
        let evidence = recreated
            .master_key_promotion_evidence(&staged_namespace, Some(&prior_namespace))
            .unwrap();

        assert!(evidence.staged_matches_expected);
        assert!(evidence.prior_matches_expected);
        assert_eq!(
            recreated.load_staged_master_key().unwrap().unwrap().as_bytes(),
            staged.as_bytes()
        );

        recreated.promote_staged_master_key().unwrap();
        assert!(!recreated.read_staged_entries().unwrap().is_absent());
        recreated.commit_master_key_promotion().unwrap();

        assert_eq!(
            recreated.master_key_promotion_status().unwrap(),
            MasterKeyPromotionStatus::None
        );
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
        let promoted = cspp.read_active_entries().unwrap();

        assert!(cspp.save_master_key(&deterministic_key(18)).is_err());
        assert_eq!(cspp.read_active_entries().unwrap(), promoted);
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
        assert!(cspp.read_active_entries().unwrap().is_absent());
        assert!(cspp.read_staged_entries().unwrap().is_absent());
        assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
        assert!(MASTER_KEY_CACHE.load().is_none());
    }

    const ACTIVE_AND_JOURNAL_SLOTS: [&str; 3] =
        [MASTER_KEY_NAME, MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, MASTER_KEY_PROMOTION_JOURNAL];

    /// Store states that hold a master key the user depends on
    fn stores_with_existing_keys() -> Vec<(&'static str, FailNthStore, MasterKey)> {
        let active_only = FailNthStore::new();
        Cspp::new(active_only.clone()).save_master_key(&deterministic_key(30)).unwrap();

        let promoted = FailNthStore::new();
        let cspp = Cspp::new(promoted.clone());
        cspp.save_master_key(&deterministic_key(31)).unwrap();
        cspp.save_staged_master_key(&deterministic_key(32)).unwrap();
        cspp.promote_staged_master_key().unwrap();

        let retrying = FailNthStore::new();
        let cspp = Cspp::new(retrying.clone());
        cspp.save_master_key(&deterministic_key(33)).unwrap();
        cspp.save_staged_master_key(&deterministic_key(34)).unwrap();
        cspp.promote_staged_master_key().unwrap();
        cspp.restore_prior_master_key_for_retry().unwrap();

        vec![
            ("active key", active_only, deterministic_key(30)),
            ("promoted key", promoted, deterministic_key(32)),
            ("prior key restored for retry", retrying, deterministic_key(33)),
        ]
    }

    #[test]
    fn unreadable_slot_never_replaces_an_existing_master_key() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();

        for (scenario, store, active_key) in stores_with_existing_keys() {
            for slot in ALL_ENTRY_NAMES {
                Cspp::<FailNthStore>::clear_cached_master_key();
                let before = store.snapshot();
                store.fail_reads_of(slot);

                let cspp = Cspp::new(store.clone());
                match cspp.get_or_create_master_key() {
                    Ok(key) => assert_eq!(
                        key.as_bytes(),
                        active_key.as_bytes(),
                        "{scenario}: unreadable {slot} returned a different key"
                    ),
                    Err(error) => assert!(
                        matches!(error, CsppError::Read(_)),
                        "{scenario}: unreadable {slot} returned {error}"
                    ),
                }

                assert_eq!(store.snapshot(), before, "{scenario}: unreadable {slot} wrote");

                store.clear_read_failures();
                Cspp::<FailNthStore>::clear_cached_master_key();
                assert_eq!(
                    cspp.get_or_create_master_key().unwrap().as_bytes(),
                    active_key.as_bytes(),
                    "{scenario}: key changed after unreadable {slot}"
                );
            }
        }
    }

    #[test]
    fn unreadable_slot_on_empty_store_does_not_create_a_master_key() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();

        for slot in ACTIVE_AND_JOURNAL_SLOTS {
            Cspp::<FailNthStore>::clear_cached_master_key();
            let store = FailNthStore::new();
            store.fail_reads_of(slot);
            let cspp = Cspp::new(store.clone());

            assert!(matches!(cspp.get_or_create_master_key(), Err(CsppError::Read(_))));
            assert!(store.snapshot().is_empty(), "unreadable {slot} created a key");
            assert!(MASTER_KEY_CACHE.load().is_none());
        }
    }

    #[test]
    fn unreadable_slot_reports_errors_instead_of_absence() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        for slot in [MASTER_KEY_NAME, MASTER_KEY_ENCRYPTION_KEY_AND_NONCE] {
            let store = FailNthStore::new();
            let cspp = Cspp::new(store.clone());
            cspp.save_master_key(&deterministic_key(35)).unwrap();
            store.fail_reads_of(slot);

            assert!(matches!(cspp.load_master_key_from_store(), Err(CsppError::Read(_))));
            assert!(matches!(cspp.has_master_key(), Err(CsppError::Read(_))));
            assert!(matches!(cspp.capture_active_master_key_snapshot(), Err(CsppError::Read(_))));
        }

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        cspp.save_staged_master_key(&deterministic_key(36)).unwrap();
        cspp.promote_staged_master_key().unwrap();
        store.fail_reads_of(MASTER_KEY_PROMOTION_JOURNAL);

        assert!(matches!(cspp.master_key_promotion_status(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.load_staged_master_key(), Err(CsppError::Read(_))));
    }

    #[test]
    fn promotion_with_unreadable_active_slot_keeps_prior_key_recoverable() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(37);
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&deterministic_key(38)).unwrap();
        let before = store.snapshot();

        // a journal recorded with an unread prior would make rollback delete the prior key
        store.fail_reads_of(MASTER_KEY_NAME);
        assert!(matches!(cspp.promote_staged_master_key(), Err(CsppError::Read(_))));
        assert_eq!(store.snapshot(), before);

        store.clear_read_failures();
        cspp.promote_staged_master_key().unwrap();
        cspp.rollback_master_key_promotion().unwrap();
        assert_eq!(
            cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
            prior.as_bytes()
        );
    }

    #[test]
    fn unreadable_promotion_journal_is_never_overwritten_or_discarded() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        let prior = deterministic_key(39);
        cspp.save_master_key(&prior).unwrap();
        cspp.save_staged_master_key(&deterministic_key(40)).unwrap();
        cspp.promote_staged_master_key().unwrap();
        let before = store.snapshot();
        store.fail_reads_of(MASTER_KEY_PROMOTION_JOURNAL);

        assert!(matches!(cspp.promote_staged_master_key(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.rollback_master_key_promotion(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.commit_master_key_promotion(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.restore_prior_master_key_for_retry(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.discard_staged_master_key(), Err(CsppError::Read(_))));
        assert!(matches!(cspp.save_master_key(&deterministic_key(41)), Err(CsppError::Read(_))));
        assert!(matches!(
            cspp.save_staged_master_key(&deterministic_key(42)),
            Err(CsppError::Read(_))
        ));
        assert_eq!(store.snapshot(), before);

        store.clear_read_failures();
        cspp.rollback_master_key_promotion().unwrap();
        assert_eq!(
            cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
            prior.as_bytes()
        );
    }

    #[test]
    fn save_with_unreadable_active_slot_leaves_entries_untouched() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();
        Cspp::<FailNthStore>::clear_cached_master_key();

        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());
        cspp.save_master_key(&deterministic_key(43)).unwrap();
        let before = store.snapshot();
        store.fail_reads_of(MASTER_KEY_ENCRYPTION_KEY_AND_NONCE);

        assert!(matches!(cspp.save_master_key(&deterministic_key(44)), Err(CsppError::Read(_))));
        assert_eq!(store.snapshot(), before);
    }

    #[test]
    fn delete_with_any_unreadable_slot_deletes_nothing() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();

        for (scenario, store, _) in stores_with_existing_keys() {
            for slot in ALL_ENTRY_NAMES {
                let before = store.snapshot();
                store.fail_reads_of(slot);

                assert!(!Cspp::new(store.clone()).delete_master_key(), "{scenario}: {slot}");
                assert_eq!(store.snapshot(), before, "{scenario}: unreadable {slot} deleted");
                store.clear_read_failures();
            }
        }
    }

    #[test]
    fn staged_discard_with_an_unreadable_slot_leaves_the_pair_intact() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();

        for slot in [STAGED_MASTER_KEY_ENCRYPTION_KEY_AND_NONCE, STAGED_MASTER_KEY_NAME] {
            let store = FailNthStore::new();
            let cspp = Cspp::new(store.clone());
            cspp.save_staged_master_key(&deterministic_key(49)).unwrap();
            let before = store.snapshot();
            store.fail_reads_of(slot);

            assert!(matches!(cspp.discard_staged_master_key(), Err(CsppError::Read(_))), "{slot}");
            assert!(
                matches!(cspp.rollback_master_key_promotion(), Err(CsppError::Read(_))),
                "{slot}"
            );
            assert_eq!(store.snapshot(), before, "unreadable {slot} left a half-deleted pair");

            store.clear_read_failures();
            assert!(
                cspp.load_staged_master_key().unwrap().unwrap().as_bytes()
                    == deterministic_key(49).as_bytes()
            );
        }
    }

    #[test]
    fn failed_staged_write_rollback_keeps_the_read_error() {
        let store = FailNthStore::new();
        let cspp = Cspp::new(store.clone());

        // the staged write and its rollback both find the staged slot unreadable
        store.fail_reads_after_next_save(STAGED_MASTER_KEY_NAME);

        let result = cspp.save_staged_master_key(&deterministic_key(51));
        assert!(matches!(result, Err(CsppError::Read(_))), "{result:?}");
    }

    #[test]
    fn failed_read_back_after_active_write_leaves_cache_empty() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap();

        // each scenario writes the active slots and then reads them back
        type Operation = fn(&Cspp<FailNthStore>) -> Result<(), CsppError>;

        let scenarios: [(&str, Operation); 4] = [
            ("save", |cspp| cspp.save_master_key(&deterministic_key(47))),
            ("promote", Cspp::promote_staged_master_key),
            ("retry restore", Cspp::restore_prior_master_key_for_retry),
            ("rollback", Cspp::rollback_master_key_promotion),
        ];

        for (name, operation) in scenarios {
            Cspp::<FailNthStore>::clear_cached_master_key();
            let store = FailNthStore::new();
            let cspp = Cspp::new(store.clone());
            cspp.save_master_key(&deterministic_key(46)).unwrap();
            if name != "save" {
                cspp.save_staged_master_key(&deterministic_key(48)).unwrap();
            }
            if name == "retry restore" || name == "rollback" {
                cspp.promote_staged_master_key().unwrap();
            }
            assert!(cspp.get_or_create_master_key().is_ok());
            assert!(MASTER_KEY_CACHE.load().is_some());

            // writes succeed, then the read-back of the active slot fails
            store.fail_reads_after_next_save(MASTER_KEY_NAME);

            let result = operation(&cspp);
            assert!(matches!(result, Err(CsppError::Read(_))), "{name} lost the read error");

            assert!(MASTER_KEY_CACHE.load().is_none(), "{name} left a stale cached key");
        }
    }
}
