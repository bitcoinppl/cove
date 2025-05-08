use bdk_wallet::{
    IsDust as _, WeightedUtxo,
    bitcoin::{Amount, FeeRate, Script, TxIn, Weight, consensus::serialize},
    coin_selection::{CoinSelectionResult, Excess, InsufficientFunds},
};

/// From: BDK
pub fn select_sorted_utxos(
    utxos: impl Iterator<Item = (bool, WeightedUtxo)>,
    fee_rate: FeeRate,
    target_amount: Amount,
    drain_script: &Script,
) -> Result<CoinSelectionResult, InsufficientFunds> {
    let mut selected_amount = Amount::ZERO;
    let mut fee_amount = Amount::ZERO;
    let selected = utxos
        .scan(
            (&mut selected_amount, &mut fee_amount),
            |(selected_amount, fee_amount), (must_use, weighted_utxo)| {
                if must_use || **selected_amount < target_amount + **fee_amount {
                    **fee_amount += fee_rate
                        * TxIn::default()
                            .segwit_weight()
                            .checked_add(weighted_utxo.satisfaction_weight)
                            .expect("`Weight` addition should not cause an integer overflow");
                    **selected_amount += weighted_utxo.utxo.txout().value;
                    Some(weighted_utxo.utxo)
                } else {
                    None
                }
            },
        )
        .collect::<Vec<_>>();

    let amount_needed_with_fees = target_amount + fee_amount;
    if selected_amount < amount_needed_with_fees {
        return Err(InsufficientFunds {
            needed: amount_needed_with_fees,
            available: selected_amount,
        });
    }

    let remaining_amount = selected_amount - amount_needed_with_fees;

    let excess = decide_change(remaining_amount, fee_rate, drain_script);

    Ok(CoinSelectionResult { selected, fee_amount, excess })
}

/// From: BDK
/// Decide if change can be created
///
/// - `remaining_amount`: the amount in which the selected coins exceed the target amount
/// - `fee_rate`: required fee rate for the current selection
/// - `drain_script`: script to consider change creation
pub fn decide_change(remaining_amount: Amount, fee_rate: FeeRate, drain_script: &Script) -> Excess {
    let drain_output_len = serialize(drain_script).len() + 8usize;
    let change_fee =
        fee_rate * Weight::from_vb(drain_output_len as u64).expect("overflow occurred");
    let drain_val = remaining_amount.checked_sub(change_fee).unwrap_or_default();

    if drain_val.is_dust(drain_script) {
        let dust_threshold = drain_script.minimal_non_dust();
        Excess::NoChange { dust_threshold, change_fee, remaining_amount }
    } else {
        Excess::Change { amount: drain_val, fee: change_fee }
    }
}
