use super::DeterministicRandomDraw;
use bdk_wallet::{
    WeightedUtxo,
    bitcoin::{Amount, FeeRate, Script, key::rand::RngCore},
    coin_selection::{
        BranchAndBoundCoinSelection, CoinSelectionAlgorithm, CoinSelectionResult, InsufficientFunds,
    },
};

#[derive(Debug, Clone, Default)]
pub struct CoveDefaultCoinSelection(BranchAndBoundCoinSelection<DeterministicRandomDraw>);

/// From default set in BDK
const DEFAULT_SIZE_OF_CHANGE: u64 = 8 + 1 + 22;

impl CoveDefaultCoinSelection {
    pub fn new(seed: u64) -> Self {
        Self(BranchAndBoundCoinSelection::new(
            DEFAULT_SIZE_OF_CHANGE,
            DeterministicRandomDraw::new(seed),
        ))
    }
}

impl CoinSelectionAlgorithm for CoveDefaultCoinSelection {
    fn coin_select<R: RngCore>(
        &self,
        required_utxos: Vec<WeightedUtxo>,
        optional_utxos: Vec<WeightedUtxo>,
        fee_rate: FeeRate,
        target_amount: Amount,
        drain_script: &Script,
        rand: &mut R,
    ) -> Result<CoinSelectionResult, InsufficientFunds> {
        self.0.coin_select(
            required_utxos,
            optional_utxos,
            fee_rate,
            target_amount,
            drain_script,
            rand,
        )
    }
}
