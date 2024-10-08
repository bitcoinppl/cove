use super::*;
use crate::{
    keychain::{Keychain, KeychainError},
    wallet::metadata::WalletId,
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
    pub fn try_from_id(id: WalletId) -> Result<Self, Error> {
        let keychain = Keychain::global();
        let mnemonic = keychain
            .get_wallet_key(&id)?
            .ok_or(Error::NotAvailable(id))?;

        Ok(Self(mnemonic))
    }

    #[uniffi::method]
    pub fn all_words(&self) -> Vec<GroupedWord> {
        self.0.grouped_words_of(1).into_iter().flatten().collect()
    }
}
