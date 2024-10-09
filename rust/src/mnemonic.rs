mod ext;
mod ffi;
mod grouped_word;
pub mod number_of_bip39_words;
pub mod parse;
pub mod word_access;

use crate::{keys::Descriptors, seed_qr::SeedQr};

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

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum MnemonicParseError {
    #[error("Invalid mnemonic, failed to parse as plain mnemonic: {0}, and as seed qr: {1}")]
    InvalidMnemonic(String, String),
}

#[uniffi::export]
pub fn grouped_plain_words_of(
    mnemonic: String,
    groups: u8,
) -> Result<Vec<Vec<String>>, MnemonicParseError> {
    match mnemonic.parse_mnemonic() {
        Ok(mnemonic) => Ok(mnemonic.grouped_plain_words_of(groups as usize)),

        Err(bip39_error) => {
            let seed_qr = SeedQr::try_from_str(&mnemonic).map_err(|seed_qr_error| {
                MnemonicParseError::InvalidMnemonic(
                    bip39_error.to_string(),
                    seed_qr_error.to_string(),
                )
            })?;

            Ok(seed_qr.mnemonic().grouped_plain_words_of(6))
        }
    }
}
