use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TxId;

#[derive(Debug, Clone, PartialEq, uniffi::Object, Serialize, Deserialize)]
pub struct ConfirmDetails {
    pub txid: TxId,
    pub spending_amount_sats: u64,
    pub sending_amount_sats: u64,
    pub fee_total_sats: u64,
    pub fee_rate_sat_per_vb: f32,
    pub sending_to_address: String,
    pub psbt_hex: String,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record, Serialize, Deserialize)]
pub struct AddressAndAmount {
    pub address: String,
    pub amount_sats: u64,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, Serialize, Deserialize)]
pub struct InputOutputDetails {
    pub inputs: Vec<AddressAndAmount>,
    pub outputs: Vec<AddressAndAmount>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record, Serialize, Deserialize)]
pub struct SplitOutput {
    pub external: Vec<AddressAndAmount>,
    pub internal: Vec<AddressAndAmount>,
}

#[derive(Debug, Error, uniffi::Error)]
pub enum ConfirmDetailsError {
    #[error("unable to represent PSBT as QR code: {0}")]
    QrCodeCreation(String),
}

impl ConfirmDetails {
    pub fn id(&self) -> TxId {
        self.txid.clone()
    }

    pub fn id_hash(&self) -> String {
        self.txid.to_string()
    }

    pub fn normalized_id(&self) -> String {
        self.id_hash() // Simplified for the example
    }

    pub fn spending_amount_sats(&self) -> u64 {
        self.spending_amount_sats
    }

    pub fn sending_amount_sats(&self) -> u64 {
        self.sending_amount_sats
    }

    pub fn fee_total_sats(&self) -> u64 {
        self.fee_total_sats
    }

    pub fn fee_rate_sat_per_vb(&self) -> f32 {
        self.fee_rate_sat_per_vb
    }

    pub fn sending_to_address(&self) -> String {
        self.sending_to_address.clone()
    }

    pub fn psbt_hex(&self) -> String {
        self.psbt_hex.clone()
    }
}
