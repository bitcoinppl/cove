mod ext;
mod grouped_word;
pub mod number_of_bip39_words;
pub mod parse;
pub mod word_access;

use crate::{
    keys::Descriptors,
    seed_qr::SeedQr,
    wallet::{WalletAddressType, metadata::WalletId},
};

use bitcoin::{Network, bip32::Xpub};
use cove_device::keychain::{Keychain, KeychainError};
use derive_more::{AsRef, Deref, From, Into};

pub type NumberOfBip39Words = number_of_bip39_words::NumberOfBip39Words;
pub type GroupedWord = grouped_word::GroupedWord;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, Into, From, AsRef, Deref)]
pub struct Mnemonic(bip39::Mnemonic);

pub type Error = MnemonicError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum MnemonicError {
    #[error("failed to get wallet keychain")]
    GetWalletKeychain(#[from] KeychainError),

    #[error("mnemonic is not available available for wallet id: {0}")]
    NotAvailable(WalletId),

    #[error("unknown word in mnemonic: {0}")]
    UnknownWord(String),
}

impl Mnemonic {
    pub fn try_from_id(id: &WalletId) -> Result<Self, Error> {
        let keychain = Keychain::global();
        let mnemonic =
            keychain.get_wallet_key(id)?.ok_or_else(|| Error::NotAvailable(id.clone()))?;

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
#[uniffi::export(Display)]
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

            Ok(seed_qr.mnemonic().grouped_plain_words_of(groups as usize))
        }
    }
}

// MARK: FFI
#[uniffi::export(name = "numberOfWordsInGroups")]
pub fn _ffi_number_of_words_in_groups(me: NumberOfBip39Words, of: u8) -> Vec<Vec<String>> {
    me.in_groups_of(of as usize)
}

#[uniffi::export(name = "numberOfWordsToWordCount")]
pub const fn _ffi_number_of_words_to_word_count(me: NumberOfBip39Words) -> u8 {
    me.to_word_count() as u8
}

#[uniffi::export]
impl Mnemonic {
    #[uniffi::constructor(name = "preview")]
    pub fn _ffi_preview_new(number_of_bip39_words: NumberOfBip39Words) -> Self {
        Self(number_of_bip39_words.generate_mnemonic())
    }

    #[uniffi::constructor(name = "new")]
    pub fn _ffi_try_from_id(id: WalletId) -> Result<Self, Error> {
        Self::try_from_id(&id)
    }

    #[uniffi::method]
    pub fn all_words(&self) -> Vec<GroupedWord> {
        self.0.grouped_words_of(1).into_iter().flatten().collect()
    }

    #[uniffi::method]
    pub fn words(&self) -> Vec<String> {
        self.0.words().map(std::string::ToString::to_string).collect()
    }

    /// Converts mnemonic to SeedQR standard format string
    /// Each word is converted to its 4-digit BIP39 index (0000-2047)
    #[uniffi::method]
    pub fn to_seed_qr_string(&self) -> Result<String, MnemonicError> {
        use bip39::Language;
        let word_list = Language::English.word_list();

        self.0
            .words()
            .map(|word| {
                word_list
                    .iter()
                    .position(|&w| w == word)
                    .map(|index| format!("{index:04}"))
                    .ok_or_else(|| MnemonicError::UnknownWord(word.to_string()))
            })
            .collect()
    }
}
