// Re-export the cove_nfc crate
pub use cove_nfc::*;

use crate::cove_nfc::message::NfcMessage;
use crate::multi_format::{MultiFormat, MultiFormatError};

/// Extension trait to add multi_format functionality to NfcMessage
/// in the main crate to avoid circular dependencies
pub trait NfcMessageExt {
    fn try_into_multi_format(&self) -> Result<MultiFormat, MultiFormatError>;
}

impl NfcMessageExt for NfcMessage {
    fn try_into_multi_format(&self) -> Result<MultiFormat, MultiFormatError> {
        match self {
            NfcMessage::Data(data) => MultiFormat::try_from_data(data),
            NfcMessage::String(nfc) => MultiFormat::try_from_string(nfc),
            NfcMessage::Both(string, data) => {
                MultiFormat::try_from_data(data).or_else(|_| MultiFormat::try_from_string(string))
            }
        }
    }
}