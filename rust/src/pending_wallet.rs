use bip39::Mnemonic;

use crate::{database::Database, mnemonic::NumberOfBip39Words, network::Network};

#[derive(Debug, uniffi::Object)]
pub struct PendingWallet {
    pub mnemonic: Mnemonic,
    pub network: Network,
    #[allow(dead_code)]
    pub passphrase: Option<String>,
}

impl PendingWallet {
    pub fn new(number_of_words: NumberOfBip39Words, passphrase: Option<String>) -> Self {
        let network = Database::global().global_config.selected_network();

        let mnemonic = number_of_words.generate_mnemonic().clone();

        Self {
            mnemonic,
            network,
            passphrase,
        }
    }

    pub fn words(&self) -> Vec<String> {
        self.mnemonic.words().map(Into::into).collect()
    }
}
