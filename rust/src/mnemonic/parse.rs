use bip39::{Error, Language, Mnemonic};

use super::ParseMnemonic;

impl ParseMnemonic for &str {
    fn parse_mnemonic(&self) -> Result<Mnemonic, Error> {
        let word_list = Language::English.word_list();

        let phrase = self
            .split_whitespace()
            .map(|word| word.to_string().to_ascii_lowercase())
            .filter_map(|word| word_list.iter().find(|w| w.starts_with(&word)))
            .copied()
            .collect::<Vec<&str>>()
            .join(" ");

        Mnemonic::parse_in(Language::English, &phrase)
    }
}

impl ParseMnemonic for String {
    fn parse_mnemonic(&self) -> Result<Mnemonic, Error> {
        self.as_str().parse_mnemonic()
    }
}
