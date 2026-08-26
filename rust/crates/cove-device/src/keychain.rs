//! Module for interacting with the secure element

use std::sync::Arc;

use bdk_wallet::descriptor::ExtendedDescriptor;
use bip39::Mnemonic;
use bitcoin::bip32::Xpub;
use once_cell::sync::OnceCell;
use tracing::warn;
use zeroize::Zeroizing;

use cove_cspp::CsppStore;
use cove_types::WalletId;

mod encrypted_pair;
mod key_teleport;
mod local_encryption;
mod tap_signer;
#[cfg(test)]
mod test_support;
mod wallet_public;
mod wallet_secrets;

pub use wallet_secrets::{WalletSecret, WalletXprv, WalletXprvError};

use key_teleport::KeyTeleportSessionStore;
use local_encryption::LocalEncryptionKeyStore;
use tap_signer::TapSignerBackupStore;
use wallet_public::WalletPublicDataStore;
use wallet_secrets::WalletSecretStore;

type SharedAccess = Arc<dyn KeychainAccess>;

/// Errors from secure keychain operations
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum KeychainError {
    /// A value could not be saved
    #[error("unable to save")]
    Save,

    /// A value could not be deleted
    #[error("unable to delete")]
    Delete,

    /// A stored value could not be parsed
    #[error("unable to parse saved value")]
    ParseSavedValue(String),

    /// A value could not be encrypted
    #[error("unable to encrypt: {0}")]
    Encrypt(String),

    /// A value could not be decrypted
    #[error("unable to decrypt: {0}")]
    Decrypt(String),

    /// A saved wallet secret has a different type than the requested secret
    #[error("saved wallet secret is a different type")]
    WalletSecretTypeMismatch,

    /// A complete wallet secret already exists
    #[error("wallet secret already exists")]
    WalletSecretExists,
}

/// Platform access to secure key-value storage
#[uniffi::export(callback_interface)]
pub trait KeychainAccess: Send + Sync + std::fmt::Debug + 'static {
    /// Saves a key-value pair
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if the save operation fails
    fn save(&self, key: String, value: String) -> Result<(), KeychainError>;

    /// Gets the value for a key, or `None` when the key does not exist
    fn get(&self, key: String) -> Option<String>;

    /// Deletes the value for a key
    ///
    /// Returns whether the value was deleted
    fn delete(&self, key: String) -> bool;

    /// Deletes every Cove wallet-specific key without requiring wallet IDs
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if enumeration or any deletion fails
    fn delete_all_wallet_items(&self) -> Result<(), KeychainError>;
}

static REF: OnceCell<Keychain> = OnceCell::new();

/// Secure storage facade for device and wallet secrets
#[derive(Debug, Clone, uniffi::Object)]
pub struct Keychain {
    access: SharedAccess,
    local_encryption: LocalEncryptionKeyStore,
    key_teleport: KeyTeleportSessionStore,
    wallet_secrets: WalletSecretStore,
    wallet_public: WalletPublicDataStore,
    tap_signer_backups: TapSignerBackupStore,
}

#[uniffi::export]
impl Keychain {
    /// Creates a new global keychain instance
    ///
    /// # Panics
    ///
    /// Panics if setting the initial global instance fails
    #[uniffi::constructor]
    pub fn new(keychain: Box<dyn KeychainAccess>) -> Self {
        if let Some(me) = REF.get() {
            warn!("keychain is already");
            return me.clone();
        }

        let me = Self::from_access(Arc::from(keychain));
        REF.set(me).expect("failed to set keychain");

        Self::global().clone()
    }
}

impl Keychain {
    /// Returns the global keychain instance
    ///
    /// # Panics
    ///
    /// Panics if the keychain has not been initialized
    pub fn global() -> &'static Self {
        REF.get().expect("keychain is not initialized")
    }

    /// Loads the existing local database encryption key
    ///
    /// # Errors
    ///
    /// Returns an error if the stored entries are incomplete or invalid, or decryption fails
    pub fn get_local_encryption_key(&self) -> Result<Option<[u8; 32]>, KeychainError> {
        self.local_encryption.get()
    }

    /// Generates, persists, and returns a new random local database encryption key
    ///
    /// # Errors
    ///
    /// Returns an error if a key already exists, encryption fails, or a value cannot be saved
    pub fn create_local_encryption_key(&self) -> Result<[u8; 32], KeychainError> {
        self.local_encryption.create()
    }

    /// Deletes partial local encryption key entries
    pub fn purge_local_encryption_key(&self) {
        self.local_encryption.purge();
    }

    /// Saves the active KeyTeleport receive session
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails or a value cannot be saved
    pub fn save_key_teleport_receive_session(&self, plaintext: &str) -> Result<(), KeychainError> {
        self.key_teleport.save_receive_session(plaintext)
    }

    /// Gets the active KeyTeleport receive session
    ///
    /// # Errors
    ///
    /// Returns an error if the stored entries are incomplete or invalid, or decryption fails
    pub fn get_key_teleport_receive_session(
        &self,
    ) -> Result<Option<Zeroizing<String>>, KeychainError> {
        self.key_teleport.get_receive_session()
    }

    /// Deletes the active KeyTeleport receive session
    pub fn delete_key_teleport_receive_session(&self) -> bool {
        self.key_teleport.delete_receive_session()
    }

    /// Saves a wallet mnemonic seed
    ///
    /// # Errors
    ///
    /// Returns an error if a complete secret already exists, an orphaned entry cannot be deleted,
    /// encryption fails, or a value cannot be saved
    pub fn save_wallet_key(
        &self,
        id: &WalletId,
        secret_key: Mnemonic,
    ) -> Result<(), KeychainError> {
        self.save_wallet_secret(id, secret_key.into())
    }

    /// Saves a typed hot wallet secret
    ///
    /// # Errors
    ///
    /// Returns an error if a complete secret already exists, an orphaned entry cannot be deleted,
    /// encryption fails, or a value cannot be saved
    pub fn save_wallet_secret(
        &self,
        id: &WalletId,
        wallet_secret: WalletSecret,
    ) -> Result<(), KeychainError> {
        self.wallet_secrets.save(id, wallet_secret)
    }

    /// Gets a wallet mnemonic seed
    ///
    /// # Errors
    ///
    /// Returns an error if the stored secret has a different type, the stored entries are
    /// incomplete or invalid, or decryption fails
    pub fn get_wallet_key(&self, id: &WalletId) -> Result<Option<Mnemonic>, KeychainError> {
        self.get_wallet_secret(id)?
            .map(|secret| match secret {
                WalletSecret::Mnemonic(mnemonic) => Ok(mnemonic),
                WalletSecret::Xpriv(_) => Err(KeychainError::WalletSecretTypeMismatch),
            })
            .transpose()
    }

    /// Gets a typed hot wallet secret
    ///
    /// # Errors
    ///
    /// Returns an error if the stored entries are incomplete or invalid, or decryption fails
    pub fn get_wallet_secret(&self, id: &WalletId) -> Result<Option<WalletSecret>, KeychainError> {
        self.wallet_secrets.get(id)
    }

    /// Deletes a wallet secret
    pub fn delete_wallet_secret(&self, id: &WalletId) -> bool {
        self.wallet_secrets.delete(id)
    }

    /// Saves a wallet extended public key
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be saved
    pub fn save_wallet_xpub(&self, id: &WalletId, xpub: Xpub) -> Result<(), KeychainError> {
        self.wallet_public.save_xpub(id, xpub)
    }

    /// Gets a wallet extended public key
    ///
    /// # Errors
    ///
    /// Returns an error if the stored value is not a valid extended public key
    pub fn get_wallet_xpub(&self, id: &WalletId) -> Result<Option<Xpub>, KeychainError> {
        self.wallet_public.get_xpub(id)
    }

    /// Deletes a wallet extended public key
    pub fn delete_wallet_xpub(&self, id: &WalletId) -> bool {
        self.wallet_public.delete_xpub(id)
    }

    /// Saves a wallet public descriptor pair
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be saved
    #[allow(clippy::needless_pass_by_value)]
    pub fn save_public_descriptor(
        &self,
        id: &WalletId,
        external_descriptor: ExtendedDescriptor,
        internal_descriptor: ExtendedDescriptor,
    ) -> Result<(), KeychainError> {
        self.wallet_public.save_descriptors(id, external_descriptor, internal_descriptor)
    }

    /// Gets a wallet public descriptor pair
    ///
    /// # Errors
    ///
    /// Returns an error if either stored descriptor is missing or invalid
    pub fn get_public_descriptor(
        &self,
        id: &WalletId,
    ) -> Result<Option<(ExtendedDescriptor, ExtendedDescriptor)>, KeychainError> {
        self.wallet_public.get_descriptors(id)
    }

    /// Deletes a wallet public descriptor pair
    pub fn delete_public_descriptor(&self, id: &WalletId) -> bool {
        self.wallet_public.delete_descriptors(id)
    }

    /// Saves an encrypted Tap Signer backup
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails or a value cannot be saved
    pub fn save_tap_signer_backup(
        &self,
        id: &WalletId,
        backup: &[u8],
    ) -> Result<(), KeychainError> {
        self.tap_signer_backups.save(id, backup)
    }

    /// Gets and decrypts a Tap Signer backup
    ///
    /// # Errors
    ///
    /// Returns an error if the stored entries are incomplete or invalid, or decryption fails
    pub fn get_tap_signer_backup(
        &self,
        id: &WalletId,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, KeychainError> {
        self.tap_signer_backups.get(id)
    }

    /// Deletes a Tap Signer backup
    pub fn delete_tap_signer_backup(&self, id: &WalletId) -> bool {
        self.tap_signer_backups.delete(id)
    }

    /// Deletes all keychain entries for a wallet
    pub fn delete_wallet_items(&self, id: &WalletId) -> bool {
        let key_ok = self.wallet_secrets.delete(id);
        let xpub_ok = self.wallet_public.delete_xpub(id);
        let descriptor_ok = self.wallet_public.delete_descriptors(id);
        let tap_signer_ok = self.tap_signer_backups.delete(id);

        key_ok && xpub_ok && descriptor_ok && tap_signer_ok
    }

    /// Deletes every Cove wallet-specific key, including metadata-orphaned entries
    ///
    /// # Errors
    ///
    /// Returns an error if platform enumeration or any deletion fails
    pub fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
        self.access.delete_all_wallet_items()
    }

    /// Checks whether any wallet-specific keychain entry exists
    pub fn wallet_items_exist(&self, id: &WalletId) -> bool {
        self.wallet_secrets.has_any(id)
            || self.wallet_public.has_any(id)
            || matches!(self.get_tap_signer_backup(id), Ok(Some(_)) | Err(_))
    }

    fn from_access(access: SharedAccess) -> Self {
        Self {
            local_encryption: LocalEncryptionKeyStore::new(access.clone()),
            key_teleport: KeyTeleportSessionStore::new(access.clone()),
            wallet_secrets: WalletSecretStore::new(access.clone()),
            wallet_public: WalletPublicDataStore::new(access.clone()),
            tap_signer_backups: TapSignerBackupStore::new(access.clone()),
            access,
        }
    }
}

impl CsppStore for Keychain {
    type Error = KeychainError;

    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        self.access.save(key, value)
    }

    fn get(&self, key: String) -> Option<String> {
        self.access.get(key)
    }

    fn delete(&self, key: String) -> bool {
        self.access.delete(key)
    }
}
