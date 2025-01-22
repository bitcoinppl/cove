use crate::transaction::{Amount, TxId};
use bdk_wallet::psbt::PsbtUtils as _;
use bitcoin::TxOut;
use derive_more::{AsRef, Deref, From, Into};
use std::fmt::Debug;

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
}

impl Psbt {
    /// Get all UTXOs
    pub fn utxos(&self) -> Option<Vec<TxOut>> {
        let tx = &self.unsigned_tx;
        (0..tx.input.len())
            .map(|i| self.0.get_utxo_for(i))
            .collect()
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
