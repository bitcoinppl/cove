pub mod tap_card;

use std::sync::Arc;

use tap_card::TapSigner;
use tracing::{debug, warn};

use crate::{
    hardware_export::HardwareExport,
    mnemonic::ParseMnemonic as _,
    transaction::ffi::BitcoinTransaction,
    wallet::{AddressWithNetwork, address::AddressError},
};

/// A string or data, could be a string or data (bytes)
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
    Bip329Labels(Arc<Bip329Labels>),
    /// TAPSIGNER has not been initialized yet
    TapSigner(TapSigner),
    /// TAPSIGNER has not been initialized yet
    TapSignerInit(TapSigner),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum MultiFormatError {
    #[error(transparent)]
    InvalidSeedQr(#[from] crate::seed_qr::SeedQrError),

    #[error("Address is not supported for any network")]
    UnsupportedNetworkAddress,

    #[error(
        "Not a valid format, we only support addresses, SeedQr, mnemonic, descriptors and XPUBs"
    )]
    UnrecognizedFormat,

    #[error("UR format not supported, please use a plain QR or a BBQr")]
    UrFormatNotSupported,

    #[error("Invalid TapSigner {0}")]
    InvalidTapSigner(tap_card::ffi::TapCardParseError),
}

type Result<T, E = MultiFormatError> = std::result::Result<T, E>;

impl MultiFormat {
    pub fn try_from_data(data: &[u8]) -> Result<Self> {
        debug!("MultiFormat::try_from_data");

        // try parsing a signed transaction
        if let Ok(txn) = BitcoinTransaction::try_from_data(data) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        // try parsing a seed qr
        if let Ok(seed_qr) = crate::seed_qr::SeedQr::try_from_data(data) {
            let mnemonic = seed_qr.into_mnemonic();
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
        }

        Err(MultiFormatError::UnrecognizedFormat)
    }

    pub fn try_from_string(string: &str) -> Result<Self> {
        debug!("MultiFormat::try_from_string");

        let string = string.trim();

        // try to parse address
        match AddressWithNetwork::try_new(string) {
            Ok(address) => return Ok(Self::Address(address.into())),

            Err(AddressError::UnsupportedNetwork) => {
                return Err(MultiFormatError::UnsupportedNetworkAddress);
            }

            _ => {}
        }

        // try to parse hardware export (xpub, json, descriptors...)
        if let Ok(format) = pubport::Format::try_new_from_str(string) {
            let hardware_export = HardwareExport::new(format);
            return Ok(Self::HardwareExport(hardware_export.into()));
        }

        // try to parse seed qr
        if let Ok(seed_qr) = crate::seed_qr::SeedQr::try_from_str(string) {
            let mnemonic = seed_qr.into_mnemonic();
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
        }

        // try to parse a mnemonic
        if let Ok(mnemonic) = string.parse_mnemonic() {
            return Ok(Self::Mnemonic(Arc::new(mnemonic.into())));
        }

        // try to parse a transaction
        if let Ok(txn) = BitcoinTransaction::try_from_str(string) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        if string.starts_with("UR:") || string.starts_with("ur:") {
            return Err(MultiFormatError::UrFormatNotSupported);
        }

        // try and parse bip329 labels
        if let Ok(labels) = bip329::Labels::try_from_str(string) {
            return Ok(Self::Bip329Labels(Arc::new(labels.into())));
        }

        if string.contains("tapsigner.com/start") {
            let tap_card = tap_card::TapCard::parse(string)
                .map_err(|e| MultiFormatError::InvalidTapSigner(e.into()))?;

            match tap_card {
                tap_card::TapCard::TapSigner(card) => {
                    return Ok(MultiFormat::from(card));
                }

                tap_card::TapCard::SatsCard(_card) => {
                    unreachable!("tap card should not be a sats card");
                }
            }
        }

        warn!("could not parse string as MultiFormat: {string}");
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
            StringOrData::String(string) => Self::try_from_string(&string),
            StringOrData::Data(data) => Self::try_from_data(&data),
        }
    }
}

#[uniffi::export]
fn string_or_data_try_into_multi_format(
    string_or_data: StringOrData,
) -> Result<MultiFormat, MultiFormatError> {
    string_or_data.try_into()
}

#[uniffi::export]
fn display_multi_format_error(error: MultiFormatError) -> String {
    error.to_string()
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    uniffi::Object,
    derive_more::Into,
    derive_more::From,
    derive_more::Deref,
    derive_more::AsRef,
)]

pub struct Bip329Labels(pub bip329::Labels);

impl From<TapSigner> for MultiFormat {
    fn from(tap_signer: TapSigner) -> Self {
        if tap_signer.state == tap_card::TapSignerState::Unused {
            Self::TapSignerInit(tap_signer)
        } else {
            Self::TapSigner(tap_signer)
        }
    }
}
