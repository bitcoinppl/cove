use bip39::Mnemonic;

#[derive(Debug, Clone, uniffi::Object)]
pub struct WordValidator(Mnemonic);

impl WordValidator {
    pub fn new(mnemonic: Mnemonic) -> Self {
        Self(mnemonic)
    }
}

impl WordValidator {
    // check if the word group passed in is valid
    #[uniffi::method]
    pub fn is_valid_word_group(&self, group_number: u8, entered_words: Vec<String>) -> bool {
        let actual_words = &self.bip_39_words_grouped()[group_number as usize];

        for (actual_word, entered_word) in actual_words.iter().zip(entered_words.iter()) {
            if actual_word.word != entered_word.to_lowercase().trim() {
                return false;
            }
        }

        true
    }

    // check if all the word groups are valid
    #[uniffi::method]
    pub fn is_all_words_valid(&self, entered_words: Vec<Vec<String>>) -> bool {
        let state = self.state.read();
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        for (actual_word, entered_word) in state.wallet.words_iter().zip(entered_words) {
            if actual_word != entered_word.to_lowercase().trim() {
                return false;
            }
        }

        true
    }

    // get string of all invalid words
    #[uniffi::method]
    pub fn invalid_words_string(&self, entered_words: Vec<Vec<String>>) -> String {
        let state = self.state.read();
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        let mut invalid_words = Vec::new();
        for (index, (actual_word, entered_word)) in
            state.wallet.words_iter().zip(entered_words).enumerate()
        {
            if actual_word != entered_word.to_lowercase().trim() {
                invalid_words.push((index + 1).to_string());
            }
        }

        invalid_words.join(", ")
    }
}
