use bdk_wallet::{
    WeightedUtxo,
    bitcoin::{Amount, FeeRate, Script, TxIn, key::rand::RngCore},
    coin_selection::{CoinSelectionAlgorithm, CoinSelectionResult, InsufficientFunds},
};

use crate::util::decide_change;

#[derive(Debug, Clone, Copy)]
pub struct ManualUtxoSelection;

impl CoinSelectionAlgorithm for ManualUtxoSelection {
    fn coin_select<R: RngCore>(
        &self,
        required_utxos: Vec<WeightedUtxo>,
        _optional_utxos: Vec<WeightedUtxo>,
        fee_rate: FeeRate,
        target_amount: Amount,
        drain_script: &Script,
        _rand: &mut R,
    ) -> Result<CoinSelectionResult, InsufficientFunds> {
        let mut selected = Vec::with_capacity(required_utxos.len());
        let mut selected_amount = Amount::ZERO;
        let mut fee_amount = Amount::ZERO;

        for weighted_utxo in required_utxos {
            let weight = TxIn::default()
                .segwit_weight()
                .checked_add(weighted_utxo.satisfaction_weight)
                .expect("`Weight` addition should not cause an integer overflow");

            fee_amount += fee_rate * weight;
            selected_amount += weighted_utxo.utxo.txout().value;
            selected.push(weighted_utxo.utxo);
        }

        let remaining_amount = selected_amount.checked_sub(target_amount).ok_or_else(|| {
            InsufficientFunds { needed: target_amount, available: selected_amount }
        })?;

        let excess = decide_change(remaining_amount, fee_rate, drain_script);
        Ok(CoinSelectionResult { selected, fee_amount, excess })
    }
}
