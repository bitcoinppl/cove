use cove_cspp::CsppStore as _;
use cove_device::keychain::{Keychain, KeychainError};
use tracing::warn;

pub(crate) const CSPP_CREDENTIAL_ID_KEY: &str = "cspp::v1::credential_id";
pub(crate) const CSPP_PRF_SALT_KEY: &str = "cspp::v1::prf_salt";
pub(crate) const CSPP_NAMESPACE_ID_KEY: &str = "cspp::v1::namespace_id";

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupKeychain(Keychain);

#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudBackupKeychainError {
    #[error("{0}")]
    Keychain(#[from] KeychainError),

    #[error(
        "failed to roll back cloud backup keychain save after original error: {original}; rollback error: {rollback}"
    )]
    RollbackFailed { original: KeychainError, rollback: KeychainError },
}

impl CloudBackupKeychain {
    pub(crate) fn global() -> Self {
        Self::new(Keychain::global().clone())
    }

    pub(crate) fn new(keychain: Keychain) -> Self {
        Self(keychain)
    }

    pub(crate) fn namespace_id(&self) -> Option<String> {
        self.0.get(CSPP_NAMESPACE_ID_KEY.into())
    }

    pub(crate) fn save_namespace_id(
        &self,
        namespace_id: &str,
    ) -> Result<(), CloudBackupKeychainError> {
        self.0.save(CSPP_NAMESPACE_ID_KEY.into(), namespace_id.to_owned())?;
        Ok(())
    }

    pub(crate) fn has_prf_salt(&self) -> bool {
        self.0.get(CSPP_PRF_SALT_KEY.into()).is_some()
    }

    pub(crate) fn save_passkey(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<(), CloudBackupKeychainError> {
        self.save_entries_with_rollback(&[
            (CSPP_CREDENTIAL_ID_KEY, hex::encode(credential_id)),
            (CSPP_PRF_SALT_KEY, hex::encode(prf_salt)),
        ])
    }

    pub(crate) fn save_passkey_and_namespace(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
        namespace_id: &str,
    ) -> Result<(), CloudBackupKeychainError> {
        self.save_entries_with_rollback(&[
            (CSPP_CREDENTIAL_ID_KEY, hex::encode(credential_id)),
            (CSPP_PRF_SALT_KEY, hex::encode(prf_salt)),
            (CSPP_NAMESPACE_ID_KEY, namespace_id.to_owned()),
        ])
    }

    pub(crate) fn load_credential_id(&self) -> Option<Vec<u8>> {
        self.0.get(CSPP_CREDENTIAL_ID_KEY.into()).and_then(|hex_str| {
            hex::decode(hex_str)
                .inspect_err(|error| {
                    warn!("Failed to decode stored cloud backup passkey metadata: {error}")
                })
                .ok()
        })
    }

    pub(crate) fn load_prf_salt(&self) -> Option<[u8; 32]> {
        self.0.get(CSPP_PRF_SALT_KEY.into()).and_then(|hex_str| {
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
        })
    }

    pub(crate) fn delete_master_key(&self) -> Result<(), CloudBackupKeychainError> {
        let cspp = cove_cspp::Cspp::new(self.0.clone());
        if !cspp.delete_master_key() {
            warn!("Failed to delete cloud backup master key");
            return Err(CloudBackupKeychainError::Keychain(KeychainError::Delete));
        }

        Ok(())
    }

    pub(crate) fn delete_namespace_id(&self) -> Result<(), CloudBackupKeychainError> {
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

    pub(crate) fn clear_local_state(&self) -> Result<(), CloudBackupKeychainError> {
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

        if failed {
            return Err(CloudBackupKeychainError::Keychain(KeychainError::Delete));
        }

        Ok(())
    }

    fn save_entries_with_rollback(
        &self,
        entries: &[(&str, String)],
    ) -> Result<(), CloudBackupKeychainError> {
        let previous_values: Vec<_> =
            entries.iter().map(|(key, _)| ((*key).to_owned(), self.0.get((*key).into()))).collect();

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
            match previous_value {
                Some(value) => {
                    self.0.save(key.clone(), value.clone())?;
                }
                None => {
                    if self.0.get(key.clone()).is_some() && !self.0.delete(key.clone()) {
                        return Err(KeychainError::Delete);
                    }
                }
            }
        }

        Ok(())
    }

    fn delete_keychain_item_if_present(&self, key: &str) -> Result<(), KeychainError> {
        if self.0.get(key.to_owned()).is_some() && !self.0.delete(key.to_owned()) {
            warn!("Failed to delete cloud backup keychain item");
            return Err(KeychainError::Delete);
        }

        Ok(())
    }
}
