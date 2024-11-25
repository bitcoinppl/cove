use std::sync::Arc;

use eyre::Context as _;

use crate::{
    hardware_export::HardwareExport,
    mnemonic::ParseMnemonic as _,
    transaction::ffi::BitcoinTransaction,
    wallet::{address::AddressError, AddressWithNetwork},
};

#[derive(Debug, Clone, uniffi::Enum)]
pub enum StringOrData {
    String(String),
    Data(Vec<u8>),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MultiFormat {
    Address(Arc<AddressWithNetwork>),
    HardwareExport(Arc<HardwareExport>),
    Mnemonic(Arc<crate::mnemonic::Mnemonic>),
    Transaction(Arc<crate::transaction::ffi::BitcoinTransaction>),
}

#[derive(Debug, uniffi::Error, thiserror::Error, derive_more::Display)]
pub enum MultiFormatError {
    #[error(transparent)]
    InvalidSeedQr(#[from] crate::seed_qr::SeedQrError),

    /// Address is not supported for any network
    UnsupportedNetworkAddress,

    /// Not a valid format, we only support addresses, SeedQr, mnemonic and xpubs
    UnrecognizedFormat,
}

type Result<T, E = MultiFormatError> = std::result::Result<T, E>;

impl MultiFormat {
    pub fn try_from_data(data: Vec<u8>) -> Result<Self> {
        let seed_qr = crate::seed_qr::SeedQr::try_from_data(data)?;
        let mnemonic = seed_qr.into_mnemonic();
        Ok(Self::Mnemonic(Arc::new(mnemonic.into())))
    }

    pub fn try_from_string(string: String) -> Result<Self> {
        // try to parse address
        match AddressWithNetwork::try_new(&string) {
            Ok(address) => return Ok(Self::Address(address.into())),

            Err(AddressError::UnsupportedNetwork) => {
                return Err(MultiFormatError::UnsupportedNetworkAddress)
            }

            _ => {}
        }

        // try to parse hardware export (xpub, json, descriptors...)
        if let Ok(format) = pubport::Format::try_new_from_str(&string) {
            let hardware_export = HardwareExport::new(format);
            return Ok(Self::HardwareExport(hardware_export.into()));
        }

        // try to parse seed qr
        if let Ok(seed_qr) = crate::seed_qr::SeedQr::try_from_str(&string) {
            let mnemonic = seed_qr.into_mnemonic();
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
        }

        // try to parse a mnemonic
        if let Ok(mnemonic) = string.as_str().parse_mnemonic() {
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
        }

        // try to parse a transaction
        if let Ok(txn) = deserialize_transaction(&string) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        Err(MultiFormatError::UnrecognizedFormat)
    }
}

impl StringOrData {
    pub fn new(data: Vec<u8>) -> Self {
        if let Ok(str) = std::str::from_utf8(&data) {
            Self::String(str.to_string())
        } else {
            Self::Data(data)
        }
    }
}

impl TryFrom<StringOrData> for MultiFormat {
    type Error = MultiFormatError;

    fn try_from(string_or_data: StringOrData) -> Result<Self, Self::Error> {
        match string_or_data {
            StringOrData::String(string) => Self::try_from_string(string),
            StringOrData::Data(data) => Self::try_from_data(data),
        }
    }
}

fn deserialize_transaction(tx_hex: &str) -> Result<BitcoinTransaction, eyre::Report> {
    let tx_bytes = hex::decode(tx_hex).context("Failed to decode hex")?;
    let transaction: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&tx_bytes).context("Failed to parse transaction")?;

    Ok(transaction.into())
}

#[uniffi::export]
fn string_or_data_try_into_multi_format(
    string_or_data: StringOrData,
) -> Result<MultiFormat, MultiFormatError> {
    string_or_data.try_into()
}
