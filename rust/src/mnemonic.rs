mod ext;
mod ffi;
mod grouped_word;
pub mod number_of_bip39_words;
pub mod parse;
pub mod word_access;

use crate::{
    keychain::{Keychain, KeychainError},
    keys::Descriptors,
    seed_qr::SeedQr,
    wallet::{WalletAddressType, metadata::WalletId},
};
use derive_more::{AsRef, Deref, From, Into};

use bdk_chain::bitcoin::{Network, bip32::Xpub};

pub type NumberOfBip39Words = number_of_bip39_words::NumberOfBip39Words;
pub type GroupedWord = grouped_word::GroupedWord;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, Into, From, AsRef, Deref)]
pub struct Mnemonic(bip39::Mnemonic);

pub type Error = MnemonicError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MnemonicError {
    #[error("failed to get wallet keychain")]
    GetWalletKeychain(#[from] KeychainError),

    #[error("mnemonic is not available available for wallet id: {0}")]
    NotAvailable(WalletId),
}

impl Mnemonic {
    pub fn try_from_id(id: &WalletId) -> Result<Self, Error> {
        let keychain = Keychain::global();
        let mnemonic = keychain
            .get_wallet_key(id)?
            .ok_or(Error::NotAvailable(id.clone()))?;

        Ok(Self(mnemonic))
    }

    pub fn generate_random(number_of_bip39_words: NumberOfBip39Words) -> Self {
        number_of_bip39_words.generate_mnemonic().into()
    }
}

// traits
pub trait WordAccess {
    fn grouped_words_of(&self, groups: usize) -> Vec<Vec<GroupedWord>>;
    fn grouped_plain_words_of(&self, groups: usize) -> Vec<Vec<String>>;
}

pub trait ParseMnemonic {
    fn parse_mnemonic(&self) -> Result<bip39::Mnemonic, bip39::Error>;
}

pub trait MnemonicExt {
    fn into_descriptors(
        self,
        passphrase: Option<String>,
        network: impl Into<cove_types::Network>,
        wallet_address_type: WalletAddressType,
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
