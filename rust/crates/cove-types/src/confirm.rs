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

/// QR code export format for PSBTs
#[derive(Debug, Clone, Copy, Default, Hash, Eq, PartialEq, derive_more::Display, uniffi::Enum)]
#[uniffi::export(Display)]
pub enum QrExportFormat {
    /// BBQr format (Binary Bitcoin QR)
    #[default]
    #[display("BBQr")]
    Bbqr,
    /// UR format (Uniform Resources)
    #[display("UR")]
    Ur,
}

/// QR code density settings for export
///
/// Controls how much data is packed into each QR code frame.
/// Higher density = larger/more complex QRs, fewer animation frames.
/// Lower density = smaller/simpler QRs, more animation frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Object)]
pub struct QrDensity {
    /// UR max fragment length in bytes (50-500, default 200)
    ur_fragment_len: u32,
    /// BBQr max version (5-40, default 15)
    bbqr_max_version: u8,
}

impl QrDensity {
    const UR_MIN: u32 = 50;
    const UR_MAX: u32 = 500;
    const UR_DEFAULT: u32 = 200;
    const UR_STEP: u32 = 50;

    const BBQR_MIN: u8 = 5;
    const BBQR_MAX: u8 = 40;
    const BBQR_DEFAULT: u8 = 15;
    const BBQR_STEP: u8 = 2;
}

impl Default for QrDensity {
    fn default() -> Self {
        Self { ur_fragment_len: Self::UR_DEFAULT, bbqr_max_version: Self::BBQR_DEFAULT }
    }
}

#[uniffi::export]
impl QrDensity {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increase density (larger QRs, fewer animation frames)
    pub fn increase(&self) -> Self {
        Self {
            ur_fragment_len: (self.ur_fragment_len + Self::UR_STEP).min(Self::UR_MAX),
            bbqr_max_version: (self.bbqr_max_version + Self::BBQR_STEP).min(Self::BBQR_MAX),
        }
    }

    /// Decrease density (smaller QRs, more animation frames)
    pub fn decrease(&self) -> Self {
        Self {
            ur_fragment_len: self.ur_fragment_len.saturating_sub(Self::UR_STEP).max(Self::UR_MIN),
            bbqr_max_version: self
                .bbqr_max_version
                .saturating_sub(Self::BBQR_STEP)
                .max(Self::BBQR_MIN),
        }
    }

    pub fn can_increase(&self) -> bool {
        self.ur_fragment_len < Self::UR_MAX || self.bbqr_max_version < Self::BBQR_MAX
    }

    pub fn can_decrease(&self) -> bool {
        self.ur_fragment_len > Self::UR_MIN || self.bbqr_max_version > Self::BBQR_MIN
    }

    pub fn ur_fragment_len(&self) -> u32 {
        self.ur_fragment_len
    }

    pub fn bbqr_max_version(&self) -> u8 {
        self.bbqr_max_version
    }
}

/// Check if two QrDensity values are equal (for Swift Equatable conformance)
#[uniffi::export]
pub fn qr_density_is_equal(lhs: &QrDensity, rhs: &QrDensity) -> bool {
    lhs == rhs
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
        self.psbt_to_bbqr_with_max_version(15)
    }

    /// Export PSBT as BBQr with specified max version
    pub fn psbt_to_bbqr_with_density(&self, density: &QrDensity) -> Result<Vec<String>> {
        self.psbt_to_bbqr_with_max_version(density.bbqr_max_version())
    }

    fn psbt_to_bbqr_with_max_version(&self, max_version: u8) -> Result<Vec<String>> {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let data = self.psbt.0.serialize();

        let version = Version::try_from(max_version).unwrap_or(Version::V15);

        let split = Split::try_from_data(
            data.as_slice(),
            FileType::Psbt,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 100,
                min_version: Version::V01,
                max_version: version,
            },
        )
        .map_err(|e| ConfirmDetailsError::QrCodeCreation(e.to_string()))?;

        Ok(split.parts)
    }

    /// Export PSBT as UR with specified density
    pub fn psbt_to_ur_with_density(&self, density: &QrDensity) -> Result<Vec<String>> {
        self.psbt_to_ur(density.ur_fragment_len())
    }

    /// Export PSBT as UR-encoded QR strings for animated display
    pub fn psbt_to_ur(&self, max_fragment_len: u32) -> Result<Vec<String>> {
        use cove_ur::CryptoPsbt;
        use foundation_ur::Encoder as UrEncoder;

        // wrap PSBT in CryptoPsbt and encode to tagged CBOR
        let crypto_psbt = CryptoPsbt::from_psbt_bytes(self.psbt.0.serialize()).map_err(|e| {
            ConfirmDetailsError::QrCodeCreation(format!("CryptoPsbt encoding failed: {}", e))
        })?;

        let cbor_psbt = crypto_psbt.encode().map_err(|e| {
            ConfirmDetailsError::QrCodeCreation(format!("CBOR encoding failed: {}", e))
        })?;

        let mut encoder = UrEncoder::new();
        encoder.start("crypto-psbt", &cbor_psbt, max_fragment_len as usize);

        let sequence_count = encoder.sequence_count() as usize;
        let mut parts = Vec::with_capacity(sequence_count);

        for _ in 0..sequence_count {
            let part = encoder.next_part();
            parts.push(part.to_string());
        }

        Ok(parts)
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

/// Preview ConfirmDetails for SwiftUI previews
#[uniffi::export]
pub fn confirm_details_preview_new() -> ConfirmDetails {
    let psbt = ffi_preview::psbt_preview_new();
    let more_details = InputOutputDetails::new(&psbt, Network::Bitcoin);

    ConfirmDetails {
        spending_amount: Amount::from_sat(20448),
        sending_amount: Amount::from_sat(20448 - 658),
        fee_total: Amount::from_sat(658),
        fee_rate: BdkFeeRate::from_sat_per_vb_unchecked(3).into(),
        fee_percentage: 3,
        sending_to: Address::preview_new(),
        psbt,
        more_details,
    }
}
