//! Module for interacting with the secure element

use std::{str::FromStr as _, sync::Arc};

use bdk_wallet::bitcoin::bip32::Xpub;
use bdk_wallet::descriptor::ExtendedDescriptor;
use bip39::Mnemonic;
use once_cell::sync::OnceCell;
use tracing::warn;

use cove_types::WalletId;
use cove_util::encryption::Cryptor;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum KeychainError {
    #[error("unable to save")]
    Save,

    #[error("unable to delete")]
    Delete,

    #[error("unable to parse saved value")]
    ParseSavedValue(String),

    #[error("unable to encrypt: {0}")]
    Encrypt(String),

    #[error("unable to decrypt: {0}")]
    Decrypt(String),
}

#[uniffi::export(callback_interface)]
pub trait KeychainAccess: Send + Sync + std::fmt::Debug + 'static {
    fn save(&self, key: String, value: String) -> Result<(), KeychainError>;
    fn get(&self, key: String) -> Option<String>;
    fn delete(&self, key: String) -> bool;
}

static REF: OnceCell<Keychain> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct Keychain(Arc<Box<dyn KeychainAccess>>);

#[uniffi::export]
impl Keychain {
    #[uniffi::constructor]
    pub fn new(keychain: Box<dyn KeychainAccess>) -> Self {
        if let Some(me) = REF.get() {
            warn!("keychain is already");
            return me.clone();
        }

        let me = Self(Arc::new(keychain));
        REF.set(me).expect("failed to set keychain");

        Keychain::global().clone()
    }
}

impl Keychain {
    pub fn global() -> &'static Self {
        REF.get().expect("keychain is not initialized")
    }

    pub fn save_wallet_key(
        &self,
        id: &WalletId,
        secret_key: Mnemonic,
    ) -> Result<(), KeychainError> {
        let encryption_key_key = wallet_mnemonic_encryption_and_nonce_key_name(id);
        let cryptor = Cryptor::new();

        let key = wallet_mnemonic_key_name(id);
        let encrypted_secret_key = cryptor
            .encrypt_to_string(secret_key.to_string())
            .map_err(|error| KeychainError::Encrypt(error.to_string()))?;

        let encryption_key = cryptor.serialize_to_string();

        self.0.save(encryption_key_key, encryption_key)?;
        self.0.save(key, encrypted_secret_key)?;

        Ok(())
    }

    pub fn get_wallet_key(&self, id: &WalletId) -> Result<Option<Mnemonic>, KeychainError> {
        let key = wallet_mnemonic_key_name(id);

        let Some(encrypted_secret_key) = self.0.get(key) else {
            return Ok(None);
        };

        let Some(encryption_key) = self.0.get(wallet_mnemonic_encryption_and_nonce_key_name(id))
        else {
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(encryption_key)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let secret_key = cryptor
            .decrypt_from_string(&encrypted_secret_key)
            .map_err(|error| KeychainError::Decrypt(error.to_string()))?;

        let mnemonic = Mnemonic::from_str(&secret_key)
            .map_err(|error| KeychainError::ParseSavedValue(error.to_string()))?;

        Ok(Some(mnemonic))
    }

    fn delete_wallet_key(&self, id: &WalletId) -> bool {
        let encryption_key_key = wallet_mnemonic_encryption_and_nonce_key_name(id);
        let key = wallet_mnemonic_key_name(id);

        self.0.delete(encryption_key_key);
        self.0.delete(key)
    }

    pub fn save_wallet_xpub(&self, id: &WalletId, xpub: Xpub) -> Result<(), KeychainError> {
        let key = wallet_xpub_key_name(id);
        let xpub_string = xpub.to_string();

        self.0.save(key, xpub_string)?;

        Ok(())
    }

    pub fn get_wallet_xpub(&self, id: &WalletId) -> Result<Option<Xpub>, KeychainError> {
        let key = wallet_xpub_key_name(id);
        let Some(xpub_string) = self.0.get(key) else {
            return Ok(None);
        };

        let xpub = Xpub::from_str(&xpub_string).map_err(|error| {
            let error = format!(
                "Unable to parse saved xpub, something went wrong \
                    with saving, this should not happen {error}"
            );

            KeychainError::ParseSavedValue(error)
        })?;

        Ok(Some(xpub))
    }

    fn delete_wallet_xpub(&self, id: &WalletId) -> bool {
        let key = wallet_xpub_key_name(id);
        self.0.delete(key)
    }

    pub fn save_public_descriptor(
        &self,
        id: &WalletId,
        external_descriptor: ExtendedDescriptor,
        internal_descriptor: ExtendedDescriptor,
    ) -> Result<(), KeychainError> {
        let key = wallet_public_descriptor_key_name(id);
        let value = format!("{external_descriptor}\n{internal_descriptor}");

        self.0.save(key, value)
    }

    pub fn get_public_descriptor(
        &self,
        id: &WalletId,
    ) -> Result<Option<(ExtendedDescriptor, ExtendedDescriptor)>, KeychainError> {
        let key = wallet_public_descriptor_key_name(id);
        let Some(value) = self.0.get(key) else {
            return Ok(None);
        };

        let mut lines = value.lines();
        let external = lines.next().ok_or_else(|| {
            KeychainError::ParseSavedValue("missing external descriptor".to_string())
        })?;
        let internal = lines.next().ok_or_else(|| {
            KeychainError::ParseSavedValue("missing internal descriptor".to_string())
        })?;

        let external = ExtendedDescriptor::from_str(external).map_err(|e| {
            KeychainError::ParseSavedValue(format!("invalid external descriptor: {e}"))
        })?;
        let internal = ExtendedDescriptor::from_str(internal).map_err(|e| {
            KeychainError::ParseSavedValue(format!("invalid internal descriptor: {e}"))
        })?;

        Ok(Some((external, internal)))
    }

    fn delete_public_descriptor(&self, id: &WalletId) -> bool {
        let key = wallet_public_descriptor_key_name(id);
        self.0.delete(key)
    }

    pub fn save_tap_signer_backup(
        &self,
        id: &WalletId,
        backup: &[u8],
    ) -> Result<(), KeychainError> {
        // create the backup hex
        let backup_key = wallet_tap_signer_backup_key_name(id);
        let backup_hex = hex::encode(backup);

        let encryption_key_key = wallet_tap_signer_encryption_key_and_nonce_key_name(id);
        let cryptor = Cryptor::new();

        // encrypt the backup
        let encrypted_backup = cryptor
            .encrypt_to_string(backup_hex)
            .map_err(|error| KeychainError::Encrypt(error.to_string()))?;

        // get the encryption key as a string
        let encryption_key = cryptor.serialize_to_string();

        // save the backup and encryption key
        self.0.save(backup_key, encrypted_backup)?;
        self.0.save(encryption_key_key, encryption_key)?;

        Ok(())
    }

    pub fn get_tap_signer_backup(&self, id: &WalletId) -> Option<Vec<u8>> {
        let cryptor = {
            let encryption_key_key = wallet_tap_signer_encryption_key_and_nonce_key_name(id);
            let encryption_secret_key = self.0.get(encryption_key_key)?;
            Cryptor::try_from_string(encryption_secret_key).ok()?
        };

        let backup_key = wallet_tap_signer_backup_key_name(id);
        let encrypted_backup = self.0.get(backup_key)?;

        let backup_hex = cryptor.decrypt_from_string(&encrypted_backup).ok()?;
        let backup = hex::decode(backup_hex).ok()?;

        Some(backup)
    }

    pub fn delete_tap_signer_backup(&self, id: &WalletId) -> bool {
        let encryption_key_key = wallet_tap_signer_encryption_key_and_nonce_key_name(id);
        let backup_key = wallet_tap_signer_backup_key_name(id);

        self.0.delete(encryption_key_key);
        self.0.delete(backup_key)
    }

    // MARK: Delete
    // deletes all items saved in the keychain for the given wallet id
    pub fn delete_wallet_items(&self, id: &WalletId) -> bool {
        self.delete_wallet_key(id)
            && self.delete_wallet_xpub(id)
            && self.delete_public_descriptor(id)
            && self.delete_tap_signer_backup(id)
    }
}

fn wallet_mnemonic_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_mnemonic")
}

fn wallet_xpub_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_xpub")
}

fn wallet_mnemonic_encryption_and_nonce_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_mnemonic_encryption_key_and_nonce")
}

fn wallet_public_descriptor_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_public_descriptor")
}

fn wallet_tap_signer_encryption_key_and_nonce_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_tap_signer_encryption_key_and_nonce_key_name")
}

fn wallet_tap_signer_backup_key_name(id: &WalletId) -> String {
    format!("{id}::tap_signer_backup")
}
