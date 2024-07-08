use crate::{
    impl_default_for,
    keys::{Descriptor, DescriptorSecretKey},
    new_type,
};
use bdk_wallet::{bitcoin, KeychainKind};
use bip39::Mnemonic;
use itertools::Itertools as _;
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

#[derive(
    Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum, derive_more::Display, strum::EnumIter,
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
}

impl WalletMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::random(),
            verified: false,
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

pub trait WordAccess {
    fn bip_39_words_groups_of(&self, groups: usize) -> Vec<Vec<GroupedWord>>;
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
    pub bdk: bdk_wallet::Wallet,

    pub mnemonic: Mnemonic,
    pub network: Network,
    pub passphrase: Option<String>,
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub bdk: bdk_wallet::Wallet,
}

impl PendingWallet {
    pub fn new(
        number_of_words: NumberOfBip39Words,
        network: Network,
        passphrase: Option<String>,
    ) -> Self {
        let mnemonic = number_of_words.to_mnemonic();
        let descriptor_secret_key =
            DescriptorSecretKey::new(network, mnemonic.clone(), passphrase.clone());

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

        Self {
            bdk: wallet,
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

impl WordAccess for Mnemonic {
    fn bip_39_words_groups_of(&self, groups: usize) -> Vec<Vec<GroupedWord>> {
        self.word_iter()
            .chunks(groups)
            .into_iter()
            .enumerate()
            .map(|(chunk_index, chunk)| {
                chunk
                    .into_iter()
                    .enumerate()
                    .map(|(index, word)| GroupedWord {
                        number: ((chunk_index * groups) + index + 1) as u8,
                        word: word.to_string(),
                    })
                    .collect()
            })
            .collect()
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
}
