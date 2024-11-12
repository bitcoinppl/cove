use bdk_wallet::bitcoin::Psbt as BdkPsbt;
use derive_more::{AsRef, Deref, From, Into};

use crate::transaction::Amount;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object, From, Deref, AsRef, Into)]
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
        println!("weight {}", self.0.unsigned_tx.vsize());
        println!("total f: {}", self.0.unsigned_tx.weight().to_vbytes_floor());
        println!("total c: {}", self.0.unsigned_tx.weight().to_vbytes_ceil());
        println!("total size: {}", self.0.unsigned_tx.total_size());

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
