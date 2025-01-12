use super::{Error, GroupedWord, Mnemonic, NumberOfBip39Words, WordAccess as _};
use crate::wallet::metadata::WalletId;

#[uniffi::export]
pub fn number_of_words_in_groups(me: NumberOfBip39Words, of: u8) -> Vec<Vec<String>> {
    me.in_groups_of(of as usize)
}

#[uniffi::export]
pub fn number_of_words_to_word_count(me: NumberOfBip39Words) -> u8 {
    me.to_word_count() as u8
}

#[uniffi::export]
impl Mnemonic {
    #[uniffi::constructor(name = "preview")]
    pub fn _ffi_preview_new(number_of_bip39_words: NumberOfBip39Words) -> Self {
        Self(number_of_bip39_words.to_mnemonic())
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
        self.0.words().map(|word| word.to_string()).collect()
    }
}
