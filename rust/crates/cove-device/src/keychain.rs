//! Module for interacting with the secure element

use std::{fmt, str::FromStr as _, sync::Arc};

use cove_util::ResultExt as _;

use bdk_wallet::descriptor::ExtendedDescriptor;
use bip39::Mnemonic;
use bitcoin::bip32::{ChildNumber, Fingerprint, Xpriv, Xpub};
use once_cell::sync::OnceCell;
use tracing::warn;
use zeroize::{Zeroize as _, Zeroizing};

use cove_cspp::CsppStore;
use cove_types::WalletId;
use cove_util::encryption::Cryptor;
use rand::RngExt as _;

const LOCAL_DB_KEY_NAME: &str = "local::v1::db_encryption_key";
const LOCAL_DB_KEY_CRYPTOR: &str = "local::v1::db_encryption_key_cryptor";
const KEY_TELEPORT_RECEIVE_SESSION: &str = "key_teleport::v1::receive_session";
const KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR: &str = "key_teleport::v1::receive_session_cryptor";
const WALLET_SECRET_TAG_PREFIX: &str = "cove::wallet_secret::v1::";
const WALLET_SECRET_MNEMONIC_TAG: &str = "mnemonic::";
const WALLET_SECRET_XPRIV_TAG: &str = "xpriv::";

/// A validated BIP32 extended private key backed by zeroizing storage
#[derive(Clone, PartialEq, Eq)]
pub struct WalletXprv(String);

/// Validation failures for a wallet extended private key
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletXprvError {
    /// The value is not a valid encoded extended private key
    #[error("invalid extended private key: {0}")]
    Invalid(String),

    /// Cove wallet secrets must represent the BIP32 root
    #[error("extended private key is not a master key")]
    NotMaster,
}

impl WalletXprv {
    /// Validates and wraps an encoded extended private key
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is invalid or contains child-key metadata
    pub fn parse(value: impl Into<String>) -> Result<Self, WalletXprvError> {
        let value = Self(value.into());
        let xprv = Xpriv::from_str(&value.0)
            .map_err(|error| WalletXprvError::Invalid(error.to_string()))?;
        validate_master_xprv(&xprv)?;

        Ok(value)
    }

    /// Exposes the encoded extended private key
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Parses the validated value into an extended private key
    pub fn to_xpriv(&self) -> Xpriv {
        Xpriv::from_str(&self.0).expect("WalletXprv must contain a validated extended private key")
    }
}

impl fmt::Debug for WalletXprv {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WalletXprv(<redacted>)")
    }
}

impl Drop for WalletXprv {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl TryFrom<Xpriv> for WalletXprv {
    type Error = WalletXprvError;

    fn try_from(value: Xpriv) -> Result<Self, Self::Error> {
        validate_master_xprv(&value)?;

        Ok(Self(value.to_string()))
    }
}

fn validate_master_xprv(value: &Xpriv) -> Result<(), WalletXprvError> {
    let is_master = value.depth == 0
        && value.parent_fingerprint == Fingerprint::default()
        && value.child_number == ChildNumber::Normal { index: 0 };
    if !is_master {
        return Err(WalletXprvError::NotMaster);
    }

    Ok(())
}

/// A hot wallet's private key material
///
/// Debug output is always redacted so diagnostics cannot expose the secret
#[derive(Clone, PartialEq, Eq)]
pub enum WalletSecret {
    /// A BIP39 mnemonic phrase
    Mnemonic(Mnemonic),

    /// A BIP32 extended private key
    Xpriv(WalletXprv),
}

impl WalletSecret {
    /// Returns the mnemonic when this secret contains one
    pub fn as_mnemonic(&self) -> Option<&Mnemonic> {
        match self {
            Self::Mnemonic(mnemonic) => Some(mnemonic),
            Self::Xpriv(_) => None,
        }
    }

    /// Returns the extended private key wrapper when this secret contains one
    pub fn as_xprv(&self) -> Option<&WalletXprv> {
        match self {
            Self::Mnemonic(_) => None,
            Self::Xpriv(xprv) => Some(xprv),
        }
    }

    fn serialize(&self) -> Zeroizing<String> {
        match self {
            // keep mnemonic values readable by Cove versions before typed wallet secrets
            Self::Mnemonic(mnemonic) => Zeroizing::new(mnemonic.to_string()),
            Self::Xpriv(xprv) => Zeroizing::new(format!(
                "{WALLET_SECRET_TAG_PREFIX}{WALLET_SECRET_XPRIV_TAG}{}",
                xprv.expose()
            )),
        }
    }

    fn parse(value: &str) -> Result<Self, KeychainError> {
        let Some(tagged_value) = value.strip_prefix(WALLET_SECRET_TAG_PREFIX) else {
            // wallet secrets saved before typed storage were untagged mnemonics
            return Mnemonic::from_str(value)
                .map(Self::Mnemonic)
                .map_err(|error| KeychainError::ParseSavedValue(error.to_string()));
        };

        if let Some(mnemonic) = tagged_value.strip_prefix(WALLET_SECRET_MNEMONIC_TAG) {
            return Mnemonic::from_str(mnemonic)
                .map(Self::Mnemonic)
                .map_err(|error| KeychainError::ParseSavedValue(error.to_string()));
        }

        if let Some(xpriv) = tagged_value.strip_prefix(WALLET_SECRET_XPRIV_TAG) {
            return WalletXprv::parse(xpriv)
                .map(Self::Xpriv)
                .map_err(|error| KeychainError::ParseSavedValue(error.to_string()));
        }

        Err(KeychainError::ParseSavedValue("unknown wallet secret type".to_string()))
    }
}

impl fmt::Debug for WalletSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mnemonic(_) => formatter.write_str("WalletSecret::Mnemonic(<redacted>)"),
            Self::Xpriv(_) => formatter.write_str("WalletSecret::Xpriv(<redacted>)"),
        }
    }
}

impl From<Mnemonic> for WalletSecret {
    fn from(value: Mnemonic) -> Self {
        Self::Mnemonic(value)
    }
}

impl TryFrom<Xpriv> for WalletSecret {
    type Error = WalletXprvError;

    fn try_from(value: Xpriv) -> Result<Self, Self::Error> {
        WalletXprv::try_from(value).map(Self::Xpriv)
    }
}

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

    #[error("saved wallet secret is a different type")]
    WalletSecretTypeMismatch,
}

#[uniffi::export(callback_interface)]
pub trait KeychainAccess: Send + Sync + std::fmt::Debug + 'static {
    /// Saves a key-value pair
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if the save operation fails
    fn save(&self, key: String, value: String) -> Result<(), KeychainError>;
    fn get(&self, key: String) -> Option<String>;
    fn delete(&self, key: String) -> bool;
}

static REF: OnceCell<Keychain> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct Keychain(Arc<Box<dyn KeychainAccess>>);

#[uniffi::export]
impl Keychain {
    /// Creates a new global keychain instance
    ///
    /// # Panics
    ///
    /// Panics if the keychain has already been initialized
    #[uniffi::constructor]
    pub fn new(keychain: Box<dyn KeychainAccess>) -> Self {
        if let Some(me) = REF.get() {
            warn!("keychain is already");
            return me.clone();
        }

        let me = Self(Arc::new(keychain));
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

    /// Load existing local DB encryption key, returns None if not found
    pub fn get_local_encryption_key(&self) -> Result<Option<[u8; 32]>, KeychainError> {
        let has_cryptor = self.0.get(LOCAL_DB_KEY_CRYPTOR.into());
        let has_key = self.0.get(LOCAL_DB_KEY_NAME.into());

        let (cryptor_str, encrypted) = match (has_cryptor, has_key) {
            (None, None) => return Ok(None),
            (Some(c), Some(k)) => (c, k),
            (Some(_), None) => {
                return Err(KeychainError::Decrypt(
                    "encryption key cryptor found but encrypted key is missing".into(),
                ));
            }
            (None, Some(_)) => {
                return Err(KeychainError::Decrypt(
                    "encrypted key found but cryptor is missing".into(),
                ));
            }
        };

        let cryptor = Cryptor::try_from_string(&cryptor_str)
            .map_err(|e| KeychainError::Decrypt(e.to_string()))?;

        let hex = cryptor
            .decrypt_from_string(&encrypted)
            .map_err(|e| KeychainError::Decrypt(e.to_string()))?;

        let bytes: [u8; 32] = hex::decode(hex)
            .map_err(|e| KeychainError::ParseSavedValue(e.to_string()))?
            .try_into()
            .map_err(|_| KeychainError::ParseSavedValue("not 32 bytes".into()))?;

        Ok(Some(bytes))
    }

    /// Generate, persist, and return a new random local DB encryption key
    ///
    /// Write-once: refuses if a key already exists in keychain
    pub fn create_local_encryption_key(&self) -> Result<[u8; 32], KeychainError> {
        if self.0.get(LOCAL_DB_KEY_NAME.into()).is_some() {
            return Err(KeychainError::Save);
        }

        let key: [u8; 32] = rand::rng().random();
        self.save_with_fresh_cryptor(
            LOCAL_DB_KEY_CRYPTOR.into(),
            LOCAL_DB_KEY_NAME.into(),
            &hex::encode(key),
            true,
        )?;

        Ok(key)
    }

    /// Delete partial local encryption key entries from keychain
    ///
    /// Used during bootstrap recovery when one entry exists but the other is missing
    pub fn purge_local_encryption_key(&self) {
        self.0.delete(LOCAL_DB_KEY_CRYPTOR.into());
        self.0.delete(LOCAL_DB_KEY_NAME.into());
    }

    pub fn save_key_teleport_receive_session(&self, plaintext: &str) -> Result<(), KeychainError> {
        self.save_with_fresh_cryptor(
            KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into(),
            KEY_TELEPORT_RECEIVE_SESSION.into(),
            plaintext,
            true,
        )
    }

    pub fn get_key_teleport_receive_session(&self) -> Result<Option<String>, KeychainError> {
        let has_cryptor = self.0.get(KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into());
        let has_session = self.0.get(KEY_TELEPORT_RECEIVE_SESSION.into());

        let (cryptor_str, encrypted) = match (has_cryptor, has_session) {
            (None, None) => return Ok(None),
            (Some(cryptor), Some(session)) => (cryptor, session),
            (Some(_), None) => {
                return Err(KeychainError::Decrypt(
                    "KeyTeleport receive session cryptor found but encrypted session is missing"
                        .into(),
                ));
            }
            (None, Some(_)) => {
                return Err(KeychainError::Decrypt(
                    "encrypted KeyTeleport receive session found but cryptor is missing".into(),
                ));
            }
        };

        let cryptor = Cryptor::try_from_string(&cryptor_str)
            .map_err_prefix("KeyTeleport receive session cryptor", KeychainError::Decrypt)?;

        cryptor
            .decrypt_from_string(&encrypted)
            .map_err_prefix("KeyTeleport receive session", KeychainError::Decrypt)
            .map(Some)
    }

    pub fn delete_key_teleport_receive_session(&self) -> bool {
        let has_session = self.0.get(KEY_TELEPORT_RECEIVE_SESSION.into()).is_some();
        let has_cryptor = self.0.get(KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into()).is_some();

        if !has_session && !has_cryptor {
            return true;
        }

        let session_ok =
            if has_session { self.0.delete(KEY_TELEPORT_RECEIVE_SESSION.into()) } else { true };
        let cryptor_ok = if has_cryptor {
            self.0.delete(KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into())
        } else {
            true
        };

        session_ok && cryptor_ok
    }

    fn save_with_fresh_cryptor(
        &self,
        cryptor_key: String,
        value_key: String,
        plaintext: &str,
        cleanup_cryptor_on_failure: bool,
    ) -> Result<(), KeychainError> {
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor
            .encrypt_to_string(plaintext)
            .map_err(|error| KeychainError::Encrypt(error.to_string()))?;

        self.0.save(cryptor_key.clone(), cryptor.serialize_to_string())?;

        if let Err(error) = self.0.save(value_key, encrypted) {
            if cleanup_cryptor_on_failure {
                self.0.delete(cryptor_key);
            }
            return Err(error);
        }

        Ok(())
    }

    /// Saves a wallet's mnemonic seed encrypted in the keychain
    ///
    /// The mnemonic is encrypted with a random [`Cryptor`] before storage. The
    /// keychain itself provides at-rest encryption, but this extra layer prevents
    /// the plaintext mnemonic from being accidentally exposed if other code
    /// enumerates keychain entries — it must be explicitly decrypted to be read
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if encryption or saving fails
    pub fn save_wallet_key(
        &self,
        id: &WalletId,
        secret_key: Mnemonic,
    ) -> Result<(), KeychainError> {
        self.save_wallet_secret(id, secret_key.into())
    }

    /// Saves a hot wallet secret encrypted in the keychain
    ///
    /// Mnemonics and extended private keys share the existing wallet mnemonic
    /// slot, so saving either variant replaces the previously stored secret
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if encryption or saving fails
    pub fn save_wallet_secret(
        &self,
        id: &WalletId,
        wallet_secret: WalletSecret,
    ) -> Result<(), KeychainError> {
        let serialized = wallet_secret.serialize();

        self.save_with_fresh_cryptor(
            wallet_mnemonic_encryption_and_nonce_key_name(id),
            wallet_mnemonic_key_name(id),
            &serialized,
            false,
        )
    }

    /// Retrieves a wallet's mnemonic seed from the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if decryption or parsing fails
    pub fn get_wallet_key(&self, id: &WalletId) -> Result<Option<Mnemonic>, KeychainError> {
        self.get_wallet_secret(id)?
            .map(|secret| match secret {
                WalletSecret::Mnemonic(mnemonic) => Ok(mnemonic),
                WalletSecret::Xpriv(_) => Err(KeychainError::WalletSecretTypeMismatch),
            })
            .transpose()
    }

    /// Retrieves a hot wallet secret from the keychain
    ///
    /// Existing untagged entries are decoded as mnemonics for backward
    /// compatibility
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if the stored entries are incomplete or the
    /// secret cannot be decrypted or parsed
    pub fn get_wallet_secret(&self, id: &WalletId) -> Result<Option<WalletSecret>, KeychainError> {
        let encrypted_secret = self.0.get(wallet_mnemonic_key_name(id));
        let encryption_key = self.0.get(wallet_mnemonic_encryption_and_nonce_key_name(id));

        let (encrypted_secret, encryption_key) = match (encrypted_secret, encryption_key) {
            (None, None) => return Ok(None),
            (Some(secret), Some(key)) => (secret, key),
            (Some(_), None) => {
                return Err(KeychainError::Decrypt(
                    "encrypted wallet secret found but encryption key is missing".into(),
                ));
            }
            (None, Some(_)) => {
                return Err(KeychainError::Decrypt(
                    "wallet secret encryption key found but encrypted secret is missing".into(),
                ));
            }
        };

        let encryption_key = Zeroizing::new(encryption_key);
        let cryptor = Cryptor::try_from_string(&encryption_key)
            .map_err_prefix("wallet secret encryption key", KeychainError::Decrypt)?;

        let wallet_secret = Zeroizing::new(
            cryptor
                .decrypt_from_string(&encrypted_secret)
                .map_err_prefix("wallet secret", KeychainError::Decrypt)?,
        );

        WalletSecret::parse(&wallet_secret).map(Some)
    }

    fn delete_wallet_key(&self, id: &WalletId) -> bool {
        let encryption_key_key = wallet_mnemonic_encryption_and_nonce_key_name(id);
        let key = wallet_mnemonic_key_name(id);

        let has_data = self.0.get(key.clone()).is_some();
        let has_key = self.0.get(encryption_key_key.clone()).is_some();

        // nothing to delete = success
        if !has_data && !has_key {
            return true;
        }

        // delete encrypted data before its encryption key (reverse of save order)
        // so a partial failure never leaves orphaned data without a decryption key
        let data_ok = if has_data { self.0.delete(key) } else { true };
        let key_ok = if has_key { self.0.delete(encryption_key_key) } else { true };
        data_ok && key_ok
    }

    /// Saves a wallet's extended public key in the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if saving fails
    pub fn save_wallet_xpub(&self, id: &WalletId, xpub: Xpub) -> Result<(), KeychainError> {
        let key = wallet_xpub_key_name(id);
        let xpub_string = xpub.to_string();

        self.0.save(key, xpub_string)?;

        Ok(())
    }

    /// Retrieves a wallet's extended public key from the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if parsing fails
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
        if self.0.get(key.clone()).is_none() {
            return true;
        }
        self.0.delete(key)
    }

    /// Saves a wallet's public descriptors in the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if saving fails
    #[allow(clippy::needless_pass_by_value)]
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

    /// Retrieves a wallet's public descriptors from the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if parsing fails
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
        if self.0.get(key.clone()).is_none() {
            return true;
        }
        self.0.delete(key)
    }

    /// Saves a Tap Signer backup encrypted in the keychain
    ///
    /// Encrypted with a random [`Cryptor`] before storage for the same reason as
    /// [`save_wallet_key`](Self::save_wallet_key) — prevents accidental plaintext
    /// exposure when keychain entries are enumerated
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if encryption or saving fails
    pub fn save_tap_signer_backup(
        &self,
        id: &WalletId,
        backup: &[u8],
    ) -> Result<(), KeychainError> {
        self.save_with_fresh_cryptor(
            wallet_tap_signer_encryption_key_and_nonce_key_name(id),
            wallet_tap_signer_backup_key_name(id),
            &hex::encode(backup),
            false,
        )
    }

    pub fn get_tap_signer_backup(&self, id: &WalletId) -> Result<Option<Vec<u8>>, KeychainError> {
        let encryption_key_key = wallet_tap_signer_encryption_key_and_nonce_key_name(id);
        let Some(encryption_secret_key) = self.0.get(encryption_key_key) else {
            // check for orphaned encrypted data without its encryption key
            let backup_key = wallet_tap_signer_backup_key_name(id);
            if self.0.get(backup_key).is_some() {
                return Err(KeychainError::Decrypt(
                    "encrypted tap signer backup found but encryption key is missing".into(),
                ));
            }
            return Ok(None);
        };

        let cryptor = Cryptor::try_from_string(&encryption_secret_key)
            .map_err_prefix("tap signer encryption key", KeychainError::Decrypt)?;

        let backup_key = wallet_tap_signer_backup_key_name(id);
        let Some(encrypted_backup) = self.0.get(backup_key) else {
            return Err(KeychainError::Decrypt(
                "tap signer encryption key found but backup data is missing".to_string(),
            ));
        };

        let backup_hex = cryptor
            .decrypt_from_string(&encrypted_backup)
            .map_err_prefix("tap signer backup", KeychainError::Decrypt)?;

        let backup = hex::decode(backup_hex)
            .map_err_prefix("tap signer backup hex", KeychainError::ParseSavedValue)?;

        Ok(Some(backup))
    }

    pub fn delete_tap_signer_backup(&self, id: &WalletId) -> bool {
        let encryption_key_key = wallet_tap_signer_encryption_key_and_nonce_key_name(id);
        let backup_key = wallet_tap_signer_backup_key_name(id);

        let has_data = self.0.get(backup_key.clone()).is_some();
        let has_key = self.0.get(encryption_key_key.clone()).is_some();

        // nothing to delete = success
        if !has_data && !has_key {
            return true;
        }

        // delete encrypted data before its encryption key (reverse of save order)
        // so a partial failure never leaves orphaned data without a decryption key
        let data_ok = self.0.delete(backup_key);
        let key_ok = self.0.delete(encryption_key_key);
        data_ok && key_ok
    }

    /// Deletes all items saved in the keychain for the given wallet id
    pub fn delete_wallet_items(&self, id: &WalletId) -> bool {
        let key_ok = self.delete_wallet_key(id);
        let xpub_ok = self.delete_wallet_xpub(id);
        let desc_ok = self.delete_public_descriptor(id);
        let tap_ok = self.delete_tap_signer_backup(id);
        key_ok && xpub_ok && desc_ok && tap_ok
    }
}

impl CsppStore for Keychain {
    type Error = KeychainError;

    fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
        self.0.save(key, value)
    }

    fn get(&self, key: String) -> Option<String> {
        self.0.get(key)
    }

    fn delete(&self, key: String) -> bool {
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

fn wallet_public_descriptor_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_public_descriptor")
}

fn wallet_tap_signer_encryption_key_and_nonce_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_tap_signer_encryption_key_and_nonce_key_name")
}

fn wallet_tap_signer_backup_key_name(id: &WalletId) -> String {
    format!("{id}::tap_signer_backup")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockKeychain(Mutex<HashMap<String, String>>);

    impl MockKeychain {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }

        fn with_entries(entries: Vec<(&str, &str)>) -> Self {
            let map: HashMap<String, String> =
                entries.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            Self(Mutex::new(map))
        }
    }

    impl KeychainAccess for MockKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
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

    fn make_keychain(mock: MockKeychain) -> Keychain {
        Keychain(Arc::new(Box::new(mock)))
    }

    fn wallet_id() -> WalletId {
        WalletId::preview_new()
    }

    fn mnemonic() -> Mnemonic {
        Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    fn xpriv() -> Xpriv {
        Xpriv::new_master(bitcoin::Network::Bitcoin, &[42; 32]).unwrap()
    }

    fn save_legacy_wallet_mnemonic(keychain: &Keychain, id: &WalletId, mnemonic: &Mnemonic) {
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(&mnemonic.to_string()).unwrap();

        keychain
            .0
            .save(wallet_mnemonic_encryption_and_nonce_key_name(id), cryptor.serialize_to_string())
            .unwrap();
        keychain.0.save(wallet_mnemonic_key_name(id), encrypted).unwrap();
    }

    #[test]
    fn wallet_secret_mnemonic_roundtrips() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        let expected = mnemonic();

        keychain.save_wallet_secret(&id, WalletSecret::Mnemonic(expected.clone())).unwrap();

        assert_eq!(
            keychain.get_wallet_secret(&id).unwrap(),
            Some(WalletSecret::Mnemonic(expected.clone()))
        );
        assert_eq!(keychain.get_wallet_key(&id).unwrap(), Some(expected));
    }

    #[test]
    fn wallet_secret_mnemonic_keeps_legacy_serialization() {
        let expected = mnemonic();
        let serialized = WalletSecret::Mnemonic(expected.clone()).serialize();

        assert_eq!(serialized.as_str(), expected.to_string());
    }

    #[test]
    fn legacy_untagged_wallet_mnemonic_remains_readable() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        let expected = mnemonic();
        save_legacy_wallet_mnemonic(&keychain, &id, &expected);

        assert_eq!(keychain.get_wallet_key(&id).unwrap(), Some(expected.clone()));
        assert_eq!(
            keychain.get_wallet_secret(&id).unwrap(),
            Some(WalletSecret::Mnemonic(expected))
        );
    }

    #[test]
    fn wallet_secret_xpriv_roundtrips() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        let expected = xpriv();

        keychain.save_wallet_secret(&id, WalletSecret::try_from(expected).unwrap()).unwrap();

        let secret = keychain.get_wallet_secret(&id).unwrap().unwrap();
        assert_eq!(secret.as_xprv().unwrap().to_xpriv(), expected);
    }

    #[test]
    fn wallet_xprv_rejects_non_master_keys() {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let child = xpriv().derive_priv(&secp, &[ChildNumber::Normal { index: 1 }]).unwrap();

        assert_eq!(WalletXprv::try_from(child), Err(WalletXprvError::NotMaster));
        assert_eq!(WalletXprv::parse(child.to_string()), Err(WalletXprvError::NotMaster));
    }

    #[test]
    fn mnemonic_getter_reports_xpriv_type_mismatch() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        keychain.save_wallet_secret(&id, WalletSecret::try_from(xpriv()).unwrap()).unwrap();

        assert_eq!(keychain.get_wallet_key(&id), Err(KeychainError::WalletSecretTypeMismatch));
    }

    #[test]
    fn saving_wallet_secret_replaces_the_existing_variant() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        keychain.save_wallet_key(&id, mnemonic()).unwrap();

        let expected = xpriv();
        keychain.save_wallet_secret(&id, WalletSecret::try_from(expected).unwrap()).unwrap();

        let secret = keychain.get_wallet_secret(&id).unwrap().unwrap();
        assert_eq!(secret.as_xprv().unwrap().to_xpriv(), expected);
        assert_eq!(keychain.get_wallet_key(&id), Err(KeychainError::WalletSecretTypeMismatch));
    }

    #[test]
    fn wallet_secret_debug_output_is_redacted() {
        let mnemonic = mnemonic();
        let mnemonic_debug = format!("{:?}", WalletSecret::Mnemonic(mnemonic.clone()));
        let xpriv = WalletXprv::try_from(xpriv()).unwrap();
        let xpriv_debug = format!("{xpriv:?}");

        assert_eq!(mnemonic_debug, "WalletSecret::Mnemonic(<redacted>)");
        assert!(!mnemonic_debug.contains(&mnemonic.to_string()));
        assert_eq!(xpriv_debug, "WalletXprv(<redacted>)");
        assert!(!xpriv_debug.contains(xpriv.expose()));
    }

    #[test]
    fn delete_wallet_items_removes_wallet_secret() {
        let keychain = make_keychain(MockKeychain::new());
        let id = wallet_id();
        keychain.save_wallet_secret(&id, WalletSecret::try_from(xpriv()).unwrap()).unwrap();

        assert!(keychain.delete_wallet_items(&id));
        assert_eq!(keychain.get_wallet_secret(&id).unwrap(), None);
    }

    #[test]
    fn get_local_encryption_key_returns_none_when_empty() {
        let kc = make_keychain(MockKeychain::new());
        assert!(kc.get_local_encryption_key().unwrap().is_none());
    }

    #[test]
    fn get_local_encryption_key_errors_on_cryptor_without_key() {
        let kc = make_keychain(MockKeychain::with_entries(vec![(
            LOCAL_DB_KEY_CRYPTOR,
            "some_cryptor_data",
        )]));
        let err = kc.get_local_encryption_key().unwrap_err();
        assert!(matches!(err, KeychainError::Decrypt(_)));
    }

    #[test]
    fn get_local_encryption_key_errors_on_key_without_cryptor() {
        let kc = make_keychain(MockKeychain::with_entries(vec![(
            LOCAL_DB_KEY_NAME,
            "some_encrypted_data",
        )]));
        let err = kc.get_local_encryption_key().unwrap_err();
        assert!(matches!(err, KeychainError::Decrypt(_)));
    }

    #[test]
    fn create_and_get_local_encryption_key_roundtrip() {
        let kc = make_keychain(MockKeychain::new());
        let created = kc.create_local_encryption_key().unwrap();
        let loaded = kc.get_local_encryption_key().unwrap().unwrap();
        assert_eq!(created, loaded);
    }

    #[test]
    fn create_local_encryption_key_cleans_up_on_second_save_failure() {
        // simulate a keychain where the second save always fails
        #[derive(Debug)]
        struct FailSecondSave(Mutex<(HashMap<String, String>, u32)>);

        impl KeychainAccess for FailSecondSave {
            fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
                let mut guard = self.0.lock().unwrap();
                guard.1 += 1;
                if guard.1 == 2 {
                    return Err(KeychainError::Save);
                }
                guard.0.insert(key, value);
                Ok(())
            }

            fn get(&self, key: String) -> Option<String> {
                self.0.lock().unwrap().0.get(&key).cloned()
            }

            fn delete(&self, key: String) -> bool {
                self.0.lock().unwrap().0.remove(&key).is_some()
            }
        }

        let mock = FailSecondSave(Mutex::new((HashMap::new(), 0)));
        let kc = Keychain(Arc::new(Box::new(mock)));

        let err = kc.create_local_encryption_key().unwrap_err();
        assert!(matches!(err, KeychainError::Save));

        // cryptor should have been cleaned up, leaving no partial state
        assert!(kc.0.get(LOCAL_DB_KEY_CRYPTOR.into()).is_none());
        assert!(kc.0.get(LOCAL_DB_KEY_NAME.into()).is_none());
    }

    #[test]
    fn purge_local_encryption_key_removes_both_entries() {
        let kc = make_keychain(MockKeychain::new());
        kc.create_local_encryption_key().unwrap();

        assert!(kc.0.get(LOCAL_DB_KEY_CRYPTOR.into()).is_some());
        assert!(kc.0.get(LOCAL_DB_KEY_NAME.into()).is_some());

        kc.purge_local_encryption_key();

        assert!(kc.0.get(LOCAL_DB_KEY_CRYPTOR.into()).is_none());
        assert!(kc.0.get(LOCAL_DB_KEY_NAME.into()).is_none());
    }

    #[test]
    fn key_teleport_receive_session_roundtrip() {
        let kc = make_keychain(MockKeychain::new());

        kc.save_key_teleport_receive_session("{\"private_key\":\"redacted\"}").unwrap();

        assert_eq!(
            kc.get_key_teleport_receive_session().unwrap().as_deref(),
            Some("{\"private_key\":\"redacted\"}")
        );
    }

    #[test]
    fn key_teleport_receive_session_errors_on_cryptor_without_session() {
        let kc = make_keychain(MockKeychain::with_entries(vec![(
            KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR,
            "some_cryptor_data",
        )]));
        let err = kc.get_key_teleport_receive_session().unwrap_err();

        assert!(matches!(err, KeychainError::Decrypt(_)));
    }

    #[test]
    fn key_teleport_receive_session_errors_on_session_without_cryptor() {
        let kc = make_keychain(MockKeychain::with_entries(vec![(
            KEY_TELEPORT_RECEIVE_SESSION,
            "some_encrypted_data",
        )]));
        let err = kc.get_key_teleport_receive_session().unwrap_err();

        assert!(matches!(err, KeychainError::Decrypt(_)));
    }

    #[test]
    fn key_teleport_receive_session_delete_removes_both_entries() {
        let kc = make_keychain(MockKeychain::new());
        kc.save_key_teleport_receive_session("session").unwrap();

        assert!(kc.delete_key_teleport_receive_session());

        assert!(kc.0.get(KEY_TELEPORT_RECEIVE_SESSION.into()).is_none());
        assert!(kc.0.get(KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into()).is_none());
    }

    #[test]
    fn key_teleport_receive_session_cleans_up_on_second_save_failure() {
        #[derive(Debug)]
        struct FailSecondSave(Mutex<(HashMap<String, String>, u32)>);

        impl KeychainAccess for FailSecondSave {
            fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
                let mut guard = self.0.lock().unwrap();
                guard.1 += 1;
                if guard.1 == 2 {
                    return Err(KeychainError::Save);
                }
                guard.0.insert(key, value);
                Ok(())
            }

            fn get(&self, key: String) -> Option<String> {
                self.0.lock().unwrap().0.get(&key).cloned()
            }

            fn delete(&self, key: String) -> bool {
                self.0.lock().unwrap().0.remove(&key).is_some()
            }
        }

        let mock = FailSecondSave(Mutex::new((HashMap::new(), 0)));
        let kc = Keychain(Arc::new(Box::new(mock)));

        let err = kc.save_key_teleport_receive_session("session").unwrap_err();
        assert!(matches!(err, KeychainError::Save));

        assert!(kc.0.get(KEY_TELEPORT_RECEIVE_SESSION_CRYPTOR.into()).is_none());
        assert!(kc.0.get(KEY_TELEPORT_RECEIVE_SESSION.into()).is_none());
    }
}
