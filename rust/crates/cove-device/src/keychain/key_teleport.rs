use zeroize::Zeroizing;

use cove_util::{ResultExt as _, encryption::Cryptor};

use super::{
    KeychainError, SharedAccess,
    encrypted_pair::{EncryptedPair, State},
};

const SESSION_KEY: &str = "key_teleport::v1::receive_session";
const CRYPTOR_KEY: &str = "key_teleport::v1::receive_session_cryptor";

#[derive(Debug, Clone)]
pub(crate) struct KeyTeleportSessionStore(SharedAccess);

impl KeyTeleportSessionStore {
    pub(crate) fn new(access: SharedAccess) -> Self {
        Self(access)
    }

    pub(crate) fn save_receive_session(&self, plaintext: &str) -> Result<(), KeychainError> {
        self.pair().save_atomically(plaintext)
    }

    pub(crate) fn get_receive_session(&self) -> Result<Option<Zeroizing<String>>, KeychainError> {
        let (encrypted, cryptor_str) = match self.pair().state() {
            State::Empty => return Ok(None),
            State::Complete { encrypted_value, serialized_cryptor } => {
                (encrypted_value, serialized_cryptor)
            }
            State::CryptorOnly { .. } => {
                return Err(KeychainError::Decrypt(
                    "KeyTeleport receive session cryptor found but encrypted session is missing"
                        .into(),
                ));
            }
            State::ValueOnly => {
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

    pub(crate) fn delete_receive_session(&self) -> bool {
        self.pair().delete_existing_value_then_cryptor()
    }

    fn pair(&self) -> EncryptedPair {
        EncryptedPair::new(self.0.clone(), SESSION_KEY, CRYPTOR_KEY)
    }
}

#[cfg(test)]
mod tests {
    use super::{CRYPTOR_KEY, SESSION_KEY};
    use crate::keychain::{
        KeychainError,
        test_support::{FailSecondSave, MockKeychain, keychain},
    };

    #[test]
    fn receive_session_roundtrips() {
        let keychain = keychain(MockKeychain::default());
        keychain.save_key_teleport_receive_session("session").unwrap();

        assert_eq!(
            keychain.get_key_teleport_receive_session().unwrap().unwrap().as_str(),
            "session"
        );
        assert!(keychain.delete_key_teleport_receive_session());
        assert!(keychain.get_key_teleport_receive_session().unwrap().is_none());
    }

    #[test]
    fn cryptor_without_session_is_rejected() {
        let keychain = keychain(MockKeychain::with_entries(&[(CRYPTOR_KEY, "cryptor")]));

        assert!(matches!(
            keychain.get_key_teleport_receive_session(),
            Err(KeychainError::Decrypt(_))
        ));
    }

    #[test]
    fn session_without_cryptor_is_rejected() {
        let keychain = keychain(MockKeychain::with_entries(&[(SESSION_KEY, "value")]));

        assert!(matches!(
            keychain.get_key_teleport_receive_session(),
            Err(KeychainError::Decrypt(_))
        ));
    }

    #[test]
    fn save_cleans_up_on_second_save_failure() {
        let keychain = keychain(FailSecondSave::default());

        assert_eq!(keychain.save_key_teleport_receive_session("session"), Err(KeychainError::Save));
        assert!(keychain.get_key_teleport_receive_session().unwrap().is_none());
    }
}
