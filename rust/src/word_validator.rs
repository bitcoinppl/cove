use bip39::Mnemonic;
use rand::seq::SliceRandom as _;

use crate::mnemonic::NumberOfBip39Words;

#[derive(Debug, Clone, uniffi::Object)]
pub struct WordValidator {
    words: Vec<&'static str>,
}

impl WordValidator {
    pub fn new(mnemonic: Mnemonic) -> Self {
        let words = mnemonic.words().collect();
        Self { words }
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

        let mut rng = rand::rng();

        let correct_word = self.words[word_index];

        let five_existing_words = {
            let mut words_clone = self.words.clone();
            words_clone.shuffle(&mut rng);
            words_clone.into_iter().take(5)
        };

        let new_words = NumberOfBip39Words::Twelve.generate_mnemonic();
        let six_new_words = new_words.words().take(6);

        let mut combined: Vec<String> = Vec::with_capacity(12);
        combined.push(correct_word.to_string());
        combined.extend(five_existing_words.chain(six_new_words).map(ToString::to_string));

        // remove last word from the list, as long as it's not the correct word
        if word_index > 0 {
            let last_word = self.words[word_index - 1];
            combined.retain(|word| word != last_word || word == correct_word);
        }

        combined.sort();
        combined.dedup();

        // make sure we have 12 words
        while combined.len() < 12 {
            let needed = 12 - combined.len();
            let new_words = NumberOfBip39Words::Twelve.generate_mnemonic();

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
        let mnemonic = number_of_words.generate_mnemonic().clone();

        Self::new(mnemonic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_possible_words_result(validator: &WordValidator, word_position: u8) {
        let possible_words = validator.possible_words(word_position);

        assert_eq!(possible_words.len(), 12, "Should always return exactly 12 words");

        let correct_word = validator.words[(word_position - 1) as usize];
        assert!(
            possible_words.contains(&correct_word.to_string()),
            "Correct word '{correct_word}' should be in the possible words list"
        );

        let mut sorted_words = possible_words.clone();
        sorted_words.sort();
        assert_eq!(possible_words, sorted_words, "Words should be sorted");

        let unique_words: std::collections::HashSet<_> = possible_words.iter().collect();
        assert_eq!(unique_words.len(), possible_words.len(), "All words should be unique");
    }

    #[test]
    fn test_possible_words_random_mnemonic() {
        let mnemonic = NumberOfBip39Words::Twelve.generate_mnemonic();
        let validator = WordValidator::new(mnemonic.clone());

        for word_position in 1..=12u8 {
            validate_possible_words_result(&validator, word_position);
        }
    }

    #[test]
    fn test_possible_words_bacon_mnemonic() {
        let bacon_words = "bacon bacon bacon bacon bacon bacon bacon bacon bacon bacon bacon boil";
        let mnemonic = Mnemonic::parse(bacon_words).expect("should parse bacon words");
        let validator = WordValidator::new(mnemonic);

        for word_position in 1..=12u8 {
            validate_possible_words_result(&validator, word_position);
        }
    }

    #[test]
    fn test_possible_words_bacon_mnemonic_manual() {
        let bacon_words = "bacon bacon bacon bacon bacon bacon bacon bacon bacon bacon bacon boil";
        let mnemonic = Mnemonic::parse(bacon_words).expect("should parse bacon words");
        let validator = WordValidator::new(mnemonic);

        let possible_words = validator.possible_words(6);
        assert_eq!(possible_words.len(), 12, "Should always return exactly 12 words");
        assert!(possible_words.contains(&"bacon".to_string()));
    }

    #[test]
    fn test_possible_words_duplicate_words() {
        let duplicate_words = ["bacon"; 24].join(" ");
        let mnemonic = Mnemonic::parse(duplicate_words).expect("should parse duplicate words");
        let validator = WordValidator::new(mnemonic);

        for word_position in 1..=12u8 {
            validate_possible_words_result(&validator, word_position);
        }
    }

    #[test]
    fn test_with_random_mnemonic() {
        let mnemonic = NumberOfBip39Words::Twelve.generate_mnemonic();
        let validator = WordValidator::new(mnemonic.clone());

        for word_position in 1..=12u8 {
            validate_possible_words_result(&validator, word_position);
        }
    }

    #[test]
    fn test_possible_words_edge_cases() {
        let mnemonic = NumberOfBip39Words::Twelve.generate_mnemonic();
        let validator = WordValidator::new(mnemonic.clone());

        assert_eq!(validator.possible_words(0).len(), 0, "Should return empty for word position 0");
        assert_eq!(
            validator.possible_words(13).len(),
            0,
            "Should return empty for word position > 12"
        );
        assert_eq!(
            validator.possible_words(255).len(),
            0,
            "Should return empty for large word position"
        );
    }

    #[test]
    fn test_is_word_correct() {
        let mnemonic = NumberOfBip39Words::Twelve.generate_mnemonic();
        let validator = WordValidator::new(mnemonic.clone());

        for (i, word) in validator.words.iter().enumerate() {
            let word_position = (i + 1) as u8;
            assert!(
                validator.is_word_correct(word.to_string(), word_position),
                "Word '{word}' should be correct for position {word_position}"
            );
            assert!(
                !validator.is_word_correct("wrongword".to_string(), word_position),
                "Wrong word should not be correct for position {word_position}"
            );
        }
    }
}
