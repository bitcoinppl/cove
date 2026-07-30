use std::{fmt, str::FromStr as _, sync::Arc};

use bip39::Mnemonic;
use bitcoin::bip32::{ChildNumber, Fingerprint, Xpriv};
use parking_lot::Mutex;
use zeroize::{Zeroize as _, Zeroizing};

use cove_types::WalletId;
use cove_util::{ResultExt as _, encryption::Cryptor};

use super::{
    KeychainError, SharedAccess,
    encrypted_pair::{EncryptedPair, State},
};

const TAG_PREFIX: &str = "cove::wallet_secret::v1::";
const XPRIV_TAG: &str = "xpriv::";

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
        let xprv = Xpriv::from_str(&value.0).map_err_str(WalletXprvError::Invalid)?;
        validate_master_xprv(&xprv)?;

        Ok(value)
    }

    /// Exposes the encoded extended private key
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Parses the validated value into an extended private key
    ///
    /// # Panics
    ///
    /// Panics if the value no longer contains the extended private key validated at construction
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
            // keep mnemonic values readable by versions before typed wallet secrets
            Self::Mnemonic(mnemonic) => Zeroizing::new(mnemonic.to_string()),
            Self::Xpriv(xprv) => {
                Zeroizing::new(format!("{TAG_PREFIX}{XPRIV_TAG}{}", xprv.expose()))
            }
        }
    }

    fn parse(value: &str) -> Result<Self, KeychainError> {
        let Some(tagged_value) = value.strip_prefix(TAG_PREFIX) else {
            // wallet secrets saved before typed storage were untagged mnemonics
            return Mnemonic::from_str(value)
                .map(Self::Mnemonic)
                .map_err_str(KeychainError::ParseSavedValue);
        };

        if let Some(xpriv) = tagged_value.strip_prefix(XPRIV_TAG) {
            return WalletXprv::parse(xpriv)
                .map(Self::Xpriv)
                .map_err_str(KeychainError::ParseSavedValue);
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

#[derive(Debug, Clone)]
pub(crate) struct WalletSecretStore(SharedAccess, Arc<Mutex<()>>);

impl WalletSecretStore {
    pub(crate) fn new(access: SharedAccess) -> Self {
        Self(access, Arc::new(Mutex::new(())))
    }

    pub(crate) fn save(&self, id: &WalletId, secret: WalletSecret) -> Result<(), KeychainError> {
        let _guard = self.1.lock();
        let pair = self.pair(id);

        match pair.state() {
            State::Complete { .. } => return Err(KeychainError::WalletSecretExists),
            State::ValueOnly => {
                if !pair.delete_value() {
                    return Err(KeychainError::Delete);
                }
            }
            State::CryptorOnly { .. } => {
                if !pair.delete_cryptor() {
                    return Err(KeychainError::Delete);
                }
            }
            State::Empty => {}
        }

        pair.save_atomically(&secret.serialize())
    }

    pub(crate) fn get(&self, id: &WalletId) -> Result<Option<WalletSecret>, KeychainError> {
        // serialize against writers so a read never observes a half-written pair
        let _guard = self.1.lock();
        let (encrypted_secret, encryption_key) = match self.pair(id).state() {
            State::Empty => return Ok(None),
            State::Complete { encrypted_value, serialized_cryptor } => {
                (encrypted_value, serialized_cryptor)
            }
            State::ValueOnly => {
                return Err(KeychainError::Decrypt(
                    "encrypted wallet secret found but encryption key is missing".into(),
                ));
            }
            State::CryptorOnly { .. } => {
                return Err(KeychainError::Decrypt(
                    "wallet secret encryption key found but encrypted secret is missing".into(),
                ));
            }
        };

        let cryptor = Cryptor::try_from_string(&encryption_key)
            .map_err_prefix("wallet secret encryption key", KeychainError::Decrypt)?;
        let wallet_secret = cryptor
            .decrypt_from_string(&encrypted_secret)
            .map_err_prefix("wallet secret", KeychainError::Decrypt)?;

        WalletSecret::parse(&wallet_secret).map(Some)
    }

    pub(crate) fn delete(&self, id: &WalletId) -> bool {
        let _guard = self.1.lock();
        self.pair(id).delete_existing_value_then_cryptor()
    }

    fn pair(&self, id: &WalletId) -> EncryptedPair {
        EncryptedPair::new(self.0.clone(), mnemonic_key_name(id), mnemonic_cryptor_key_name(id))
    }
}

pub(crate) fn mnemonic_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_mnemonic")
}

pub(crate) fn mnemonic_cryptor_key_name(id: &WalletId) -> String {
    format!("{id}::wallet_mnemonic_encryption_key_and_nonce")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bip39::Mnemonic;
    use bitcoin::bip32::{ChildNumber, Xpriv};

    use super::{
        TAG_PREFIX, WalletSecret, WalletXprv, XPRIV_TAG, mnemonic_cryptor_key_name,
        mnemonic_key_name,
    };
    use crate::keychain::{
        KeychainAccess as _, KeychainError,
        test_support::{FailingKeychain, MockKeychain, keychain, wallet_id},
    };
    use cove_util::encryption::Cryptor;

    fn mnemonic() -> Mnemonic {
        Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    fn xpriv() -> Xpriv {
        Xpriv::new_master(bitcoin::Network::Bitcoin, &[42; 32]).unwrap()
    }

    #[test]
    fn mnemonic_roundtrips() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        let expected_mnemonic = mnemonic();

        keychain
            .save_wallet_secret(&id, WalletSecret::Mnemonic(expected_mnemonic.clone()))
            .unwrap();

        assert_eq!(keychain.get_wallet_key(&id).unwrap(), Some(expected_mnemonic));
    }

    #[test]
    fn xpriv_roundtrips() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        let expected_xpriv = xpriv();
        keychain.save_wallet_secret(&id, WalletSecret::try_from(expected_xpriv).unwrap()).unwrap();

        assert_eq!(
            keychain.get_wallet_secret(&id).unwrap().unwrap().as_xprv().unwrap().to_xpriv(),
            expected_xpriv
        );
    }

    #[test]
    fn xprv_rejects_non_master_keys() {
        let child = xpriv()
            .derive_priv(&bitcoin::secp256k1::Secp256k1::new(), &[ChildNumber::Normal { index: 1 }])
            .unwrap();

        assert!(WalletXprv::try_from(child).is_err());
    }

    #[test]
    fn complete_secret_is_write_once() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        keychain.save_wallet_key(&id, mnemonic()).unwrap();

        assert_eq!(
            keychain.save_wallet_secret(&id, WalletSecret::try_from(xpriv()).unwrap()),
            Err(KeychainError::WalletSecretExists)
        );
    }

    #[test]
    fn failed_secret_write_leaves_no_partial_state() {
        let id = wallet_id();
        let value_key = mnemonic_key_name(&id);
        let cryptor_key = mnemonic_cryptor_key_name(&id);

        for failing_key in [value_key.clone(), cryptor_key.clone()] {
            let access = FailingKeychain::default();
            access.fail_save_for(failing_key);
            let keychain = keychain(access.clone());

            assert_eq!(keychain.save_wallet_key(&id, mnemonic()), Err(KeychainError::Save));
            assert!(access.entry(&value_key).is_none());
            assert!(access.entry(&cryptor_key).is_none());
        }
    }

    #[test]
    fn mnemonic_serialization_is_untagged() {
        let expected = mnemonic();

        assert_eq!(
            WalletSecret::Mnemonic(expected.clone()).serialize().as_str(),
            expected.to_string()
        );
    }

    #[test]
    fn untagged_mnemonic_remains_readable() {
        let access = FailingKeychain::default();
        let id = wallet_id();
        let expected = mnemonic();
        let mut cryptor = Cryptor::new();
        let encrypted = cryptor.encrypt_to_string(&expected.to_string()).unwrap();
        access
            .save(mnemonic_cryptor_key_name(&id), cryptor.serialize_to_string().as_str().to_owned())
            .unwrap();
        access.save(mnemonic_key_name(&id), encrypted).unwrap();
        let keychain = keychain(access);

        assert_eq!(keychain.get_wallet_key(&id).unwrap(), Some(expected));
    }

    #[test]
    fn mnemonic_getter_reports_xpriv_type_mismatch() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        keychain.save_wallet_secret(&id, WalletSecret::try_from(xpriv()).unwrap()).unwrap();

        assert_eq!(keychain.get_wallet_key(&id), Err(KeychainError::WalletSecretTypeMismatch));
    }

    #[test]
    fn concurrent_saves_allow_exactly_one_create() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        let expected = WalletSecret::Mnemonic(mnemonic());
        let first_keychain = keychain.clone();
        let first_id = id.clone();
        let first_secret = expected.clone();
        let second_keychain = keychain.clone();
        let second_id = id.clone();
        let second_secret = expected.clone();
        let first =
            std::thread::spawn(move || first_keychain.save_wallet_secret(&first_id, first_secret));
        let second = std::thread::spawn(move || {
            second_keychain.save_wallet_secret(&second_id, second_secret)
        });
        let results = [first.join().unwrap(), second.join().unwrap()];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(KeychainError::WalletSecretExists))
                .count(),
            1
        );
    }

    #[test]
    fn save_repairs_a_lone_orphaned_entry() {
        let id = wallet_id();

        for orphaned_key in [mnemonic_key_name(&id), mnemonic_cryptor_key_name(&id)] {
            let keychain = keychain(MockKeychain::with_entries(&[(&orphaned_key, "orphan")]));
            keychain.save_wallet_key(&id, mnemonic()).unwrap();

            assert!(keychain.get_wallet_key(&id).unwrap().is_some());
        }
    }

    #[test]
    fn save_rejects_an_unreadable_committed_pair() {
        let id = wallet_id();
        let value_key = mnemonic_key_name(&id);
        let cryptor_key = mnemonic_cryptor_key_name(&id);
        let keychain = keychain(MockKeychain::with_entries(&[
            (&value_key, "unreadable secret"),
            (&cryptor_key, "unreadable cryptor"),
        ]));

        assert_eq!(
            keychain.save_wallet_key(&id, mnemonic()),
            Err(KeychainError::WalletSecretExists)
        );
    }

    #[test]
    fn uses_exact_wallet_mnemonic_key_names() {
        let access = FailingKeychain::default();
        let id = wallet_id();
        let keychain = keychain(access.clone());
        keychain.save_wallet_key(&id, mnemonic()).unwrap();
        let keys = access.keys();

        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&format!("{id}::wallet_mnemonic")));
        assert!(keys.contains(&format!("{id}::wallet_mnemonic_encryption_key_and_nonce")));
    }

    #[test]
    fn mnemonic_is_readable_by_legacy_logic() {
        let access = FailingKeychain::default();
        let id = wallet_id();
        let expected = mnemonic();
        let keychain = keychain(access.clone());
        keychain.save_wallet_key(&id, expected.clone()).unwrap();
        let encrypted = access.entry(&mnemonic_key_name(&id)).unwrap();
        let cryptor = access.entry(&mnemonic_cryptor_key_name(&id)).unwrap();
        let decrypted =
            Cryptor::try_from_string(&cryptor).unwrap().decrypt_from_string(&encrypted).unwrap();

        assert_eq!(Mnemonic::from_str(&decrypted).unwrap(), expected);
    }

    #[test]
    fn xpriv_is_tagged_under_wallet_mnemonic_keys() {
        let access = FailingKeychain::default();
        let id = wallet_id();
        let expected = xpriv();
        let keychain = keychain(access.clone());
        keychain.save_wallet_secret(&id, WalletSecret::try_from(expected).unwrap()).unwrap();
        let encrypted = access.entry(&mnemonic_key_name(&id)).unwrap();
        let cryptor = access.entry(&mnemonic_cryptor_key_name(&id)).unwrap();
        let decrypted =
            Cryptor::try_from_string(&cryptor).unwrap().decrypt_from_string(&encrypted).unwrap();

        assert_eq!(*decrypted, format!("{TAG_PREFIX}{XPRIV_TAG}{expected}"));
        assert!(Mnemonic::from_str(&decrypted).is_err());
    }

    #[test]
    fn debug_output_is_redacted() {
        let mnemonic = mnemonic();
        let mnemonic_debug = format!("{:?}", WalletSecret::Mnemonic(mnemonic.clone()));
        let xprv = WalletXprv::try_from(xpriv()).unwrap();
        let xprv_debug = format!("{xprv:?}");

        assert!(!mnemonic_debug.contains(&mnemonic.to_string()));
        assert!(!xprv_debug.contains(xprv.expose()));
    }

    #[test]
    fn delete_wallet_secret_removes_both_entries() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        keychain.save_wallet_key(&id, mnemonic()).unwrap();

        assert!(keychain.delete_wallet_secret(&id));
        assert!(keychain.get_wallet_secret(&id).unwrap().is_none());
    }

    #[test]
    fn delete_wallet_items_removes_wallet_secret() {
        let keychain = keychain(MockKeychain::default());
        let id = wallet_id();
        keychain.save_wallet_key(&id, mnemonic()).unwrap();

        assert!(keychain.delete_wallet_items(&id));
        assert!(keychain.get_wallet_secret(&id).unwrap().is_none());
    }
}
