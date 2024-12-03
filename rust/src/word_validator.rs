use bip39::Mnemonic;
use rand::seq::SliceRandom;

use crate::mnemonic::{GroupedWord, NumberOfBip39Words, WordAccess as _};

#[derive(Debug, Clone, uniffi::Object)]
pub struct WordValidator {
    mnemonic: Mnemonic,
    words: Vec<&'static str>,
}

impl WordValidator {
    pub fn new(mnemonic: Mnemonic) -> Self {
        let words = mnemonic.words().collect();
        Self { mnemonic, words }
    }
}

#[uniffi::export]
impl WordValidator {
    // get a word list of possible words for the word number
    #[uniffi::method]
    pub fn possible_words(&self, for_: u8) -> Vec<String> {
        let Some(word_index) = for_.checked_sub(1) else { return vec![] };
        let word_index = word_index as usize;
        if word_index > self.words.len() {
            return vec![];
        }

        let mut rng = rand::thread_rng();
        let correct_word = self.words[word_index as usize];

        let mut words_clone = self.words.clone();
        words_clone.shuffle(&mut rng);

        let new_words = NumberOfBip39Words::Twelve.to_mnemonic();

        let five_existing_words = words_clone.iter().take(5).cloned();

        let six_new_words = new_words.words().take(6);
        let correct = std::iter::once(correct_word);

        let mut combined: Vec<String> = five_existing_words
            .chain(six_new_words)
            .chain(correct)
            .map(|word| word.to_string())
            .collect();

        combined.shuffle(&mut rng);

        combined
    }

    // check if the selected word is correct
    #[uniffi::method]
    pub fn is_word_correct(&self, word: String, for_: u8) -> bool {
        let Some(word_index) = for_.checked_sub(1) else { return false };
        let word_index = word_index as usize;
        if word_index > self.words.len() {
            return false;
        }

        let correct_word = self.words[word_index];
        correct_word == word
    }

    // OLD API
    // TODO: remove this if no longer used

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

        Self::new(mnemonic)
    }
}
