use crate::wallet::GroupedWord;
use bdk_wallet::{
    bitcoin::{bip32::Xpub, key::Secp256k1, Network},
    keys::{DerivableKey as _, ExtendedKey},
};
use bip39::Mnemonic;
use itertools::Itertools as _;

// word access
pub trait WordAccess {
    fn bip_39_words_groups_of(&self, groups: usize) -> Vec<Vec<GroupedWord>>;
}

impl WordAccess for Mnemonic {
    fn bip_39_words_groups_of(&self, groups: usize) -> Vec<Vec<GroupedWord>> {
        self.word_iter()
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
}

// public key
pub trait MnemonicExt {
    fn xpub(&self, network: Network) -> Xpub;
}

impl MnemonicExt for Mnemonic {
    fn xpub(&self, network: Network) -> Xpub {
        let seed = self.to_seed("");
        let xkey: ExtendedKey = seed
            .into_extended_key()
            .expect("never fail proper mnemonic");

        xkey.into_xpub(network, &Secp256k1::new())
    }
}
