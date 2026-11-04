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

impl EncryptedPair {
    pub(crate) fn new(
        access: SharedAccess,
        value_key: impl Into<String>,
        cryptor_key: impl Into<String>,
    ) -> Self {
        Self { access, value_key: value_key.into(), cryptor_key: cryptor_key.into() }
    }

    /// Reads both halves of the pair
    ///
    /// A read failure of either half is returned as an error instead of being classified as a
    /// missing half, so callers never repair or replace entries that may still exist
    pub(crate) fn state(&self) -> Result<State, KeychainError> {
        let encrypted_value = self.access.get(self.value_key.clone())?;
        let serialized_cryptor = self.access.get(self.cryptor_key.clone())?;

        let state = match (encrypted_value, serialized_cryptor) {
            (None, None) => State::Empty,
            (Some(encrypted_value), Some(serialized_cryptor)) => State::Complete {
                encrypted_value,
                serialized_cryptor: Zeroizing::new(serialized_cryptor),
            },
            (Some(_), None) => State::ValueOnly,
            (None, Some(serialized_cryptor)) => {
                State::CryptorOnly { serialized_cryptor: Zeroizing::new(serialized_cryptor) }
            }
        };

        Ok(state)
    }

    pub(crate) fn delete_value(&self) -> bool {
        self.access.delete(self.value_key.clone())
    }

    pub(crate) fn delete_cryptor(&self) -> bool {
        self.access.delete(self.cryptor_key.clone())
    }

    /// Deletes whichever halves exist, value first
    ///
    /// Both halves are read before anything is deleted. If either read fails nothing is
    /// deleted and false is returned, so an unreadable half is never orphaned or reported as
    /// removed
    pub(crate) fn delete_existing_value_then_cryptor(&self) -> bool {
        let Ok(state) = self.state() else {
            return false;
        };

        match state {
            State::Empty => true,
            State::ValueOnly => self.delete_value(),
            State::CryptorOnly { .. } => self.delete_cryptor(),
            State::Complete { .. } => {
                let value_ok = self.delete_value();
                let cryptor_ok = self.delete_cryptor();

                value_ok && cryptor_ok
            }
        }
    }

    /// Saves the value under a new cryptor, leaving any prior pair usable if the value save fails
    ///
    /// The prior cryptor is read first, so an unreadable pair stops the save before anything is
    /// written. A failed value save puts the prior cryptor back, so the value that is still
    /// stored stays decryptable
    pub(crate) fn save_atomically(&self, plaintext: &str) -> Result<(), KeychainError> {
        let prior_cryptor = self.access.get(self.cryptor_key.clone())?.map(Zeroizing::new);

        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(plaintext).map_err_str(KeychainError::Encrypt)?;

        // the callback needs an owned string, so the FFI boundary needs one plain copy
        let serialized = cryptor.serialize_to_string();
        self.access.save(self.cryptor_key.clone(), serialized.as_str().to_owned())?;

        if let Err(error) = self.access.save(self.value_key.clone(), encrypted) {
            // cleanup is best effort, the value save error is what the caller needs
            match prior_cryptor {
                Some(prior_cryptor) => {
                    let _ = self.access.save(self.cryptor_key.clone(), prior_cryptor.to_string());
                }
                None => {
                    self.delete_cryptor();
                }
            }

            return Err(error);
        }

        Ok(())
    }
}
