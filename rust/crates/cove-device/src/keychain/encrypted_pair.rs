use zeroize::Zeroizing;

use cove_util::{ResultExt as _, encryption::Cryptor};

use super::{KeychainError, SharedAccess};

pub(crate) struct EncryptedPair {
    access: SharedAccess,
    value_key: String,
    cryptor_key: String,
}

pub(crate) enum State {
    Empty,
    Complete { encrypted_value: String, serialized_cryptor: Zeroizing<String> },
    ValueOnly,
    CryptorOnly { serialized_cryptor: Zeroizing<String> },
}

enum FailedValueSave {
    DeleteCryptor,
    KeepCryptor,
}

impl EncryptedPair {
    pub(crate) fn new(
        access: SharedAccess,
        value_key: impl Into<String>,
        cryptor_key: impl Into<String>,
    ) -> Self {
        Self { access, value_key: value_key.into(), cryptor_key: cryptor_key.into() }
    }

    pub(crate) fn state(&self) -> State {
        let encrypted_value = self.access.get(self.value_key.clone());
        let serialized_cryptor = self.access.get(self.cryptor_key.clone());

        match (encrypted_value, serialized_cryptor) {
            (None, None) => State::Empty,
            (Some(encrypted_value), Some(serialized_cryptor)) => State::Complete {
                encrypted_value,
                serialized_cryptor: Zeroizing::new(serialized_cryptor),
            },
            (Some(_), None) => State::ValueOnly,
            (None, Some(serialized_cryptor)) => {
                State::CryptorOnly { serialized_cryptor: Zeroizing::new(serialized_cryptor) }
            }
        }
    }

    pub(crate) fn delete_value(&self) -> bool {
        self.access.delete(self.value_key.clone())
    }

    pub(crate) fn delete_cryptor(&self) -> bool {
        self.access.delete(self.cryptor_key.clone())
    }

    pub(crate) fn value_exists(&self) -> bool {
        self.access.get(self.value_key.clone()).is_some()
    }

    pub(crate) fn cryptor_exists(&self) -> bool {
        self.access.get(self.cryptor_key.clone()).is_some()
    }

    pub(crate) fn delete_existing_value_then_cryptor(&self) -> bool {
        let value_ok = if self.value_exists() { self.delete_value() } else { true };
        let cryptor_ok = if self.cryptor_exists() { self.delete_cryptor() } else { true };

        value_ok && cryptor_ok
    }

    pub(crate) fn save_atomically(&self, plaintext: &str) -> Result<(), KeychainError> {
        self.save(plaintext, FailedValueSave::DeleteCryptor)
    }

    pub(crate) fn save_preserving_cryptor_on_failure(
        &self,
        plaintext: &str,
    ) -> Result<(), KeychainError> {
        self.save(plaintext, FailedValueSave::KeepCryptor)
    }

    fn save(
        &self,
        plaintext: &str,
        failed_value_save: FailedValueSave,
    ) -> Result<(), KeychainError> {
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(plaintext).map_err_str(KeychainError::Encrypt)?;

        // the callback needs an owned string, so the FFI boundary needs one plain copy
        let serialized = cryptor.serialize_to_string();
        self.access.save(self.cryptor_key.clone(), serialized.as_str().to_owned())?;

        if let Err(error) = self.access.save(self.value_key.clone(), encrypted) {
            if matches!(failed_value_save, FailedValueSave::DeleteCryptor) {
                self.delete_cryptor();
            }

            return Err(error);
        }

        Ok(())
    }
}
