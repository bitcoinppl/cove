use cove_cspp::CsppStore as _;
use cove_device::keychain::{Keychain, KeychainError};
use tracing::warn;

pub(crate) const CSPP_CREDENTIAL_ID_KEY: &str = "cspp::v1::credential_id";
pub(crate) const CSPP_PRF_SALT_KEY: &str = "cspp::v1::prf_salt";
pub(crate) const CSPP_NAMESPACE_ID_KEY: &str = "cspp::v1::namespace_id";

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupKeychain(Keychain);

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

    pub(crate) fn save_namespace_id(&self, namespace_id: &str) -> Result<(), KeychainError> {
        self.0.save(CSPP_NAMESPACE_ID_KEY.into(), namespace_id.to_owned())
    }

    pub(crate) fn has_prf_salt(&self) -> bool {
        self.0.get(CSPP_PRF_SALT_KEY.into()).is_some()
    }

    pub(crate) fn save_passkey(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<(), KeychainError> {
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
    ) -> Result<(), KeychainError> {
        self.save_entries_with_rollback(&[
            (CSPP_CREDENTIAL_ID_KEY, hex::encode(credential_id)),
            (CSPP_PRF_SALT_KEY, hex::encode(prf_salt)),
            (CSPP_NAMESPACE_ID_KEY, namespace_id.to_owned()),
        ])
    }

    pub(crate) fn load_credential_id(&self) -> Option<Vec<u8>> {
        self.0.get(CSPP_CREDENTIAL_ID_KEY.into()).and_then(|hex_str| {
            hex::decode(hex_str)
                .inspect_err(|error| warn!("Failed to decode stored credential_id: {error}"))
                .ok()
        })
    }

    pub(crate) fn clear_passkey(&self) {
        self.0.delete(CSPP_CREDENTIAL_ID_KEY.into());
        self.0.delete(CSPP_PRF_SALT_KEY.into());
    }

    pub(crate) fn clear_local_state(&self) {
        self.delete_keychain_item_if_present(CSPP_NAMESPACE_ID_KEY);
        self.delete_keychain_item_if_present(CSPP_CREDENTIAL_ID_KEY);
        self.delete_keychain_item_if_present(CSPP_PRF_SALT_KEY);

        let cspp = cove_cspp::Cspp::new(self.0.clone());
        if !cspp.delete_master_key() {
            warn!("Failed to delete cloud backup master key from keychain");
        }
    }

    fn save_entries_with_rollback(&self, entries: &[(&str, String)]) -> Result<(), KeychainError> {
        let previous_values: Vec<_> =
            entries.iter().map(|(key, _)| ((*key).to_owned(), self.0.get((*key).into()))).collect();

        for (key, value) in entries {
            if let Err(error) = self.0.save((*key).to_owned(), value.clone()) {
                self.restore_entries(&previous_values);
                return Err(error);
            }
        }

        Ok(())
    }

    fn restore_entries(&self, previous_values: &[(String, Option<String>)]) {
        for (key, previous_value) in previous_values {
            match previous_value {
                Some(value) => {
                    let _ = self.0.save(key.clone(), value.clone());
                }
                None => {
                    self.0.delete(key.clone());
                }
            }
        }
    }

    fn delete_keychain_item_if_present(&self, key: &str) {
        if self.0.get(key.to_owned()).is_some() && !self.0.delete(key.to_owned()) {
            warn!("Failed to delete cloud backup keychain item key={key}");
        }
    }
}
