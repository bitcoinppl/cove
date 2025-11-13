use std::sync::Arc;

use bitcoin::params::Params;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BdkTxId, Network, TxId, address::Address, amount::Amount, fees::FeeRate, psbt::Psbt,
    utxo::UtxoType,
};
use bitcoin::{FeeRate as BdkFeeRate, TxOut};

use ahash::AHashMap as HashMap;

type Error = ConfirmDetailsError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, serde::Serialize, serde::Deserialize,
)]
pub struct ConfirmDetails {
    pub spending_amount: Amount,
    pub sending_amount: Amount,
    pub fee_total: Amount,
    pub fee_rate: FeeRate,
    pub fee_percentage: u64,
    pub sending_to: Address,
    pub psbt: Psbt,
    pub more_details: InputOutputDetails,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record, Serialize, Deserialize)]
pub struct AddressAndAmount {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub utxo_type: Option<UtxoType>,

    pub address: Arc<Address>,
    pub amount: Arc<Amount>,
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

#[derive(Debug, Default, Clone, Hash, Eq, PartialEq)]
pub struct ExtraItem {
    pub label: Option<String>,
    pub utxo_type: Option<UtxoType>,
}

#[uniffi::export]
impl ConfirmDetails {
    pub fn id(&self) -> TxId {
        self.psbt.0.unsigned_tx.compute_txid().into()
    }

    pub fn id_hash(&self) -> String {
        self.id().0.to_raw_hash().to_string()
    }

    pub fn normalized_id(&self) -> String {
        self.psbt.0.unsigned_tx.compute_ntxid().to_string()
    }

    pub fn spending_amount(&self) -> Amount {
        self.spending_amount
    }

    pub fn fee_percentage(&self) -> u64 {
        self.fee_percentage
    }

    pub fn sending_amount(&self) -> Amount {
        self.sending_amount
    }

    pub fn fee_total(&self) -> Amount {
        self.fee_total
    }

    pub fn fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    pub fn sending_to(&self) -> Address {
        self.sending_to.clone()
    }

    pub fn inputs(&self) -> Vec<AddressAndAmount> {
        self.more_details.inputs.clone()
    }

    pub fn outputs(&self) -> Vec<AddressAndAmount> {
        self.more_details.outputs.clone()
    }

    pub fn psbt(&self) -> Psbt {
        self.psbt.clone()
    }

    pub fn psbt_to_hex(&self) -> String {
        self.psbt.serialize_hex()
    }

    pub fn psbt_bytes(&self) -> Vec<u8> {
        self.psbt.0.serialize()
    }

    pub fn psbt_to_bbqr(&self) -> Result<Vec<String>> {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let data = self.psbt.0.serialize();

        let split = Split::try_from_data(
            data.as_slice(),
            FileType::Psbt,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 100,
                min_version: Version::V01,
                max_version: Version::V15,
            },
        )
        .map_err(|e| ConfirmDetailsError::QrCodeCreation(e.to_string()))?;

        Ok(split.parts)
    }
}

impl AddressAndAmount {
    pub fn try_new(tx_out: &TxOut, network: Network) -> eyre::Result<Self> {
        let address = bitcoin::Address::from_script(&tx_out.script_pubkey, Params::from(network))?;
        Ok(Self {
            label: None,
            utxo_type: None,
            address: Arc::new(address.into()),
            amount: Arc::new(tx_out.value.into()),
        })
    }

    pub fn try_new_with_extra_opt(
        tx_out: &TxOut,
        network: Network,
        extra: Option<ExtraItem>,
    ) -> eyre::Result<Self> {
        match extra {
            Some(extra) => Self::try_new_with_extra(tx_out, network, extra),
            None => Self::try_new(tx_out, network),
        }
    }

    pub fn try_new_with_extra(
        tx_out: &TxOut,
        network: Network,
        extra: ExtraItem,
    ) -> eyre::Result<Self> {
        let address = bitcoin::Address::from_script(&tx_out.script_pubkey, Params::from(network))?;
        Ok(Self {
            label: extra.label,
            utxo_type: extra.utxo_type,
            address: Arc::new(address.into()),
            amount: Arc::new(tx_out.value.into()),
        })
    }
}

impl InputOutputDetails {
    pub fn new(psbt: &Psbt, network: Network) -> Self {
        Self::new_with_labels_opt(psbt, network, None)
    }

    pub fn new_with_labels(
        psbt: &Psbt,
        network: Network,
        extra: HashMap<&BdkTxId, ExtraItem>,
    ) -> Self {
        Self::new_with_labels_opt(psbt, network, Some(extra))
    }

    fn new_with_labels_opt(
        psbt: &Psbt,
        network: Network,
        extra_map: Option<HashMap<&BdkTxId, ExtraItem>>,
    ) -> Self {
        let mut extra_map = extra_map;

        let inputs = psbt
            .utxos()
            .iter()
            .map(|(tx_in, tx_out)| {
                let extra = extra_map
                    .as_mut()
                    .and_then(|extras| extras.remove(&tx_in.previous_output.txid))
                    .unwrap_or_default();

                AddressAndAmount::try_new_with_extra(tx_out, network, extra)
            })
            .filter_map(Result::ok)
            .collect();

        let outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .map(|output| AddressAndAmount::try_new(output, network))
            .filter_map(Result::ok)
            .collect();

        Self { inputs, outputs }
    }
}

impl ExtraItem {
    pub fn new(label: Option<String>, utxo_type: Option<UtxoType>) -> Self {
        Self { label, utxo_type }
    }
}

// MARK: CONFIRM DETAILS PREVIEW
mod ffi_preview {
    use crate::psbt::BdkPsbt;

    use super::*;

    pub fn psbt_preview_new() -> Psbt {
        let psbt_hex = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";
        let psbt_bytes = hex::decode(psbt_hex).expect("unable to decode psbt hex");

        BdkPsbt::deserialize(&psbt_bytes).expect("unable to deserialize psbt").into()
    }
}

#[uniffi::export]
impl ConfirmDetails {
    #[uniffi::constructor(name = "previewNew", default(amount = 20448))]
    pub fn _ffi_preview_new(amount: u64) -> Self {
        let psbt = ffi_preview::psbt_preview_new();
        let more_details = InputOutputDetails::new(&psbt, Network::Bitcoin);

        Self {
            spending_amount: Amount::from_sat(amount),
            sending_amount: Amount::from_sat(amount - 658),
            fee_total: Amount::from_sat(658),
            fee_rate: BdkFeeRate::from_sat_per_vb_unchecked(3).into(),
            fee_percentage: 3,
            sending_to: Address::preview_new(),
            psbt,
            more_details,
        }
    }
}
