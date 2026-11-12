use zeroize::Zeroizing;

use cove_types::WalletId;
use cove_util::{ResultExt as _, encryption::Cryptor};

use super::{
    KeychainError, SavedValueParseError, SharedAccess,
    encrypted_pair::{EncryptedPair, State},
};

#[derive(Debug, Clone)]
pub(crate) struct TapSignerBackupStore(SharedAccess);

impl TapSignerBackupStore {
    pub(crate) fn new(access: SharedAccess) -> Self {
        Self(access)
    }

    pub(crate) fn save(&self, id: &WalletId, backup: &[u8]) -> Result<(), KeychainError> {
        let encoded = Zeroizing::new(hex::encode(backup));
        self.pair(id).save_atomically(&encoded)
    }

    pub(crate) fn get(&self, id: &WalletId) -> Result<Option<Zeroizing<Vec<u8>>>, KeychainError> {
        let (encrypted_backup, encryption_secret_key) = match self.pair(id).state()? {
            State::Empty => return Ok(None),
            State::Complete { encrypted_value, serialized_cryptor } => {
                (encrypted_value, serialized_cryptor)
            }
            State::ValueOnly => {
                return Err(KeychainError::Decrypt(
                    "encrypted tap signer backup found but encryption key is missing".into(),
                ));
            }
            State::CryptorOnly { serialized_cryptor } => {
                Cryptor::try_from_string(&serialized_cryptor)
                    .map_err_prefix("tap signer encryption key", KeychainError::Decrypt)?;

                return Err(KeychainError::Decrypt(
                    "tap signer encryption key found but backup data is missing".to_string(),
                ));
            }
        };

        let cryptor = Cryptor::try_from_string(&encryption_secret_key)
            .map_err_prefix("tap signer encryption key", KeychainError::Decrypt)?;
        let backup_hex = cryptor
            .decrypt_from_string(&encrypted_backup)
            .map_err_prefix("tap signer backup", KeychainError::Decrypt)?;
        let backup =
            Zeroizing::new(hex::decode(backup_hex.as_str()).map_err(|_| {
                KeychainError::ParseSavedValue(SavedValueParseError::TapSignerBackup)
            })?);

        Ok(Some(backup))
    }

    pub(crate) fn has_any(&self, id: &WalletId) -> Result<bool, KeychainError> {
        Ok(!matches!(self.pair(id).state()?, State::Empty))
    }

    pub(crate) fn delete(&self, id: &WalletId) -> bool {
        // delete data first so a partial failure does not leave data without its key
        self.pair(id).delete_existing_value_then_cryptor()
    }

    fn pair(&self, id: &WalletId) -> EncryptedPair {
        EncryptedPair::new(
            self.0.clone(),
            format!("{id}::tap_signer_backup"),
            format!("{id}::wallet_tap_signer_encryption_key_and_nonce_key_name"),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::keychain::test_support::{MockKeychain, keychain, wallet_id};

    #[test]
    fn backup_roundtrips_and_deletes() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        let expected = [1, 2, 3, 4];
        keychain.save_tap_signer_backup(&id, &expected).unwrap();

        assert_eq!(keychain.get_tap_signer_backup(&id).unwrap().unwrap().as_slice(), expected);
        assert!(keychain.delete_tap_signer_backup(&id));
        assert!(keychain.get_tap_signer_backup(&id).unwrap().is_none());
    }

    #[test]
    fn failed_overwrite_keeps_the_prior_backup_readable() {
        let access = MockKeychain::default();
        let keychain = keychain(access.clone());
        let id = wallet_id();
        keychain.save_tap_signer_backup(&id, &[1, 2, 3]).unwrap();
        access.fail_save_for(format!("{id}::tap_signer_backup"));

        assert!(keychain.save_tap_signer_backup(&id, &[4, 5, 6]).is_err());
        assert_eq!(keychain.get_tap_signer_backup(&id).unwrap().unwrap().as_slice(), [1, 2, 3]);
    }
}
