//! Module for interacting with the secure element

use std::{str::FromStr as _, sync::Arc};

use cove_util::ResultExt as _;

use bdk_wallet::bitcoin::bip32::Xpub;
use bdk_wallet::descriptor::ExtendedDescriptor;
use bip39::Mnemonic;
use once_cell::sync::OnceCell;
use tracing::warn;

use cove_cspp::CsppStore;
use cove_types::WalletId;
use cove_util::encryption::Cryptor;
use rand::RngExt as _;

const LOCAL_DB_KEY_NAME: &str = "local::v1::db_encryption_key";
const LOCAL_DB_KEY_CRYPTOR: &str = "local::v1::db_encryption_key_cryptor";

pub const CSPP_CREDENTIAL_ID_KEY: &str = "cspp::v1::credential_id";
pub const CSPP_PRF_SALT_KEY: &str = "cspp::v1::prf_salt";
pub const CSPP_NAMESPACE_ID_KEY: &str = "cspp::v1::namespace_id";

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

    fn save_entries_with_rollback(&self, entries: &[(&str, String)]) -> Result<(), KeychainError> {
        let previous_values: Vec<_> = entries
            .iter()
            .map(|(key, _)| ((*key).to_string(), self.0.get((*key).to_string())))
            .collect();

        for (key, value) in entries {
            if let Err(error) = self.0.save((*key).to_string(), value.clone()) {
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

    /// Saves CSPP passkey credentials (credential_id and PRF salt) to the keychain
    ///
    /// Hex-encodes both values before saving
    pub fn save_cspp_passkey(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<(), KeychainError> {
        self.save_entries_with_rollback(&[
            (CSPP_CREDENTIAL_ID_KEY, hex::encode(credential_id)),
            (CSPP_PRF_SALT_KEY, hex::encode(prf_salt)),
        ])
    }

    /// Loads the stored CSPP passkey credential ID from the keychain
    pub fn load_cspp_credential_id(&self) -> Option<Vec<u8>> {
        self.get(CSPP_CREDENTIAL_ID_KEY.into()).and_then(|hex_str| {
            hex::decode(hex_str)
                .inspect_err(|error| warn!("Failed to decode stored credential_id: {error}"))
                .ok()
        })
    }

    /// Saves CSPP passkey credentials and namespace ID to the keychain
    pub fn save_cspp_passkey_and_namespace(
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

    /// Clears persisted CSPP passkey credentials without touching namespace state
    pub fn clear_cspp_passkey(&self) {
        self.0.delete(CSPP_CREDENTIAL_ID_KEY.into());
        self.0.delete(CSPP_PRF_SALT_KEY.into());
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
        self.save_with_fresh_cryptor(
            wallet_mnemonic_encryption_and_nonce_key_name(id),
            wallet_mnemonic_key_name(id),
            &secret_key.to_string(),
            false,
        )
    }

    /// Retrieves a wallet's mnemonic seed from the keychain
    ///
    /// # Errors
    ///
    /// Returns a `KeychainError` if decryption or parsing fails
    pub fn get_wallet_key(&self, id: &WalletId) -> Result<Option<Mnemonic>, KeychainError> {
        let key = wallet_mnemonic_key_name(id);

        let Some(encrypted_secret_key) = self.0.get(key) else {
            return Ok(None);
        };

        let Some(encryption_key) = self.0.get(wallet_mnemonic_encryption_and_nonce_key_name(id))
        else {
            return Err(KeychainError::Decrypt(
                "encrypted mnemonic found but encryption key is missing".into(),
            ));
        };

        let cryptor = Cryptor::try_from_string(&encryption_key)
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

    #[derive(Debug)]
    struct FailNthSave {
        entries: Mutex<HashMap<String, String>>,
        save_count: Mutex<u32>,
        fail_at: u32,
    }

    impl FailNthSave {
        fn new(fail_at: u32, entries: Vec<(&str, &str)>) -> Self {
            Self {
                entries: Mutex::new(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect(),
                ),
                save_count: Mutex::new(0),
                fail_at,
            }
        }
    }

    impl KeychainAccess for FailNthSave {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            let mut save_count = self.save_count.lock().unwrap();
            *save_count += 1;
            if *save_count == self.fail_at {
                return Err(KeychainError::Save);
            }

            self.entries.lock().unwrap().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.entries.lock().unwrap().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.entries.lock().unwrap().remove(&key).is_some()
        }
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
    fn save_cspp_passkey_rolls_back_on_second_save_failure() {
        let kc = Keychain(Arc::new(Box::new(FailNthSave::new(
            2,
            vec![(CSPP_CREDENTIAL_ID_KEY, "old_credential"), (CSPP_PRF_SALT_KEY, "old_salt")],
        ))));

        let err = kc.save_cspp_passkey(&[1, 2, 3], [7; 32]).unwrap_err();
        assert!(matches!(err, KeychainError::Save));
        assert_eq!(kc.0.get(CSPP_CREDENTIAL_ID_KEY.into()).as_deref(), Some("old_credential"));
        assert_eq!(kc.0.get(CSPP_PRF_SALT_KEY.into()).as_deref(), Some("old_salt"));
    }

    #[test]
    fn save_cspp_passkey_and_namespace_rolls_back_on_third_save_failure() {
        let kc = Keychain(Arc::new(Box::new(FailNthSave::new(
            3,
            vec![
                (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
                (CSPP_PRF_SALT_KEY, "old_salt"),
                (CSPP_NAMESPACE_ID_KEY, "old_namespace"),
            ],
        ))));

        let err =
            kc.save_cspp_passkey_and_namespace(&[1, 2, 3], [9; 32], "new_namespace").unwrap_err();
        assert!(matches!(err, KeychainError::Save));
        assert_eq!(kc.0.get(CSPP_CREDENTIAL_ID_KEY.into()).as_deref(), Some("old_credential"));
        assert_eq!(kc.0.get(CSPP_PRF_SALT_KEY.into()).as_deref(), Some("old_salt"));
        assert_eq!(kc.0.get(CSPP_NAMESPACE_ID_KEY.into()).as_deref(), Some("old_namespace"));
    }

    #[test]
    fn load_cspp_credential_id_returns_none_for_invalid_hex_and_decodes_valid_hex() {
        let kc = make_keychain(MockKeychain::new());
        kc.0.save(CSPP_CREDENTIAL_ID_KEY.into(), "not-hex".into()).unwrap();

        assert!(kc.load_cspp_credential_id().is_none());

        let credential_id = vec![1, 2, 3, 254, 255];
        kc.0.save(CSPP_CREDENTIAL_ID_KEY.into(), hex::encode(&credential_id)).unwrap();

        assert_eq!(kc.load_cspp_credential_id(), Some(credential_id));
    }

    #[test]
    fn clear_cspp_passkey_removes_credential_and_salt_only() {
        let kc = make_keychain(MockKeychain::with_entries(vec![
            (CSPP_CREDENTIAL_ID_KEY, "credential"),
            (CSPP_PRF_SALT_KEY, "salt"),
            (CSPP_NAMESPACE_ID_KEY, "namespace"),
        ]));

        kc.clear_cspp_passkey();

        assert!(kc.0.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
        assert!(kc.0.get(CSPP_PRF_SALT_KEY.into()).is_none());
        assert_eq!(kc.0.get(CSPP_NAMESPACE_ID_KEY.into()).as_deref(), Some("namespace"));
    }
}
