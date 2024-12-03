use bip39::Mnemonic;

use crate::mnemonic::{GroupedWord, NumberOfBip39Words, WordAccess as _};

#[derive(Debug, Clone, uniffi::Object)]
pub struct WordValidator {
    mnemonic: Mnemonic,
}

impl WordValidator {
    pub fn new(mnemonic: Mnemonic) -> Self {
        Self { mnemonic }
    }
}

#[uniffi::export]
impl WordValidator {
    // get the grouped words
    #[uniffi::method(default(groups_of = 12))]
    pub fn grouped_words(&self, groups_of: u8) -> Vec<Vec<GroupedWord>> {
        self.mnemonic.grouped_words_of(groups_of as usize)
    }

    // check if the word group passed in is valid
    #[uniffi::method]
    pub fn is_valid_word_group(&self, group_number: u8, entered_words: Vec<String>) -> bool {
        let actual_words = &self.mnemonic.grouped_words_of(6)[group_number as usize];

        for (actual_word, entered_word) in actual_words.iter().zip(entered_words.iter()) {
            if !entered_word.trim().eq_ignore_ascii_case(&actual_word.word) {
                return false;
            }
        }

        true
    }

    // check if all the word groups are valid
    #[uniffi::method]
    pub fn is_all_words_valid(&self, entered_words: Vec<Vec<String>>) -> bool {
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        for (actual_word, entered_word) in self.mnemonic.words().zip(entered_words) {
            if !entered_word.trim().eq_ignore_ascii_case(actual_word) {
                return false;
            }
        }

        true
    }

    // get string of all invalid words
    #[uniffi::method]
    pub fn invalid_words_string(&self, entered_words: Vec<Vec<String>>) -> String {
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        let mut invalid_words = Vec::new();
        for (index, (actual_word, entered_word)) in
            self.mnemonic.words().zip(entered_words).enumerate()
        {
            if !entered_word.trim().eq_ignore_ascii_case(actual_word) {
                invalid_words.push((index + 1).to_string());
            }
        }

        invalid_words.join(", ")
    }

    // preview only
    #[uniffi::constructor(name = "preview", default(number_of_words = None))]
    pub fn preview(preview: bool, number_of_words: Option<NumberOfBip39Words>) -> Self {
        assert!(preview);

        let number_of_words = number_of_words.unwrap_or(NumberOfBip39Words::Twelve);
        let mnemonic = number_of_words.to_mnemonic().clone();

        Self { mnemonic }
    }
}
