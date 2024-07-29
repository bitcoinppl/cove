use std::path::PathBuf;

use crate::{
    consts::ROOT_DATA_DIR,
    database::{self, Database},
    impl_default_for,
    keychain::KeychainError,
    keys::{Descriptor, DescriptorSecretKey},
    network::Network,
    new_type,
};
use bdk_file_store::Store;
use bdk_wallet::{
    bitcoin::bip32::Fingerprint, descriptor::ExtendedDescriptor, keys::DescriptorPublicKey,
    KeychainKind,
};
use bip39::Mnemonic;
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum WalletError {
    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("unsupported wallet: {0}")]
    UnsupportedWallet(String),

    #[error("failed to save wallet: {0}")]
    PersistError(String),

    #[error("failed to load wallet: {0}")]
    LoadError(String),

    #[error("failed to save in keychain: {0}")]
    KeychainError(#[from] KeychainError),

    #[error("failed to save in database: {0}")]
    DatabaseError(#[from] database::Error),

    #[error("wallet not found")]
    WalletNotFound,
}

new_type!(WalletId, String);
impl_default_for!(WalletId);
impl WalletId {
    pub fn new() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletMetadata {
    pub id: WalletId,
    pub name: String,
    pub color: WalletColor,
    pub verified: bool,
    pub network: Network,
}

impl WalletMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        let network = Database::global().global_config.selected_network();

        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::random(),
            verified: false,
            network,
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletColor {
    Red,
    Blue,
    Green,
    Yellow,
    Orange,
    Purple,
    Pink,
    Custom { r: u8, g: u8, b: u8 },
}

impl WalletColor {
    pub fn random() -> Self {
        let options = [
            WalletColor::Red,
            WalletColor::Blue,
            WalletColor::Green,
            WalletColor::Yellow,
            WalletColor::Orange,
            WalletColor::Purple,
            WalletColor::Pink,
        ];

        let random_index = rand::thread_rng().gen_range(0..options.len());
        options[random_index]
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct GroupedWord {
    pub number: u8,
    pub word: String,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum NumberOfBip39Words {
    Twelve,
    TwentyFour,
}

mod ffi {
    use super::*;

    #[uniffi::export]
    pub fn number_of_words_in_groups(me: NumberOfBip39Words, of: u8) -> Vec<Vec<String>> {
        me.in_groups_of(of as usize)
    }

    #[uniffi::export]
    pub fn number_of_words_to_word_count(me: NumberOfBip39Words) -> u8 {
        me.to_word_count() as u8
    }
}

impl NumberOfBip39Words {
    pub const fn to_word_count(self) -> usize {
        match self {
            NumberOfBip39Words::Twelve => 12,
            NumberOfBip39Words::TwentyFour => 24,
        }
    }

    pub const fn to_entropy_bits(self) -> usize {
        match self {
            NumberOfBip39Words::Twelve => 128,
            NumberOfBip39Words::TwentyFour => 256,
        }
    }

    pub const fn to_entropy_bytes(self) -> usize {
        self.to_entropy_bits() / 8
    }

    pub fn to_mnemonic(self) -> Mnemonic {
        match self {
            NumberOfBip39Words::Twelve => {
                // 128 / 8  = 16
                let random_bytes = rand::thread_rng().gen::<[u8; 16]>();
                Mnemonic::from_entropy(&random_bytes).expect("failed to create mnemonic")
            }
            NumberOfBip39Words::TwentyFour => {
                // 256 / 8  = 32
                let random_bytes = rand::thread_rng().gen::<[u8; 32]>();
                Mnemonic::from_entropy(&random_bytes).expect("failed to create mnemonic")
            }
        }
    }

    pub fn in_groups_of(&self, groups_of: usize) -> Vec<Vec<String>> {
        let number_of_groups = self.to_word_count() / groups_of;
        vec![vec![String::new(); groups_of]; number_of_groups]
    }
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub network: Network,
    pub bdk: bdk_wallet::PersistedWallet,
}

impl Wallet {
    pub fn try_new_persisted(
        number_of_words: NumberOfBip39Words,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        let mnemonic = number_of_words.to_mnemonic();
        Self::try_new_persisted_from_mnemonic(mnemonic, passphrase)
    }

    pub fn try_new_persisted_from_mnemonic(
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        let id = WalletId::new();
        let network = Database::global().global_config.selected_network();

        let descriptor_secret_key = DescriptorSecretKey::new(network, mnemonic.clone(), passphrase);

        let mut db = Store::<bdk_wallet::ChangeSet>::open_or_create_new(
            id.to_string().as_bytes(),
            data_path(&id),
        )
        .map_err(|error| WalletError::PersistError(error.to_string()))?;

        let descriptor =
            Descriptor::new_bip84(&descriptor_secret_key, KeychainKind::External, network);

        let change_descriptor =
            Descriptor::new_bip84(&descriptor_secret_key, KeychainKind::Internal, network);

        let wallet =
            bdk_wallet::Wallet::create(descriptor.to_tuple(), change_descriptor.to_tuple())
                .network(network.into())
                .create_wallet(&mut db)
                .map_err(|error| WalletError::BdkError(error.to_string()))?;

        Ok(Self {
            id,
            network,
            bdk: wallet,
        })
    }

    pub fn try_load_persisted(id: WalletId) -> Result<Self, WalletError> {
        let network = Database::global().global_config.selected_network();

        let mut db =
            Store::<bdk_wallet::ChangeSet>::open(id.to_string().as_bytes(), data_path(&id))
                .map_err(|error| WalletError::LoadError(error.to_string()))?;

        let wallet = bdk_wallet::Wallet::load()
            .load_wallet(&mut db)
            .map_err(|error| WalletError::LoadError(error.to_string()))?
            .ok_or(WalletError::WalletNotFound)?;

        Ok(Self {
            id,
            network,
            bdk: wallet,
        })
    }

    pub fn get_pub_key(&self) -> Result<DescriptorPublicKey, WalletError> {
        use bdk_wallet::miniscript::descriptor::ShInner;
        use bdk_wallet::miniscript::Descriptor;

        let extended_descriptor: ExtendedDescriptor =
            self.bdk.public_descriptor(KeychainKind::External).clone();

        let key = match extended_descriptor {
            Descriptor::Pkh(pk) => pk.into_inner(),
            Descriptor::Wpkh(pk) => pk.into_inner(),
            Descriptor::Tr(pk) => pk.internal_key().clone(),
            Descriptor::Sh(pk) => match pk.into_inner() {
                ShInner::Wpkh(pk) => pk.into_inner(),
                _ => {
                    return Err(WalletError::UnsupportedWallet(
                        "unsupported wallet bare descriptor not wpkh".to_string(),
                    ))
                }
            },
            // not sure
            Descriptor::Bare(pk) => pk.as_inner().iter_pk().next().unwrap(),
            // multi-sig
            Descriptor::Wsh(_pk) => {
                return Err(WalletError::UnsupportedWallet(
                    "unsupported wallet, multisig".to_string(),
                ))
            }
        };

        Ok(key)
    }

    pub fn master_fingerprint(&self) -> Result<Fingerprint, WalletError> {
        let key = self.get_pub_key()?;
        Ok(key.master_fingerprint())
    }
}

pub fn delete_data_path(wallet_id: &WalletId) -> Result<(), std::io::Error> {
    let path = data_path(wallet_id);
    std::fs::remove_file(path)?;
    Ok(())
}

fn data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_of_bip39_words() {
        assert_eq!(NumberOfBip39Words::Twelve.to_entropy_bits(), 128);
        assert_eq!(NumberOfBip39Words::TwentyFour.to_entropy_bits(), 256);

        assert_eq!(NumberOfBip39Words::Twelve.to_mnemonic().word_count(), 12);

        assert_eq!(
            NumberOfBip39Words::TwentyFour.to_mnemonic().word_count(),
            24
        );

        assert_eq!(
            NumberOfBip39Words::Twelve.to_word_count(),
            NumberOfBip39Words::Twelve.to_mnemonic().word_count()
        );

        assert_eq!(
            NumberOfBip39Words::TwentyFour.to_word_count(),
            NumberOfBip39Words::TwentyFour.to_mnemonic().word_count()
        );
    }

    #[test]
    fn test_fingerprint() {
        let mnemonic = Mnemonic::parse_normalized(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();

        let wallet = Wallet::try_new_persisted_from_mnemonic(mnemonic, None).unwrap();
        let fingerprint = wallet.master_fingerprint();

        assert_eq!("73c5da0a", fingerprint.unwrap().to_string().as_str());
    }
}
