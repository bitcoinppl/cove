use crate::mnemonic::NumberOfBip39Words;
use cove_macros::impl_default_for;

#[uniffi::export(with_foreign)]
pub trait AutoComplete: Send + Sync + std::fmt::Debug + 'static {
    fn autocomplete(&self, word: String) -> Vec<String>;
    fn is_valid_word(&self, word: String) -> bool;
}

impl_default_for!(Bip39AutoComplete);

#[derive(Debug, Copy, Clone, uniffi::Object)]
pub struct Bip39AutoComplete {
    max_auto_complete: usize,
}

#[derive(Debug, Copy, Clone, uniffi::Object)]
pub enum Bip39WordSpecificAutocomplete {
    Regular(Bip39AutoComplete),
    LastWord(NumberOfBip39Words),
}

#[uniffi::export]
impl Bip39AutoComplete {
    #[uniffi::constructor]
    pub const fn new() -> Self {
        Self { max_auto_complete: 3 }
    }
}

#[uniffi::export]
impl Bip39AutoComplete {
    /// Find the next invalid or empty field number
    #[uniffi::method]
    pub fn next_field_number(self, current_field_number: u8, entered_words: Vec<String>) -> u8 {
        let current_index = current_field_number.saturating_sub(1) as usize;

        // look over the entire group, this way we find the first empty or invalid word, even if
        // its an index before the current one
        for (index, word) in entered_words.iter().enumerate() {
            if index == current_index {
                continue;
            }

            // return the field number of the next empty or invalid word
            if word.is_empty() || !is_bip39_word(word) {
                return (index + 1) as u8;
            }
        }

        // no matches just stay the same
        current_field_number
    }
}

#[uniffi::export]
impl AutoComplete for Bip39AutoComplete {
    #[uniffi::method]
    fn autocomplete(&self, word: String) -> Vec<String> {
        if word.is_empty() {
            return vec![];
        }

        let word = word.to_ascii_lowercase();

        bip39::Language::English
            .word_list()
            .iter()
            .filter(|w| w.starts_with(&word))
            .take(self.max_auto_complete)
            .map(std::string::ToString::to_string)
            .collect()
    }

    #[uniffi::method]
    fn is_valid_word(&self, word: String) -> bool {
        is_bip39_word(&word)
    }
}

#[uniffi::export]
impl Bip39WordSpecificAutocomplete {
    #[uniffi::constructor]
    pub fn new(word_number: u16, number_of_words: NumberOfBip39Words) -> Self {
        match (word_number, number_of_words) {
            (12, NumberOfBip39Words::Twelve) => Self::LastWord(number_of_words),
            (24, NumberOfBip39Words::TwentyFour) => Self::LastWord(number_of_words),
            _ => Self::Regular(Bip39AutoComplete::new()),
        }
    }

    #[uniffi::method]
    pub fn autocomplete(&self, word: String, all_words: Vec<Vec<String>>) -> Vec<String> {
        match self {
            Self::Regular(ac) => ac.autocomplete(word),
            Self::LastWord(number_of_words) => {
                let all_words = all_words
                    .into_iter()
                    .flatten()
                    .take(number_of_words.to_word_count() - 1)
                    .collect::<Vec<String>>()
                    .join(" ");

                let possible =
                    cove_bip39::generate_possible_final_words(&all_words).unwrap_or_default();

                if word.is_empty() {
                    return possible.into_iter().take(4).collect();
                }

                let word = word.to_ascii_lowercase();
                possible.into_iter().filter(|w| w.starts_with(&word)).take(4).collect()
            }
        }
    }

    #[uniffi::method]
    pub fn is_valid_word(&self, word: String, all_words: Vec<Vec<String>>) -> bool {
        match self {
            Self::Regular(ac) => ac.is_valid_word(word),
            Self::LastWord(number_of_words) => {
                let all_words = all_words
                    .into_iter()
                    .flatten()
                    .take(number_of_words.to_word_count() - 1)
                    .collect::<Vec<String>>()
                    .join(" ");

                let possible =
                    cove_bip39::generate_possible_final_words(&all_words).unwrap_or_default();

                possible.contains(&word)
            }
        }
    }

    #[uniffi::method]
    pub fn is_bip39_word(&self, word: String) -> bool {
        match self {
            Self::Regular(ac) => ac.is_valid_word(word),
            Self::LastWord(_number_of_words) => is_bip39_word(&word),
        }
    }

    #[uniffi::method]
    pub fn next_field_number(&self, current_field_number: u8, entered_words: Vec<String>) -> u8 {
        match self {
            Self::Regular(ac) => ac.next_field_number(current_field_number, entered_words),
            Self::LastWord(_) => current_field_number,
        }
    }
}

fn is_bip39_word(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    bip39::Language::English.word_list().contains(&word.as_str())
}
