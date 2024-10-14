use bip39::Mnemonic;
use itertools::Itertools;

use super::{GroupedWord, WordAccess};

impl WordAccess for Mnemonic {
    fn grouped_words_of(&self, groups: usize) -> Vec<Vec<GroupedWord>> {
        self.words()
            .chunks(groups)
            .into_iter()
            .enumerate()
            .map(|(chunk_index, chunk)| {
                chunk
                    .into_iter()
                    .enumerate()
                    .map(|(index, word)| GroupedWord {
                        number: ((chunk_index * groups) + index + 1) as u8,
                        word: word.to_string(),
                    })
                    .collect()
            })
            .collect()
    }

    fn grouped_plain_words_of(&self, groups: usize) -> Vec<Vec<String>> {
        self.words()
            .chunks(groups)
            .into_iter()
            .map(|chunk| chunk.map(ToString::to_string).collect())
            .collect()
    }
}
