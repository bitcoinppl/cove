use bdk_wallet::{
    bitcoin::{bip32::Xpub, key::Secp256k1, Network},
    keys::{DerivableKey as _, ExtendedKey},
};
use bip39::Mnemonic;
use itertools::Itertools as _;
use rand::Rng as _;

use crate::keys::Descriptors;

// word access
pub trait WordAccess {
    fn bip_39_words_groups_of(&self, groups: usize) -> Vec<Vec<GroupedWord>>;
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

// public key
pub trait MnemonicExt {
    fn into_descriptors(
        self,
        passphrase: Option<String>,
        network: impl Into<crate::network::Network>,
    ) -> Descriptors;

    fn xpub(&self, network: Network) -> Xpub;
}

impl MnemonicExt for Mnemonic {
    fn into_descriptors(
        self,
        passphrase: Option<String>,
        network: impl Into<crate::network::Network>,
    ) -> Descriptors {
        use crate::keys::{Descriptor, DescriptorSecretKey};

        let network = network.into();
        let descriptor_secret_key = DescriptorSecretKey::new(network, self, passphrase);

        let descriptor = Descriptor::new_bip84(
            &descriptor_secret_key,
            bdk_wallet::KeychainKind::External,
            network,
        );

        let change_descriptor = Descriptor::new_bip84(
            &descriptor_secret_key,
            bdk_wallet::KeychainKind::Internal,
            network,
        );

        Descriptors {
            external: descriptor,
            internal: change_descriptor,
        }
    }

    fn xpub(&self, network: Network) -> Xpub {
        let seed = self.to_seed("");
        let xkey: ExtendedKey = seed
            .into_extended_key()
            .expect("never fail proper mnemonic");

        xkey.into_xpub(network, &Secp256k1::new())
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

    pub fn in_groups_of(&self, groups_of: usize) -> Vec<Vec<String>> {
        let number_of_groups = self.to_word_count() / groups_of;
        vec![vec![String::new(); groups_of]; number_of_groups]
    }
}

mod ffi {
    use super::*;
    use crate::{
        keychain::{Keychain, KeychainError},
        wallet::metadata::{WalletId, WalletMetadata},
    };

    type Error = MnemonicError;

    #[derive(Debug, thiserror::Error, uniffi::Error)]
    pub enum MnemonicError {
        #[error("failed to get wallet keychain")]
        GetWalletKeychain(#[from] KeychainError),

        #[error("mnemonic is not available available for wallet id: {0}")]
        NotAvailable(WalletId),
    }

    #[uniffi::export]
    pub fn number_of_words_in_groups(me: NumberOfBip39Words, of: u8) -> Vec<Vec<String>> {
        me.in_groups_of(of as usize)
    }

    #[uniffi::export]
    pub fn number_of_words_to_word_count(me: NumberOfBip39Words) -> u8 {
        me.to_word_count() as u8
    }

    #[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, derive_more::Into)]
    pub struct Mnemonic(bip39::Mnemonic);

    #[uniffi::export]
    impl Mnemonic {
        #[uniffi::constructor(name = "preview")]
        pub fn new(number_of_bip39_words: NumberOfBip39Words) -> Self {
            Self(number_of_bip39_words.to_mnemonic())
        }

        #[uniffi::constructor(name = "new")]
        pub fn try_from_metadata(metadata: WalletMetadata) -> Result<Self, Error> {
            let keychain = Keychain::global();
            let mnemonic = keychain
                .get_wallet_key(&metadata.id)?
                .ok_or(Error::NotAvailable(metadata.id))?;

            Ok(Self(mnemonic))
        }

        #[uniffi::method]
        pub fn all_words(&self) -> Vec<GroupedWord> {
            self.0
                .bip_39_words_groups_of(1)
                .into_iter()
                .flatten()
                .collect()
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
