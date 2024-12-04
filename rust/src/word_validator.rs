use bip39::Mnemonic;
use rand::seq::SliceRandom;

use crate::mnemonic::NumberOfBip39Words;

#[derive(Debug, Clone, uniffi::Object)]
pub struct WordValidator {
    #[allow(dead_code)]
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
        if word_index >= self.words.len() {
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

        // remove last word from the list
        if word_index > 0 {
            let last_word = self.words[word_index - 1];
            combined.retain(|word| word != last_word);
        }

        combined.sort();
        combined.dedup();

        // make sure we have 12 words
        while combined.len() < 12 {
            let needed = 12 - combined.len();
            let new_words = NumberOfBip39Words::Twelve.to_mnemonic();

            new_words.words().take(needed).for_each(|word| {
                combined.push(word.to_string());
            });

            combined.sort();
            combined.dedup();
        }

        combined.sort_unstable();
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

    #[uniffi::method]
    pub fn is_complete(&self, word_number: u8) -> bool {
        word_number == self.words.len() as u8
    }

    // MARK: preview only
    #[uniffi::constructor(name = "preview", default(number_of_words = None))]
    pub fn preview(preview: bool, number_of_words: Option<NumberOfBip39Words>) -> Self {
        assert!(preview);

        let number_of_words = number_of_words.unwrap_or(NumberOfBip39Words::Twelve);
        let mnemonic = number_of_words.to_mnemonic().clone();

        Self::new(mnemonic)
    }
}
