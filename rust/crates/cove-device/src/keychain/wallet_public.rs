use std::str::FromStr as _;

use bdk_wallet::descriptor::ExtendedDescriptor;
use bitcoin::bip32::Xpub;

use cove_types::WalletId;

use super::{KeychainError, SavedValueParseError, SharedAccess};

#[derive(Debug, Clone)]
pub(crate) struct WalletPublicDataStore(SharedAccess);

impl WalletPublicDataStore {
    pub(crate) fn new(access: SharedAccess) -> Self {
        Self(access)
    }

    pub(crate) fn save_xpub(&self, id: &WalletId, xpub: Xpub) -> Result<(), KeychainError> {
        self.0.save(xpub_key_name(id), xpub.to_string())
    }

    pub(crate) fn get_xpub(&self, id: &WalletId) -> Result<Option<Xpub>, KeychainError> {
        let Some(value) = self.0.get(xpub_key_name(id))? else {
            return Ok(None);
        };

        let xpub = Xpub::from_str(&value)
            .map_err(|_| KeychainError::ParseSavedValue(SavedValueParseError::Xpub))?;

        Ok(Some(xpub))
    }

    pub(crate) fn delete_xpub(&self, id: &WalletId) -> bool {
        delete_if_present(&self.0, xpub_key_name(id))
    }

    pub(crate) fn save_descriptors(
        &self,
        id: &WalletId,
        external: ExtendedDescriptor,
        internal: ExtendedDescriptor,
    ) -> Result<(), KeychainError> {
        self.0.save(descriptor_key_name(id), format!("{external}\n{internal}"))
    }

    pub(crate) fn get_descriptors(
        &self,
        id: &WalletId,
    ) -> Result<Option<(ExtendedDescriptor, ExtendedDescriptor)>, KeychainError> {
        let Some(value) = self.0.get(descriptor_key_name(id))? else {
            return Ok(None);
        };

        let mut lines = value.lines();
        let external = lines.next().ok_or(KeychainError::ParseSavedValue(
            SavedValueParseError::MissingExternalDescriptor,
        ))?;

        let internal = lines.next().ok_or(KeychainError::ParseSavedValue(
            SavedValueParseError::MissingInternalDescriptor,
        ))?;

        let external = ExtendedDescriptor::from_str(external).map_err(|_| {
            KeychainError::ParseSavedValue(SavedValueParseError::ExternalDescriptor)
        })?;

        let internal = ExtendedDescriptor::from_str(internal).map_err(|_| {
            KeychainError::ParseSavedValue(SavedValueParseError::InternalDescriptor)
        })?;

        Ok(Some((external, internal)))
    }

    pub(crate) fn delete_descriptors(&self, id: &WalletId) -> bool {
        delete_if_present(&self.0, descriptor_key_name(id))
    }

    /// Reads both entries even when the first exists, so a caller can rely on a successful
    /// result meaning neither entry is unreadable
    pub(crate) fn has_any(&self, id: &WalletId) -> Result<bool, KeychainError> {
        let xpub = self.0.get(xpub_key_name(id))?;
        let descriptors = self.0.get(descriptor_key_name(id))?;

        Ok(xpub.is_some() || descriptors.is_some())
    }
}

/// Returns false when the entry could not be read, so an unreadable entry is never reported as
/// deleted
fn delete_if_present(access: &SharedAccess, key: String) -> bool {
    match access.get(key.clone()) {
        Ok(None) => true,
        Ok(Some(_)) => access.delete(key),
        Err(_) => false,
    }
}

fn xpub_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_xpub")
}

fn descriptor_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_public_descriptor")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bdk_wallet::descriptor::ExtendedDescriptor;

    use crate::keychain::test_support::{MockKeychain, keychain, wallet_id};
    use crate::keychain::{KeychainError, SavedValueParseError};

    const XPUB: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";

    // the descriptor and xpub parsers quote the saved text, which must not reach the error
    #[test]
    fn unparseable_saved_values_name_the_value_without_quoting_it() {
        let id = wallet_id();
        let descriptor_key = format!("{id}::wallet_public_descriptor");
        let xpub_key = format!("{id}::wallet_xpub");
        let bare_xpub_line = format!("{XPUB}\nwpkh({XPUB}/1/*)");
        let truncated_xpub = &XPUB[..XPUB.len() - 1];
        let access = MockKeychain::with_entries(&[
            (descriptor_key.as_str(), bare_xpub_line.as_str()),
            (xpub_key.as_str(), truncated_xpub),
        ]);
        let keychain = keychain(access);

        // the fixture must make the parsers echo the saved text, or the checks below prove nothing
        let raw_descriptor_error = ExtendedDescriptor::from_str(XPUB).unwrap_err().to_string();
        assert!(raw_descriptor_error.contains(truncated_xpub), "{raw_descriptor_error}");

        let descriptor_error = keychain.get_public_descriptor(&id).unwrap_err();
        let xpub_error = keychain.get_wallet_xpub(&id).unwrap_err();

        assert_eq!(
            descriptor_error,
            KeychainError::ParseSavedValue(SavedValueParseError::ExternalDescriptor)
        );
        assert_eq!(xpub_error, KeychainError::ParseSavedValue(SavedValueParseError::Xpub));

        for error in [descriptor_error, xpub_error] {
            for text in [error.to_string(), format!("{error:?}")] {
                assert!(!text.contains(truncated_xpub), "{text}");
            }
        }
    }
}
