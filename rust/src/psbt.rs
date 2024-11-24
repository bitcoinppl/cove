use crate::transaction::Amount;
use derive_more::{AsRef, Deref, From, Into};
use std::fmt::Debug;

pub type BdkPsbt = bdk_wallet::bitcoin::Psbt;

#[derive(Clone, PartialEq, Eq, Hash, uniffi::Object, From, Deref, AsRef, Into)]
pub struct Psbt(BdkPsbt);

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, uniffi::Error, thiserror::Error, derive_more::Display,
)]
pub enum PsbtError {
    /// Missing UTXO
    MissingUtxo,

    /// Negative fee.
    NegativeFee,

    /// Fee overflow.
    FeeOverflow,

    /// Other PSBT error {0}
    Other(String),
}

type Error = PsbtError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl Psbt {
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
