use std::sync::Arc;

use crate::{
    hardware_export::HardwareExport,
    mnemonic::ParseMnemonic as _,
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

        if let Ok(mnemonic) = string.as_str().parse_mnemonic() {
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
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