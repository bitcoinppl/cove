use crate::{
    database::Database,
    impl_default_for,
    keys::{Descriptor, DescriptorSecretKey},
    new_type,
};
use bdk_wallet::{
    bitcoin::{self, bip32::Fingerprint},
    descriptor::ExtendedDescriptor,
    keys::DescriptorPublicKey,
    KeychainKind,
};
use bip39::Mnemonic;
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    Eq,
    PartialEq,
    uniffi::Enum,
    derive_more::Display,
    strum::EnumIter,
    Serialize,
    Deserialize,
)]
pub enum Network {
    Bitcoin,
    Testnet,
}

#[uniffi::export]
pub fn network_to_string(network: Network) -> String {
    network.to_string()
}

#[uniffi::export]
pub fn all_networks() -> Vec<Network> {
    Network::iter().collect()
}

impl TryFrom<&str> for Network {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bitcoin" | "Bitcoin" => Ok(Network::Bitcoin),
            "testnet" | "Testnet" => Ok(Network::Testnet),
            _ => Err(format!("Unknown network: {}", value)),
        }
    }
}

new_type!(WalletId, String);
impl_default_for!(WalletId);
impl WalletId {
    pub fn new() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct WalletMetadata {
    pub id: WalletId,
    pub name: String,
    pub color: WalletColor,
    pub verified: bool,
    pub network: crate::wallet::Network,
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

#[derive(Debug, Copy, Clone, Serialize, Deserialize, uniffi::Enum)]
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
}

#[derive(Debug, uniffi::Object)]
pub struct PendingWallet {
    pub wallet: Wallet,
    pub mnemonic: Mnemonic,
    pub network: Network,
    pub passphrase: Option<String>,
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub bdk: bdk_wallet::Wallet,
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum Error {
    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("unsupported wallet: {0}")]
    UnsupportedWallet(String),
}

impl Wallet {
    pub fn try_new(
        number_of_words: NumberOfBip39Words,
        passphrase: Option<String>,
    ) -> Result<Self, Error> {
        let mnemonic = number_of_words.to_mnemonic();
        Self::try_new_from_mnemonic(mnemonic, passphrase)
    }

    pub fn try_new_from_mnemonic(
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, Error> {
        let network = Database::global().global_config.selected_network();

        let descriptor_secret_key = DescriptorSecretKey::new(network, mnemonic.clone(), passphrase);

        let descriptor =
            Descriptor::new_bip84(&descriptor_secret_key, KeychainKind::External, network);

        let change_descriptor =
            Descriptor::new_bip84(&descriptor_secret_key, KeychainKind::Internal, network);

        let wallet = bdk_wallet::Wallet::new(
            descriptor.to_tuple(),
            change_descriptor.to_tuple(),
            network.into(),
        )
        .expect("failed to create wallet");

        Ok(Self {
            id: WalletId::new(),
            bdk: wallet,
        })
    }

    pub fn get_pub_key(&self) -> Result<DescriptorPublicKey, Error> {
        use bdk_wallet::miniscript::descriptor::ShInner;
        use bdk_wallet::miniscript::Descriptor;

        let extended_descriptor: ExtendedDescriptor =
            self.bdk.public_descriptor(KeychainKind::External).clone();

        println!("extended descriptor: {extended_descriptor:#?}");

        let key = match extended_descriptor {
            Descriptor::Pkh(pk) => pk.into_inner(),
            Descriptor::Wpkh(pk) => pk.into_inner(),
            Descriptor::Tr(pk) => pk.internal_key().clone(),
            Descriptor::Sh(pk) => match pk.into_inner() {
                ShInner::Wpkh(pk) => pk.into_inner(),
                _ => {
                    return Err(Error::UnsupportedWallet(
                        "unsupported wallet bare descriptor not wpkh".to_string(),
                    ))
                }
            },
            // not sure
            Descriptor::Bare(pk) => pk.as_inner().iter_pk().next().unwrap(),
            // multi-sig
            Descriptor::Wsh(_pk) => {
                return Err(Error::UnsupportedWallet(
                    "unsupported wallet, multisig".to_string(),
                ))
            }
        };

        Ok(key)
    }

    pub fn master_fingerprint(&self) -> Result<Fingerprint, Error> {
        let key = self.get_pub_key()?;
        Ok(key.master_fingerprint())
    }
}

impl PendingWallet {
    pub fn new(number_of_words: NumberOfBip39Words, passphrase: Option<String>) -> Self {
        let network = Database::global().global_config.selected_network();

        let mnemonic = number_of_words.to_mnemonic().clone();

        let wallet = Wallet::try_new_from_mnemonic(mnemonic.clone(), passphrase.clone())
            .expect("failed to create wallet");

        Self {
            wallet,
            mnemonic,
            network,
            passphrase,
        }
    }

    pub fn words(&self) -> Vec<String> {
        self.words_iter().map(ToString::to_string).collect()
    }

    pub fn words_iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.mnemonic.word_iter()
    }
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => bitcoin::Network::Bitcoin,
            Network::Testnet => bitcoin::Network::Testnet,
        }
    }
}

impl From<bitcoin::Network> for Network {
    fn from(network: bitcoin::Network) -> Self {
        match network {
            bitcoin::Network::Bitcoin => Network::Bitcoin,
            bitcoin::Network::Testnet => Network::Testnet,
            network => panic!("unsupported network: {network:?}"),
        }
    }
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

        let wallet = Wallet::try_new_from_mnemonic(mnemonic, None).unwrap();
        let fingerprint = wallet.master_fingerprint();

        assert_eq!("73c5da0a", fingerprint.unwrap().to_string().as_str());
    }
}
