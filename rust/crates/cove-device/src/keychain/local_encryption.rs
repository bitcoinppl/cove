use rand::RngExt as _;
use zeroize::Zeroizing;

use cove_util::{ResultExt as _, encryption::Cryptor};

use super::{
    KeychainError, SharedAccess,
    encrypted_pair::{EncryptedPair, State},
};

const KEY_NAME: &str = "local::v1::db_encryption_key";
const CRYPTOR_NAME: &str = "local::v1::db_encryption_key_cryptor";

#[derive(Debug, Clone)]
pub(crate) struct LocalEncryptionKeyStore(SharedAccess);

impl LocalEncryptionKeyStore {
    pub(crate) fn new(access: SharedAccess) -> Self {
        Self(access)
    }

    pub(crate) fn get(&self) -> Result<Option<[u8; 32]>, KeychainError> {
        let (encrypted, cryptor_str) = match self.pair().state() {
            State::Empty => return Ok(None),
            State::Complete { encrypted_value, serialized_cryptor } => {
                (encrypted_value, serialized_cryptor)
            }
            State::CryptorOnly { .. } => {
                return Err(KeychainError::Decrypt(
                    "encryption key cryptor found but encrypted key is missing".into(),
                ));
            }
            State::ValueOnly => {
                return Err(KeychainError::Decrypt(
                    "encrypted key found but cryptor is missing".into(),
                ));
            }
        };

        let cryptor = Cryptor::try_from_string(&cryptor_str).map_err_str(KeychainError::Decrypt)?;
        let hex = cryptor.decrypt_from_string(&encrypted).map_err_str(KeychainError::Decrypt)?;
        let decoded =
            Zeroizing::new(hex::decode(hex.as_str()).map_err_str(KeychainError::ParseSavedValue)?);
        let bytes = decoded
            .as_slice()
            .try_into()
            .map_err(|_| KeychainError::ParseSavedValue("not 32 bytes".into()))?;

        Ok(Some(bytes))
    }

    pub(crate) fn create(&self) -> Result<[u8; 32], KeychainError> {
        let pair = self.pair();
        if pair.value_exists() {
            return Err(KeychainError::Save);
        }

        let key: [u8; 32] = rand::rng().random();
        let encoded = Zeroizing::new(hex::encode(key));
        pair.save_atomically(&encoded)?;

        Ok(key)
    }

    pub(crate) fn purge(&self) {
        let pair = self.pair();
        pair.delete_cryptor();
        pair.delete_value();
    }

    fn pair(&self) -> EncryptedPair {
        EncryptedPair::new(self.0.clone(), KEY_NAME, CRYPTOR_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::{CRYPTOR_NAME, KEY_NAME};
    use crate::keychain::{
        KeychainAccess as _, KeychainError,
        test_support::{FailSecondSave, MockKeychain, keychain},
    };

    #[test]
    fn get_returns_none_when_empty() {
        assert!(keychain(MockKeychain::default()).get_local_encryption_key().unwrap().is_none());
    }

    #[test]
    fn create_and_get_roundtrip() {
        let keychain = keychain(MockKeychain::default());
        let created = keychain.create_local_encryption_key().unwrap();

        assert_eq!(keychain.get_local_encryption_key().unwrap(), Some(created));
    }

    #[test]
    fn cryptor_without_key_is_rejected() {
        let keychain = keychain(MockKeychain::with_entries(&[(CRYPTOR_NAME, "cryptor")]));

        assert!(matches!(keychain.get_local_encryption_key(), Err(KeychainError::Decrypt(_))));
    }

    #[test]
    fn key_without_cryptor_is_rejected() {
        let keychain = keychain(MockKeychain::with_entries(&[(KEY_NAME, "value")]));

        assert!(matches!(keychain.get_local_encryption_key(), Err(KeychainError::Decrypt(_))));
    }

    #[test]
    fn purge_removes_both_entries() {
        let access = std::sync::Arc::new(MockKeychain::default());
        let keychain = keychain(SharedMock(access.clone()));
        keychain.create_local_encryption_key().unwrap();
        keychain.purge_local_encryption_key();

        assert!(access.get(KEY_NAME.into()).is_none());
        assert!(access.get(CRYPTOR_NAME.into()).is_none());
    }

    #[test]
    fn create_cleans_up_on_second_save_failure() {
        let keychain = keychain(FailSecondSave::default());

        assert_eq!(keychain.create_local_encryption_key(), Err(KeychainError::Save));
        assert!(keychain.get_local_encryption_key().unwrap().is_none());
    }

    #[derive(Debug)]
    struct SharedMock(std::sync::Arc<MockKeychain>);

    impl crate::keychain::KeychainAccess for SharedMock {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.save(key, value)
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.get(key)
        }

        fn delete(&self, key: String) -> bool {
            self.0.delete(key)
        }

        fn delete_all_wallet_items(&self) -> Result<(), KeychainError> {
            self.0.delete_all_wallet_items()
        }
    }
}
