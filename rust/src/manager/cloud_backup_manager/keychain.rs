use cove_cspp::backup_data::PasskeyProviderHint;
use cove_cspp::{ActiveMasterKeySnapshot, Cspp, CsppStore as _};
use cove_device::keychain::{Keychain, KeychainError};
use cove_util::json::JsonDecodeError;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::{
    PENDING_ENABLE_JOURNAL_VERSION, PendingEnableJournal, PendingEnableLocalMetadataSnapshot,
};

pub(crate) const CSPP_CREDENTIAL_ID_KEY: &str = "cspp::v1::credential_id";
pub(crate) const CSPP_PRF_SALT_KEY: &str = "cspp::v1::prf_salt";
pub(crate) const CSPP_NAMESPACE_ID_KEY: &str = "cspp::v1::namespace_id";
pub(crate) const CSPP_PENDING_ENABLE_JOURNAL_KEY: &str = "cspp::v1::pending_enable_journal";
pub(crate) const CSPP_PASSKEY_PROVIDER_HINT_KEY: &str = "cspp::v1::passkey_provider_hint";

/// Where a passkey was created, stored with the credential it describes
///
/// A later credential saved without a hint leaves this entry naming the earlier credential, so
/// it is never read as the hint of a credential it doesn't describe
#[derive(Debug, Serialize, Deserialize)]
struct StoredProviderHint {
    credential_id: String,
    hint: PasskeyProviderHint,
}

impl StoredProviderHint {
    fn encode(
        credential_id: &[u8],
        hint: &PasskeyProviderHint,
    ) -> Result<String, CloudBackupKeychainError> {
        let stored = Self { credential_id: hex::encode(credential_id), hint: hint.clone() };
        serde_json::to_string(&stored).map_err(CloudBackupKeychainError::EncodeProviderHint)
    }

    /// The hint, when the stored value is well formed and describes `credential_id`
    fn decode_for(value: &str, credential_id: &[u8]) -> Option<PasskeyProviderHint> {
        let stored: Self = serde_json::from_str(value)
            .inspect_err(|error| {
                let error = JsonDecodeError::from(error);
                warn!("Failed to decode stored passkey provider hint: {error}");
            })
            .ok()?;

        (stored.credential_id == hex::encode(credential_id)).then_some(stored.hint)
    }
}

#[derive(Clone)]
pub(crate) struct RestoreActivationKeychainSnapshot {
    metadata: PendingEnableLocalMetadataSnapshot,
    master_key: ActiveMasterKeySnapshot,
}

impl PendingEnableLocalMetadataSnapshot {
    /// The keychain entries this snapshot restores, in save order
    fn keychain_entries(&self) -> [(String, Option<String>); 4] {
        [
            (CSPP_CREDENTIAL_ID_KEY.into(), self.credential_id.clone()),
            (CSPP_PRF_SALT_KEY.into(), self.prf_salt.clone()),
            (CSPP_NAMESPACE_ID_KEY.into(), self.namespace_id.clone()),
            (CSPP_PASSKEY_PROVIDER_HINT_KEY.into(), self.provider_hint.clone()),
        ]
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupKeychain(Keychain);

#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudBackupKeychainError {
    #[error("{0}")]
    Keychain(#[from] KeychainError),

    #[error("{0}")]
    MasterKey(#[from] cove_cspp::CsppError),

    #[error(
        "failed to roll back cloud backup keychain save after original error: {original}; rollback error: {rollback}"
    )]
    RollbackFailed { original: KeychainError, rollback: KeychainError },

    #[error("could not encode pending Cloud Backup enable state: {0}")]
    EncodePendingEnable(serde_json::Error),

    #[error("could not encode the passkey provider hint: {0}")]
    EncodeProviderHint(serde_json::Error),

    /// The journal holds passkey data, so only the parse position is kept
    #[error("could not decode pending Cloud Backup enable state: {0}")]
    DecodePendingEnable(JsonDecodeError),

    #[error("unsupported pending Cloud Backup enable state version {0}")]
    UnsupportedPendingEnableVersion(u8),

    #[error("failed to restore {}", describe_restore_activation_failure(.metadata, .master_key))]
    RestoreActivation { metadata: Option<KeychainError>, master_key: Option<cove_cspp::CsppError> },
}

fn describe_restore_activation_failure(
    metadata: &Option<KeychainError>,
    master_key: &Option<cove_cspp::CsppError>,
) -> String {
    let parts = [
        metadata.as_ref().map(|error| format!("Cloud Backup metadata ({error})")),
        master_key.as_ref().map(|error| format!("the CSPP master key ({error})")),
    ];

    parts.into_iter().flatten().collect::<Vec<_>>().join(" and ")
}

/// Who may clear Cloud Backup's local keychain state, including its namespace
///
/// Only one of them can exist at a time: the recovery-cleanup permit is granted only while no
/// Cloud Backup manager exists, and bootstrap waits while it is held. So the namespace has one
/// owner at any moment, and `commit_disable` can read it before its transaction
pub(crate) enum LocalStateOwner<'a> {
    /// The lifecycle owner, in a handler or an `Effecting` stage
    Lifecycle(
        #[expect(dead_code, reason = "a witness: holding the token is the proof")]
        &'a crate::manager::cloud_backup_manager::actors::supervisor::OwnerToken,
    ),
    /// Catastrophic recovery, before any Cloud Backup manager exists
    RecoveryCleanup(
        #[expect(dead_code, reason = "a witness: holding the permit is the proof")]
        &'a crate::manager::cloud_backup_manager::CloudBackupRecoveryCleanupPermit,
    ),
}

impl CloudBackupKeychain {
    pub(crate) fn global() -> Self {
        Self::new(Keychain::global().clone())
    }

    pub(crate) fn new(keychain: Keychain) -> Self {
        Self(keychain)
    }

    pub(crate) fn namespace_id(&self) -> Result<Option<String>, KeychainError> {
        self.0.get(CSPP_NAMESPACE_ID_KEY.into())
    }

    pub(crate) fn save_namespace_id(
        &self,
        namespace_id: &str,
    ) -> Result<(), CloudBackupKeychainError> {
        self.0.save(CSPP_NAMESPACE_ID_KEY.into(), namespace_id.to_owned())?;
        Ok(())
    }

    pub(crate) fn has_prf_salt(&self) -> Result<bool, KeychainError> {
        Ok(self.0.get(CSPP_PRF_SALT_KEY.into())?.is_some())
    }

    /// Saves a newly promoted passkey with its namespace, and the hint of where it was created
    ///
    /// Without a hint, an earlier credential's hint stays stored but no longer applies
    pub(crate) fn save_passkey_namespace_and_hint(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
        namespace_id: &str,
        provider_hint: Option<&PasskeyProviderHint>,
    ) -> Result<(), CloudBackupKeychainError> {
        let mut entries = Self::passkey_entries(credential_id, prf_salt, provider_hint)?;
        entries.push((CSPP_NAMESPACE_ID_KEY, namespace_id.to_owned()));

        self.save_entries_with_rollback(&entries)
    }

    /// Saves a passkey that replaces this device's, with the hint of where it was created, in one
    /// rollback-protected write
    pub(crate) fn save_passkey_and_hint(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
        provider_hint: Option<&PasskeyProviderHint>,
    ) -> Result<(), CloudBackupKeychainError> {
        let entries = Self::passkey_entries(credential_id, prf_salt, provider_hint)?;
        self.save_entries_with_rollback(&entries)
    }

    /// The entries that save a credential, its salt and, when known, its hint
    fn passkey_entries(
        credential_id: &[u8],
        prf_salt: [u8; 32],
        provider_hint: Option<&PasskeyProviderHint>,
    ) -> Result<Vec<(&'static str, String)>, CloudBackupKeychainError> {
        let mut entries = vec![
            (CSPP_CREDENTIAL_ID_KEY, hex::encode(credential_id)),
            (CSPP_PRF_SALT_KEY, hex::encode(prf_salt)),
        ];
        if let Some(hint) = provider_hint {
            entries.push((
                CSPP_PASSKEY_PROVIDER_HINT_KEY,
                StoredProviderHint::encode(credential_id, hint)?,
            ));
        }

        Ok(entries)
    }

    /// Where the passkey with `credential_id` was created, when this device stored that
    ///
    /// Reusing a credential keeps its hint, since the hint belongs to the credential
    pub(crate) fn provider_hint_for(
        &self,
        credential_id: &[u8],
    ) -> Result<Option<PasskeyProviderHint>, KeychainError> {
        Ok(self
            .0
            .get(CSPP_PASSKEY_PROVIDER_HINT_KEY.into())?
            .and_then(|value| StoredProviderHint::decode_for(&value, credential_id)))
    }

    /// [`Self::provider_hint_for`], with an unreadable keychain read as no hint
    ///
    /// The hint only says where the passkey was created, so failing to read it must not fail
    /// the passkey request that reuses the credential
    pub(crate) fn known_provider_hint(&self, credential_id: &[u8]) -> Option<PasskeyProviderHint> {
        self.provider_hint_for(credential_id)
            .inspect_err(|error| warn!("Failed to read the passkey provider hint: {error}"))
            .ok()
            .flatten()
    }

    /// Where this device's passkey was created, when this device stored that
    ///
    /// Read from the credential the keychain holds now, so it never describes a replaced one. An
    /// unreadable entry reads as no hint, since the hint is only shown to the user
    pub(crate) fn active_provider_hint(&self) -> Option<PasskeyProviderHint> {
        let credential_id = self
            .0
            .get(CSPP_CREDENTIAL_ID_KEY.into())
            .inspect_err(|error| warn!("Failed to read the passkey credential: {error}"))
            .ok()
            .flatten()?;

        let credential_id = hex::decode(credential_id).ok()?;
        self.known_provider_hint(&credential_id)
    }

    /// Captures the entries a failed restore activation must put back
    ///
    /// Fails when any entry cannot be read, since restoring an unread entry would delete it
    pub(crate) fn capture_restore_activation_snapshot(
        &self,
    ) -> Result<RestoreActivationKeychainSnapshot, CloudBackupKeychainError> {
        Ok(RestoreActivationKeychainSnapshot {
            metadata: self.snapshot_passkey_metadata()?,
            master_key: Cspp::new(self.0.clone()).capture_active_master_key_snapshot()?,
        })
    }

    pub(crate) fn restore_activation_snapshot(
        &self,
        snapshot: &RestoreActivationKeychainSnapshot,
    ) -> Result<(), CloudBackupKeychainError> {
        let metadata = self.restore_all_entries(&snapshot.metadata.keychain_entries());
        let master_key =
            Cspp::new(self.0.clone()).restore_active_master_key_snapshot(&snapshot.master_key);

        if metadata.is_ok() && master_key.is_ok() {
            return Ok(());
        }

        Err(CloudBackupKeychainError::RestoreActivation {
            metadata: metadata.err(),
            master_key: master_key.err(),
        })
    }

    /// Captures passkey metadata for a later exact restore
    ///
    /// Fails when any entry cannot be read, since restoring an unread entry would delete it
    pub(crate) fn snapshot_passkey_metadata(
        &self,
    ) -> Result<PendingEnableLocalMetadataSnapshot, KeychainError> {
        Ok(PendingEnableLocalMetadataSnapshot {
            credential_id: self.0.get(CSPP_CREDENTIAL_ID_KEY.into())?,
            prf_salt: self.0.get(CSPP_PRF_SALT_KEY.into())?,
            namespace_id: self.0.get(CSPP_NAMESPACE_ID_KEY.into())?,
            provider_hint: self.0.get(CSPP_PASSKEY_PROVIDER_HINT_KEY.into())?,
        })
    }

    pub(crate) fn restore_passkey_metadata(
        &self,
        snapshot: &PendingEnableLocalMetadataSnapshot,
    ) -> Result<(), CloudBackupKeychainError> {
        self.restore_entries(&snapshot.keychain_entries())?;
        Ok(())
    }

    pub(crate) fn save_pending_enable_journal(
        &self,
        journal: &PendingEnableJournal,
    ) -> Result<(), CloudBackupKeychainError> {
        let encoded = serde_json::to_string(journal)
            .map_err(CloudBackupKeychainError::EncodePendingEnable)?;
        self.0.save(CSPP_PENDING_ENABLE_JOURNAL_KEY.into(), encoded)?;
        super::pending_enable::evidence::journal_written(journal);
        Ok(())
    }

    pub(crate) fn load_pending_enable_journal(
        &self,
    ) -> Result<Option<PendingEnableJournal>, CloudBackupKeychainError> {
        let Some(encoded) = self.0.get(CSPP_PENDING_ENABLE_JOURNAL_KEY.into())? else {
            super::pending_enable::evidence::journal_read(None);
            return Ok(None);
        };

        let journal: PendingEnableJournal = serde_json::from_str(&encoded)
            .map_err(|error| CloudBackupKeychainError::DecodePendingEnable(error.into()))?;

        if journal.version() != PENDING_ENABLE_JOURNAL_VERSION {
            return Err(CloudBackupKeychainError::UnsupportedPendingEnableVersion(
                journal.version(),
            ));
        }

        super::pending_enable::evidence::journal_read(Some(&journal));
        Ok(Some(journal))
    }

    pub(crate) fn delete_pending_enable_journal(&self) -> Result<(), CloudBackupKeychainError> {
        self.delete_keychain_item_if_present(CSPP_PENDING_ENABLE_JOURNAL_KEY)?;
        super::pending_enable::evidence::journal_deleted();
        Ok(())
    }

    /// Loads the stored passkey credential ID, or `None` when it is absent or malformed
    ///
    /// # Errors
    ///
    /// Returns an error when the keychain cannot be read
    pub(crate) fn load_credential_id(&self) -> Result<Option<Vec<u8>>, KeychainError> {
        Ok(self.0.get(CSPP_CREDENTIAL_ID_KEY.into())?.and_then(|hex_str| {
            hex::decode(hex_str)
                .inspect_err(|error| {
                    warn!("Failed to decode stored cloud backup passkey metadata: {error}")
                })
                .ok()
        }))
    }

    /// Loads the stored PRF salt, or `None` when it is absent or malformed
    ///
    /// # Errors
    ///
    /// Returns an error when the keychain cannot be read
    pub(crate) fn load_prf_salt(&self) -> Result<Option<[u8; 32]>, KeychainError> {
        Ok(self.0.get(CSPP_PRF_SALT_KEY.into())?.and_then(|hex_str| {
            hex::decode(hex_str)
                .inspect_err(|error| {
                    warn!("Failed to decode stored cloud backup passkey metadata: {error}")
                })
                .ok()
                .and_then(|bytes| {
                    bytes
                        .try_into()
                        .inspect_err(|_| {
                            warn!("Stored cloud backup passkey metadata has invalid length")
                        })
                        .ok()
                })
        }))
    }

    pub(crate) fn delete_master_key(&self) -> Result<(), CloudBackupKeychainError> {
        let cspp = cove_cspp::Cspp::new(self.0.clone());
        if !cspp.delete_master_key() {
            warn!("Failed to delete cloud backup master key");
            return Err(CloudBackupKeychainError::Keychain(KeychainError::Delete));
        }

        Ok(())
    }

    fn delete_namespace_id(&self) -> Result<(), CloudBackupKeychainError> {
        self.delete_keychain_item_if_present(CSPP_NAMESPACE_ID_KEY)?;
        Ok(())
    }

    pub(crate) fn delete_credential_id(&self) -> Result<(), CloudBackupKeychainError> {
        self.delete_keychain_item_if_present(CSPP_CREDENTIAL_ID_KEY)?;
        Ok(())
    }

    pub(crate) fn delete_prf_salt(&self) -> Result<(), CloudBackupKeychainError> {
        self.delete_keychain_item_if_present(CSPP_PRF_SALT_KEY)?;
        Ok(())
    }

    fn delete_provider_hint(&self) -> Result<(), CloudBackupKeychainError> {
        self.delete_keychain_item_if_present(CSPP_PASSKEY_PROVIDER_HINT_KEY)?;
        Ok(())
    }

    pub(crate) fn clear_local_state(
        &self,
        _owner: &LocalStateOwner<'_>,
    ) -> Result<(), CloudBackupKeychainError> {
        let mut failed = false;

        if self.delete_master_key().is_err() {
            failed = true;
        }

        if self.delete_namespace_id().is_err() {
            failed = true;
        }

        if self.delete_credential_id().is_err() {
            failed = true;
        }

        if self.delete_prf_salt().is_err() {
            failed = true;
        }

        if self.delete_provider_hint().is_err() {
            failed = true;
        }

        if self.delete_pending_enable_journal().is_err() {
            failed = true;
        }

        if failed {
            return Err(CloudBackupKeychainError::Keychain(KeychainError::Delete));
        }

        Ok(())
    }

    fn save_entries_with_rollback(
        &self,
        entries: &[(&str, String)],
    ) -> Result<(), CloudBackupKeychainError> {
        // rollback restores these exact values, so an unread value must stop the save
        let previous_values = entries
            .iter()
            .map(|(key, _)| Ok(((*key).to_owned(), self.0.get((*key).into())?)))
            .collect::<Result<Vec<_>, KeychainError>>()?;

        for (key, value) in entries {
            if let Err(error) = self.0.save((*key).to_owned(), value.clone()) {
                if let Err(rollback) = self.restore_entries(&previous_values) {
                    return Err(CloudBackupKeychainError::RollbackFailed {
                        original: error,
                        rollback,
                    });
                }
                return Err(CloudBackupKeychainError::Keychain(error));
            }
        }

        Ok(())
    }

    fn restore_entries(
        &self,
        previous_values: &[(String, Option<String>)],
    ) -> Result<(), KeychainError> {
        for (key, previous_value) in previous_values {
            self.restore_entry(key, previous_value.as_deref())?;
        }

        Ok(())
    }

    fn restore_all_entries(
        &self,
        previous_values: &[(String, Option<String>)],
    ) -> Result<(), KeychainError> {
        let mut first_error = None;

        for (key, previous_value) in previous_values {
            if let Err(error) = self.restore_entry(key, previous_value.as_deref()) {
                first_error.get_or_insert(error);
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    fn restore_entry(&self, key: &str, previous_value: Option<&str>) -> Result<(), KeychainError> {
        match previous_value {
            Some(value) => self.0.save(key.to_owned(), value.to_owned()),
            None => self.delete_keychain_item_if_present(key),
        }
    }

    fn delete_keychain_item_if_present(&self, key: &str) -> Result<(), KeychainError> {
        if self.0.get(key.to_owned())?.is_some() && !self.0.delete(key.to_owned()) {
            warn!("Failed to delete cloud backup keychain item");
            return Err(KeychainError::Delete);
        }

        Ok(())
    }
}

#[cfg(test)]
mod test_support {
    use super::{CloudBackupKeychain, CloudBackupKeychainError};

    impl CloudBackupKeychain {
        /// Saves a passkey and its namespace without a provider hint, as a device set up before
        /// hints were stored has them
        pub(crate) fn save_passkey_and_namespace(
            &self,
            credential_id: &[u8],
            prf_salt: [u8; 32],
            namespace_id: &str,
        ) -> Result<(), CloudBackupKeychainError> {
            self.save_passkey_namespace_and_hint(credential_id, prf_salt, namespace_id, None)
        }
    }
}
