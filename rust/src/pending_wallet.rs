use bip39::Mnemonic;

use crate::{database::Database, network::Network, wallet::NumberOfBip39Words};

#[derive(Debug, uniffi::Object)]
pub struct PendingWallet {
    pub mnemonic: Mnemonic,
    pub network: Network,
    pub passphrase: Option<String>,
}

impl PendingWallet {
    pub fn new(number_of_words: NumberOfBip39Words, passphrase: Option<String>) -> Self {
        let network = Database::global().global_config.selected_network();

        let mnemonic = number_of_words.to_mnemonic().clone();

        Self {
            mnemonic,
            network,
            passphrase,
        }
    }

    pub fn words(&self) -> Vec<String> {
        self.words_iter().map(ToString::to_string).collect()
    }

    pub fn words_iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.mnemonic.word_iter()
    }
}
