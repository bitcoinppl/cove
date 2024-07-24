//! Module for interacting with the secure element

use std::{str::FromStr as _, sync::Arc};

use bdk_wallet::bitcoin::bip32::Xpub;
use bip39::Mnemonic;
use once_cell::sync::OnceCell;
use tracing::warn;

use crate::encryption::Cryptor;
use crate::wallet::metadata::WalletId;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum KeychainError {
    #[error("unable to save")]
    UnableToSave,

    #[error("unable to delete")]
    UnableToDelete,

    #[error("unable to parse saved value")]
    UnableToParseSavedValue(String),

    #[error("unable to encrypt: {0}")]
    UnableToEncrypt(String),

    #[error("unable to decrypt: {0}")]
    UnableToDecrypt(String),
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
            .map_err(|error| KeychainError::UnableToEncrypt(error.to_string()))?;

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

        let Some(encryption_key) = self
            .0
            .get(wallet_mnemonic_encryption_and_nonce_key_name(id))
        else {
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(encryption_key)
            .map_err(|error| KeychainError::UnableToDecrypt(error.to_string()))?;

        let secret_key = cryptor
            .decrypt_from_string(&encrypted_secret_key)
            .map_err(|error| KeychainError::UnableToDecrypt(error.to_string()))?;

        let mnemonic = Mnemonic::from_str(&secret_key)
            .map_err(|error| KeychainError::UnableToParseSavedValue(error.to_string()))?;

        Ok(Some(mnemonic))
    }

    pub fn delete_wallet_key(&self, id: &WalletId) -> bool {
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

            KeychainError::UnableToParseSavedValue(error)
        })?;

        Ok(Some(xpub))
    }

    pub fn delete_wallet_xpub(&self, id: &WalletId) -> bool {
        let key = wallet_xpub_key_name(id);
        self.0.delete(key)
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
