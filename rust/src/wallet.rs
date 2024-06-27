use crate::{
    keys::{Descriptor, DescriptorSecretKey},
    view_model::wallet::WalletId,
};
use bdk_wallet::{bitcoin::Network, KeychainKind};
use bip39::Mnemonic;
use itertools::Itertools as _;
use rand::Rng as _;

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

        let wallet =
            bdk_wallet::Wallet::new(descriptor.to_tuple(), change_descriptor.to_tuple(), network)
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
