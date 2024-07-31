/// create mod to remove all logs
mod ffi {
    use crate::{impl_default_for, mnemonic::NumberOfBip39Words};

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
        pub fn new() -> Self {
            Self {
                max_auto_complete: 3,
            }
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
                .map(|w| w.to_string())
                .collect()
        }

        #[uniffi::method]
        fn is_valid_word(&self, word: String) -> bool {
            let word = word.to_ascii_lowercase();

            bip39::Language::English
                .word_list()
                .contains(&word.as_str())
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
                        crate::bip39::generate_possible_final_words(&all_words).unwrap_or_default();

                    if word.is_empty() {
                        return possible.into_iter().take(4).collect();
                    }

                    let word = word.to_ascii_lowercase();
                    possible
                        .into_iter()
                        .filter(|w| w.starts_with(&word))
                        .take(4)
                        .collect()
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
                        crate::bip39::generate_possible_final_words(&all_words).unwrap_or_default();

                    possible.contains(&word)
                }
            }
        }
    }
}
