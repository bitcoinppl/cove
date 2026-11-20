//! Finalizing PSBTs that a hardware signer sends back
//!
//! Signers may strip a signed PSBT down to its signatures before export to shrink the QR.
//! SeedSigner, for one, keeps only `partial_sigs` (or `final_scriptwitness` for taproot) and
//! drops every input's UTXO and derivation fields. Finalizing needs each input's previous output
//! to find the wallet key that signed it, so the wallet restores what it knows about its own
//! coins before finalizing
//!
//! BDK's finalizer places signatures without verifying them, so the finalized inputs are run
//! through the miniscript interpreter against their previous outputs before the transaction is
//! handed back for broadcast

use bdk_wallet::{
    SignOptions,
    bitcoin::{Psbt, Transaction, psbt::Input},
    miniscript::psbt::interpreter_check,
    psbt::PsbtUtils as _,
};
use tracing::warn;

use super::Wallet;

/// Why a signed PSBT could not become a transaction ready to broadcast
///
/// Its text is the body of the finalize alert, so each variant says what the user can do
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Enum)]
pub enum PsbtFinalizeFailure {
    #[error(
        "This transaction spends coins that aren't in this wallet. Check that you scanned the signed transaction for this wallet"
    )]
    InputsNotInWallet,

    #[error(
        "The signed transaction is missing signatures. Sign it again with the signer that holds this wallet's seed"
    )]
    MissingSignatures,

    #[error(
        "The signatures don't match this wallet's keys. Check that your signer is using this wallet's seed and passphrase"
    )]
    SignaturesDoNotMatch,

    #[error(
        "The signatures aren't valid for this transaction. Sign it again with the signer that holds this wallet's seed"
    )]
    InvalidSignatures,

    #[error(
        "The signed transaction has input data that doesn't match the transaction it spends. Export it from your signer again"
    )]
    MalformedInputs,

    #[error("Unable to finalize the signed transaction: {0}")]
    Finalizer(String),

    #[error("Unable to build the final transaction: {0}")]
    Extract(String),
}

/// Why one input could not be finalized, ordered from most to least fundamental
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum InputProblem {
    NotInWallet,
    Unsigned,
    SignaturesDoNotMatch,
}

impl From<InputProblem> for PsbtFinalizeFailure {
    fn from(problem: InputProblem) -> Self {
        match problem {
            InputProblem::NotInWallet => Self::InputsNotInWallet,
            InputProblem::Unsigned => Self::MissingSignatures,
            InputProblem::SignaturesDoNotMatch => Self::SignaturesDoNotMatch,
        }
    }
}

impl Wallet {
    /// Finalize a signed PSBT into a transaction ready to broadcast
    #[allow(deprecated)] // SignOptions usage required by bdk_wallet API, no replacement yet
    pub fn finalize_signed_psbt(&self, mut psbt: Psbt) -> Result<Transaction, PsbtFinalizeFailure> {
        if !self.input_utxos_are_consistent(&psbt) {
            return Err(PsbtFinalizeFailure::MalformedInputs);
        }

        self.restore_input_utxos(&mut psbt);

        let finalized = self
            .bdk
            .finalize_psbt(&mut psbt, SignOptions::default())
            .map_err(|error| PsbtFinalizeFailure::Finalizer(error.to_string()))?;

        if !finalized {
            return Err(self.diagnose_unfinalized(&psbt));
        }

        if let Err(error) = interpreter_check(&psbt, self.bdk.secp_ctx()) {
            warn!(%error, "finalized PSBT failed signature verification");
            return Err(PsbtFinalizeFailure::InvalidSignatures);
        }

        // use Display, since the Debug form embeds the whole PSBT or transaction
        psbt.extract_tx().map_err(|error| PsbtFinalizeFailure::Extract(error.to_string()))
    }

    /// Whether every previous output the PSBT carries is the one its input spends
    ///
    /// BDK and rust-bitcoin index a previous transaction's outputs without a bounds check, so a
    /// mismatched one would panic instead of failing. A segwit signature commits to the amount in
    /// `witness_utxo`, which BDK and miniscript prefer over the full transaction, so a wrong
    /// amount would verify here and fail only at broadcast
    fn input_utxos_are_consistent(&self, psbt: &Psbt) -> bool {
        let graph = self.bdk.tx_graph();

        psbt.unsigned_tx.input.iter().zip(&psbt.inputs).all(|(txin, input)| {
            let outpoint = txin.previous_output;
            let previous_tx_matches = input.non_witness_utxo.as_ref().is_none_or(|previous| {
                previous.compute_txid() == outpoint.txid
                    && previous.output.len() > outpoint.vout as usize
            });

            let witness_utxo_matches = match (&input.witness_utxo, graph.get_txout(outpoint)) {
                (Some(supplied), Some(known)) => supplied == known,
                _ => true,
            };

            previous_tx_matches && witness_utxo_matches
        })
    }

    /// Fill in the previous output of every input that arrived without one, from the
    /// transactions this wallet already knows
    fn restore_input_utxos(&self, psbt: &mut Psbt) {
        let graph = self.bdk.tx_graph();
        let inputs = psbt.unsigned_tx.input.iter().zip(psbt.inputs.iter_mut());

        for (txin, input) in inputs {
            if input.witness_utxo.is_some() || input.non_witness_utxo.is_some() {
                continue;
            }

            let outpoint = txin.previous_output;
            let Some(txout) = graph.get_txout(outpoint).cloned() else { continue };

            // the full previous transaction is valid for every script type, BIP174 asks for the
            // output alone on segwit inputs
            input.non_witness_utxo = graph.get_tx(outpoint.txid).map(|tx| (*tx).clone());
            if txout.script_pubkey.is_witness_program() {
                input.witness_utxo = Some(txout);
            }
        }
    }

    /// The most fundamental reason any input was left unfinalized
    fn diagnose_unfinalized(&self, psbt: &Psbt) -> PsbtFinalizeFailure {
        let problem = psbt
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| !is_finalized(input))
            .map(|(index, input)| self.input_problem(psbt, index, input))
            .min();

        if let Some(problem) = problem {
            warn!(?problem, "signed PSBT left inputs unfinalized");
            return problem.into();
        }

        // bdk reported failure with every input finalized, which its finalizer never does
        PsbtFinalizeFailure::Finalizer("every input is finalized, yet finalizing failed".into())
    }

    fn input_problem(&self, psbt: &Psbt, index: usize, input: &Input) -> InputProblem {
        let is_mine =
            psbt.get_utxo_for(index).is_some_and(|txout| self.bdk.is_mine(txout.script_pubkey));

        if !is_mine {
            return InputProblem::NotInWallet;
        }

        if !has_signatures(input) {
            return InputProblem::Unsigned;
        }

        InputProblem::SignaturesDoNotMatch
    }
}

fn is_finalized(input: &Input) -> bool {
    input.final_script_sig.is_some() || input.final_script_witness.is_some()
}

fn has_signatures(input: &Input) -> bool {
    !input.partial_sigs.is_empty()
        || input.tap_key_sig.is_some()
        || !input.tap_script_sigs.is_empty()
}
