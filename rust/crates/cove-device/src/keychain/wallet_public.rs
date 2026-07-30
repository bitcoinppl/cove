use std::str::FromStr as _;

use bdk_wallet::descriptor::ExtendedDescriptor;
use bitcoin::bip32::Xpub;

use cove_types::WalletId;

use super::{KeychainError, SharedAccess};

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
        let Some(value) = self.0.get(xpub_key_name(id)) else {
            return Ok(None);
        };

        let xpub = Xpub::from_str(&value).map_err(|error| {
            KeychainError::ParseSavedValue(format!(
                "Unable to parse saved xpub, something went wrong \
                 with saving, this should not happen {error}"
            ))
        })?;

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
        let Some(value) = self.0.get(descriptor_key_name(id)) else {
            return Ok(None);
        };

        let mut lines = value.lines();
        let external = lines.next().ok_or_else(|| {
            KeychainError::ParseSavedValue("missing external descriptor".to_string())
        })?;

        let internal = lines.next().ok_or_else(|| {
            KeychainError::ParseSavedValue("missing internal descriptor".to_string())
        })?;

        let external = ExtendedDescriptor::from_str(external).map_err(|error| {
            KeychainError::ParseSavedValue(format!("invalid external descriptor: {error}"))
        })?;

        let internal = ExtendedDescriptor::from_str(internal).map_err(|error| {
            KeychainError::ParseSavedValue(format!("invalid internal descriptor: {error}"))
        })?;

        Ok(Some((external, internal)))
    }

    pub(crate) fn delete_descriptors(&self, id: &WalletId) -> bool {
        delete_if_present(&self.0, descriptor_key_name(id))
    }
}

fn delete_if_present(access: &SharedAccess, key: String) -> bool {
    if access.get(key.clone()).is_none() {
        return true;
    }

    access.delete(key)
}

fn xpub_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_xpub")
}

fn descriptor_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_public_descriptor")
}
