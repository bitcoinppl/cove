mod ext;
mod ffi;
mod grouped_word;
pub mod number_of_bip39_words;
pub mod parse;
pub mod word_access;

use crate::keys::Descriptors;

use bdk_chain::bitcoin::{bip32::Xpub, Network};
use bip39::Mnemonic;

pub type NumberOfBip39Words = number_of_bip39_words::NumberOfBip39Words;
pub type GroupedWord = grouped_word::GroupedWord;

// traits
pub trait WordAccess {
    fn grouped_words_of(&self, groups: usize) -> Vec<Vec<GroupedWord>>;
    fn grouped_plain_words_of(&self, groups: usize) -> Vec<Vec<String>>;
}

pub trait ParseMnemonic {
    fn parse_mnemonic(&self) -> Result<Mnemonic, bip39::Error>;
}

pub trait MnemonicExt {
    fn into_descriptors(
        self,
        passphrase: Option<String>,
        network: impl Into<crate::network::Network>,
    ) -> Descriptors;

    fn xpub(&self, network: Network) -> Xpub;
}
