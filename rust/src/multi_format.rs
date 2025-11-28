//! Parsed format types representing the final result of scanning/parsing data.
//!
//! [`MultiFormat`] is a discriminated union of all supported data formats that can be
//! parsed from QR codes, NFC messages, or raw strings/bytes. This represents the "what" -
//! what type of data was scanned.
//!
//! For the scanning state machine that handles multi-part QR codes (BBQR, animated URs),
//! see [`crate::qr_scanner`].

use std::sync::Arc;

use cove_nfc::message::NfcMessage;
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
#[uniffi::export(Eq)]
pub enum MultiFormat {
    Address(Arc<AddressWithNetwork>),
    HardwareExport(Arc<HardwareExport>),
    Mnemonic(Arc<crate::mnemonic::Mnemonic>),
    Transaction(Arc<crate::transaction::ffi::BitcoinTransaction>),
    Bip329Labels(Arc<Bip329Labels>),
    /// TAPSIGNER has not been initialized yet
    TapSignerReady(Arc<cove_tap_card::TapSigner>),
    /// TAPSIGNER has not been initialized yet
    TapSignerUnused(Arc<cove_tap_card::TapSigner>),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum MultiFormatError {
    #[error(transparent)]
    InvalidSeedQr(#[from] crate::seed_qr::SeedQrError),

    #[error("Address is not supported for any network")]
    UnsupportedNetworkAddress,

    #[error(
        "Not a valid format, we only support addresses, SeedQr, mnemonic, descriptors and XPUBs"
    )]
    UnrecognizedFormat,

    #[error("Invalid TapSigner {0}")]
    InvalidTapSigner(cove_tap_card::TapCardParseError),

    #[error("Taproot wallets are not supported yet")]
    TaprootNotSupported,
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

    pub fn try_from_nfc_message(nfc_message: NfcMessage) -> Result<Self> {
        debug!("MultiFormat::try_from_nfc_message");

        match nfc_message {
            NfcMessage::Data(data) => Self::try_from_data(&data),
            NfcMessage::String(string) => Self::try_from_string(&string),
            NfcMessage::Both(string, data) => {
                Self::try_from_data(&data).or_else(|_| Self::try_from_string(&string))
            }
        }
    }

    pub fn try_from_string(string: &str) -> Result<Self> {
        debug!("MultiFormat::try_from_string");
        let string = string.trim();

        // try to parse UR format (single-part URs only)
        if string.to_ascii_lowercase().starts_with("ur:") {
            return Self::try_from_ur_string(string);
        }

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

        // try and parse bip329 labels
        if let Ok(labels) = bip329::Labels::try_from_str(string) {
            return Ok(Self::Bip329Labels(Arc::new(labels.into())));
        }

        if string.contains("tapsigner.com/start") {
            let tap_card = cove_tap_card::TapCard::parse(string)
                .map_err(|e| MultiFormatError::InvalidTapSigner(e.into()))?;

            match tap_card {
                cove_tap_card::TapCard::TapSigner(card) => {
                    return Ok(MultiFormat::from(card));
                }

                cove_tap_card::TapCard::SatsCard(_card) => {
                    unreachable!("tap card should not be a sats card");
                }
            }
        }

        warn!("could not parse string as MultiFormat: {string}");
        Err(MultiFormatError::UnrecognizedFormat)
    }

    /// Parse a single UR string (non-animated, complete UR)
    fn try_from_ur_string(ur_string: &str) -> Result<Self> {
        let ur = cove_ur::Ur::parse(ur_string).map_err(|_| MultiFormatError::UnrecognizedFormat)?;
        let ur_type_str = ur.ur_type();
        let ur_type = crate::ur::UrType::from_str(ur_type_str);
        let message = ur.message_bytes().ok_or(MultiFormatError::UnrecognizedFormat)?;

        Self::try_from_ur_payload(&message, &ur_type)
    }

    /// Convert UR payload to appropriate MultiFormat variant
    pub(crate) fn try_from_ur_payload(data: &[u8], ur_type: &crate::ur::UrType) -> Result<Self> {
        use crate::ur::UrType;

        match ur_type {
            UrType::CryptoPsbt => {
                // decode crypto-psbt CBOR structure
                let crypto_psbt = cove_ur::CryptoPsbt::decode(data.to_vec())
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // extract the unsigned transaction from the PSBT
                let psbt = crypto_psbt.psbt();
                let unsigned_tx = &psbt.unsigned_tx;

                // serialize the unsigned transaction
                let tx_bytes = bitcoin::consensus::serialize(unsigned_tx);

                // parse as BitcoinTransaction
                let txn = BitcoinTransaction::try_from_data(&tx_bytes)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;
                Ok(Self::Transaction(Arc::new(txn)))
            }

            UrType::CryptoSeed => {
                // decode crypto-seed CBOR structure
                let crypto_seed = cove_ur::CryptoSeed::decode(data.to_vec())
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;
                let entropy = crypto_seed.entropy();
                let mnemonic = bip39::Mnemonic::from_entropy(&entropy)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;
                Ok(Self::Mnemonic(Arc::new(mnemonic.into())))
            }

            UrType::CryptoOutput => {
                // decode crypto-output CBOR structure
                let crypto_output = cove_ur::CryptoOutput::decode(data.to_vec())
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // get descriptor string using inferred network from UR metadata
                let network = crypto_output.hdkey().infer_network();
                let descriptor = crypto_output
                    .descriptor_string(network)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // parse as hardware export
                let format = pubport::Format::try_new_from_str(&descriptor)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                let hardware_export = HardwareExport::new(format);
                Ok(Self::HardwareExport(hardware_export.into()))
            }

            UrType::CryptoHdkey => {
                // decode crypto-hdkey CBOR structure
                let crypto_hdkey = cove_ur::CryptoHdkey::decode(data.to_vec())
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // convert to xpub string using inferred network from UR metadata
                let network = crypto_hdkey.infer_network();
                let xpub_str = crypto_hdkey
                    .to_xpub_string(network)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // parse as hardware export
                let format = pubport::Format::try_new_from_str(&xpub_str)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                let hardware_export = HardwareExport::new(format);
                Ok(Self::HardwareExport(hardware_export.into()))
            }

            UrType::CryptoAccount => {
                let account = match cove_ur::CryptoAccount::from_cbor(data) {
                    Ok(acc) => acc,
                    Err(e) => {
                        warn!("Failed to decode CryptoAccount CBOR: {:?}", e);
                        return Err(MultiFormatError::UnrecognizedFormat);
                    }
                };

                let descriptor = account.get_preferred_descriptor();

                // if only P2TR available, return TaprootNotSupported error
                if descriptor.is_none() && account.is_taproot_only() {
                    warn!("CryptoAccount only has taproot descriptors");
                    return Err(MultiFormatError::TaprootNotSupported);
                }

                let descriptor = match descriptor {
                    Some(d) => d,
                    None => {
                        warn!("CryptoAccount has no supported descriptor");
                        return Err(MultiFormatError::UnrecognizedFormat);
                    }
                };

                let network = descriptor.hdkey.infer_network();
                let xpub_str = match descriptor.hdkey.to_xpub_string(network) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to convert hdkey to xpub: {:?}", e);
                        return Err(MultiFormatError::UnrecognizedFormat);
                    }
                };

                let format = match pubport::Format::try_new_from_str(&xpub_str) {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("Failed to parse xpub with pubport: {:?}", e);
                        return Err(MultiFormatError::UnrecognizedFormat);
                    }
                };

                let hardware_export = HardwareExport::new(format);
                Ok(Self::HardwareExport(hardware_export.into()))
            }

            UrType::Bytes => {
                // decode bytes - could be JSON (Passport format)
                let json_str =
                    std::str::from_utf8(data).map_err(|_| MultiFormatError::UnrecognizedFormat)?;

                // pubport already handles Passport JSON format
                let format = pubport::Format::try_new_from_str(json_str)
                    .map_err(|_| MultiFormatError::UnrecognizedFormat)?;
                let hardware_export = HardwareExport::new(format);
                Ok(Self::HardwareExport(hardware_export.into()))
            }

            UrType::Unknown(_) => {
                // unsupported UR type
                Err(MultiFormatError::UnrecognizedFormat)
            }
        }
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
fn multi_format_try_from_nfc_message(
    nfc_message: Arc<NfcMessage>,
) -> Result<MultiFormat, MultiFormatError> {
    let nfc_message = Arc::unwrap_or_clone(nfc_message);
    MultiFormat::try_from_nfc_message(nfc_message)
}

#[uniffi::export]
fn string_or_data_try_into_multi_format(
    string_or_data: StringOrData,
) -> Result<MultiFormat, MultiFormatError> {
    string_or_data.try_into()
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

impl From<cove_tap_card::TapSigner> for MultiFormat {
    fn from(tap_signer: cove_tap_card::TapSigner) -> Self {
        Self::from(Arc::new(tap_signer))
    }
}

impl From<Arc<cove_tap_card::TapSigner>> for MultiFormat {
    fn from(tap_signer: Arc<cove_tap_card::TapSigner>) -> Self {
        if tap_signer.state == cove_tap_card::TapSignerState::Unused {
            Self::TapSignerUnused(tap_signer)
        } else {
            Self::TapSignerReady(tap_signer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundation_ur::UR;

    // helper to create valid crypto-seed URs
    fn create_crypto_seed_ur(entropy: Vec<u8>) -> String {
        let crypto_seed = cove_ur::CryptoSeed::new(entropy);
        let cbor = crypto_seed.to_cbor().unwrap();
        let ur = UR::new("crypto-seed", &cbor);
        ur.to_string()
    }

    #[test]
    fn test_crypto_seed_ur_12_words() {
        // test 16-byte entropy (12-word mnemonic)
        let entropy = vec![
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];

        let ur_string = create_crypto_seed_ur(entropy.clone());
        let result = MultiFormat::try_from_ur_string(&ur_string).unwrap();

        match result {
            MultiFormat::Mnemonic(mnemonic) => {
                let expected = bip39::Mnemonic::from_entropy(&entropy).unwrap();
                assert_eq!(mnemonic.to_string(), expected.to_string());
            }
            _ => panic!("Expected Mnemonic variant"),
        }
    }

    #[test]
    fn test_crypto_seed_ur_24_words() {
        // test 32-byte entropy (24-word mnemonic)
        let entropy: Vec<u8> = (0..32).collect();

        let ur_string = create_crypto_seed_ur(entropy.clone());
        let result = MultiFormat::try_from_ur_string(&ur_string).unwrap();

        match result {
            MultiFormat::Mnemonic(mnemonic) => {
                let expected = bip39::Mnemonic::from_entropy(&entropy).unwrap();
                assert_eq!(mnemonic.to_string(), expected.to_string());
            }
            _ => panic!("Expected Mnemonic variant"),
        }
    }

    #[test]
    fn test_crypto_seed_ur_with_metadata() {
        // test that metadata (name, note, date) is handled gracefully
        let entropy = vec![0xaa; 16];
        let crypto_seed = cove_ur::CryptoSeed::with_metadata(
            entropy.clone(),
            Some("Test Wallet".to_string()),
            Some("Test note".to_string()),
            Some(1234567890),
        );

        let cbor = crypto_seed.to_cbor().unwrap();
        let ur = UR::new("crypto-seed", &cbor);
        let ur_string = ur.to_string();

        let result = MultiFormat::try_from_ur_string(&ur_string).unwrap();

        match result {
            MultiFormat::Mnemonic(_) => {} // success
            _ => panic!("Expected Mnemonic variant"),
        }
    }

    #[test]
    fn test_crypto_psbt_ur() {
        // use same test PSBT as cove-ur crate
        const TEST_PSBT_HEX: &str = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";

        let psbt_bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let crypto_psbt = cove_ur::CryptoPsbt::from_psbt_bytes(psbt_bytes).unwrap();
        let ur_string = crypto_psbt.to_ur().unwrap();

        let result = MultiFormat::try_from_ur_string(&ur_string).unwrap();

        match result {
            MultiFormat::Transaction(_) => {} // success
            _ => panic!("Expected Transaction variant"),
        }
    }

    #[test]
    fn test_malformed_crypto_hdkey_ur() {
        // truncated/malformed crypto-hdkey UR should return error
        let ur_string =
            "ur:crypto-hdkey/oeadgdstaslplabghydrpfmkbggufgludprfgmaotpiecffltnlpqdenos";

        let result = MultiFormat::try_from_ur_string(ur_string);
        assert!(result.is_err());
        assert!(matches!(result, Err(MultiFormatError::UnrecognizedFormat)));
    }

    #[test]
    fn test_malformed_ur_string() {
        // invalid bytewords encoding
        let result = MultiFormat::try_from_ur_string("ur:crypto-seed/invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_uppercase_single_part_ur() {
        // uppercase UR from QR scanner (BCR-2020-005 allows case-insensitive scheme)
        let ur_string = "UR:CRYPTO-OUTPUT/TAADMWTAADDLOSAOWKAXHDCLAXNSRSIMBNDRBNFTDEJSAXADLSMTWNDSAOWPLBIHFLSBEMLGMWCTDWDSFTFLDACPREAAHDCXMOCXBYKEGWNBDYADGHEMPYCFHGEYRYCATDTIWTWTLBGTSGPEGYECBDDARFHTFNLFAHTAADEHOEADAEAOAEAMTAADDYOTADLNCSGHYKAEYKAEYKAOCYGHENTSDKAXAXAYCYBGKBNBVAASIHFWGAGDEOESCLCFPSPY";
        let result = MultiFormat::try_from_string(ur_string);
        assert!(result.is_ok(), "Should parse uppercase UR: {:?}", result);
        assert!(matches!(result.unwrap(), MultiFormat::HardwareExport(_)));
    }
}
