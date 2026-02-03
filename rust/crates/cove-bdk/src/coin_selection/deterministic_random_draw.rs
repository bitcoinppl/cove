use bdk_wallet::{
    WeightedUtxo,
    bitcoin::{Amount, FeeRate, Script, key::rand::RngCore},
    coin_selection::{CoinSelectionAlgorithm, CoinSelectionResult, InsufficientFunds},
};
use rand::{RngCore as _, SeedableRng as _};

use crate::util::select_sorted_utxos;

/// Pull UTXOs at random until we have enough to meet the target.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicRandomDraw {
    seed: u64,
}

impl CoinSelectionAlgorithm for DeterministicRandomDraw {
    fn coin_select<R: RngCore>(
        &self,
        required_utxos: Vec<WeightedUtxo>,
        mut optional_utxos: Vec<WeightedUtxo>,
        fee_rate: FeeRate,
        target_amount: Amount,
        drain_script: &Script,
        _rand: &mut R,
    ) -> Result<CoinSelectionResult, InsufficientFunds> {
        // We put the required UTXOs first and then the randomize optional UTXOs to take as needed
        let utxos = {
            self.shuffle_slice(&mut optional_utxos);

            required_utxos
                .into_iter()
                .map(|utxo| (true, utxo))
                .chain(optional_utxos.into_iter().map(|utxo| (false, utxo)))
        };

        // select required UTXOs and then random optional UTXOs.
        select_sorted_utxos(utxos, fee_rate, target_amount, drain_script)
    }
}

impl DeterministicRandomDraw {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    // Modified version of the Fisher-Yates algorithm from BDK to use a custom RNG
    // The Knuth shuffling algorithm based on the original [Fisher-Yates method](https://en.wikipedia.org/wiki/Fisher%E2%80%93Yates_shuffle)
    pub fn shuffle_slice<T>(&self, list: &mut [T]) {
        let mut rng = rand::rngs::StdRng::seed_from_u64(self.seed);

        if list.is_empty() {
            return;
        }

        let mut current_index = list.len() - 1;
        while current_index > 0 {
            let random_index = rng.next_u32() as usize % (current_index + 1);
            list.swap(current_index, random_index);
            current_index -= 1;
        }
    }
}
