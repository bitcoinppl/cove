use bdk_wallet::psbt::PsbtUtils as _;
use bitcoin::{Amount as BdkAmount, TxIn, TxOut};
use derive_more::{AsRef, Deref, From, Into};
use std::fmt::Debug;

use crate::{TxId, amount::Amount};
pub type BdkPsbt = bdk_wallet::bitcoin::Psbt;

#[derive(
    Clone,
    PartialEq,
    Eq,
    Hash,
    uniffi::Object,
    From,
    Deref,
    AsRef,
    Into,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Psbt(pub BdkPsbt);

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Error, thiserror::Error)]
pub enum PsbtError {
    #[error("Missing UTXO")]
    MissingUtxo,

    #[error("Negative fee.")]
    NegativeFee,

    #[error("Fee overflow.")]
    FeeOverflow,

    #[error("Other PSBT error {0}")]
    Other(String),
}

type Error = PsbtError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl Psbt {
    #[uniffi::constructor(name = "new")]
    pub fn try_new(data: Vec<u8>) -> Result<Self> {
        let psbt = BdkPsbt::deserialize(&data).map_err(|e| PsbtError::Other(e.to_string()))?;
        Ok(psbt.into())
    }

    /// The virtual size of the transaction.
    pub fn weight(&self) -> u64 {
        self.0.unsigned_tx.vsize() as u64
    }

    /// Total fee in sats.
    pub fn fee(&self) -> Result<Amount> {
        use bitcoin::psbt::Error as E;

        let fee = self.0.fee().map_err(|e| match e {
            E::MissingUtxo => PsbtError::MissingUtxo,
            E::NegativeFee => PsbtError::NegativeFee,
            E::FeeOverflow => PsbtError::FeeOverflow,
            e => PsbtError::Other(e.to_string()),
        })?;

        Ok(fee.into())
    }

    /// Get the transaction id of the unsigned transaction
    pub fn tx_id(&self) -> TxId {
        self.0.unsigned_tx.compute_txid().into()
    }

    /// Get total sending amount of all outputs
    pub fn output_total_amount(&self) -> Amount {
        let amount: BdkAmount = self.0.unsigned_tx.output.iter().map(|output| output.value).sum();

        amount.into()
    }
}

impl Psbt {
    /// Get all UTXOs
    pub fn utxos(&self) -> Vec<(TxIn, TxOut)> {
        self.utxos_iter().map(|(tx_in, tx_out)| (tx_in.clone(), tx_out)).collect()
    }

    pub fn utxos_iter(&self) -> impl Iterator<Item = (&TxIn, TxOut)> {
        let tx = &self.unsigned_tx;
        tx.input.iter().enumerate().filter_map(|(i, tx_in)| {
            let tx_out = self.0.get_utxo_for(i)?;
            Some((tx_in, tx_out))
        })
    }
}

impl Debug for Psbt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Psbt")
            .field("weight", &self.weight())
            .field("fee", &self.fee())
            .field("num_inputs", &self.0.inputs.len())
            .field("num_outputs", &self.0.outputs.len())
            .finish()
    }
}
