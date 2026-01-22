//! Parsing for signed transaction imports from hardware wallets
//!
//! Hardware wallets may return either:
//! - A signed but un-finalized PSBT (per BIP174 standard)
//! - A finalized raw Bitcoin transaction (hex encoded)
//!
//! This module provides [`SignedTransactionOrPsbt`] to detect and parse both formats.

use std::sync::Arc;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use cove_nfc::message::NfcMessage;
use cove_types::{TxId, psbt::Psbt};

use crate::transaction::ffi::BitcoinTransaction;

/// PSBT magic bytes: "psbt" + 0xff
const PSBT_MAGIC: &[u8] = &[0x70, 0x73, 0x62, 0x74, 0xff];

/// Base64 encoding of PSBT magic prefix ("cHNidP" is base64 for bytes starting with "psbt")
const PSBT_BASE64_PREFIX: &str = "cHNidP";

/// Hex encoding of PSBT magic prefix
const PSBT_HEX_PREFIX: &str = "70736274ff";

/// Result of parsing a signed transaction import
///
/// Hardware wallets may return either a signed PSBT or a finalized transaction.
/// This enum allows callers to handle both cases appropriately.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SignedTransactionOrPsbt {
    /// A finalized raw Bitcoin transaction
    Transaction(Arc<BitcoinTransaction>),
    /// A signed but un-finalized PSBT (requires finalization before broadcast)
    SignedPsbt(Arc<Psbt>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum SignedImportError {
    #[error("Failed to decode hex: {0}")]
    HexDecodeError(String),

    #[error("Failed to decode base64: {0}")]
    Base64DecodeError(String),

    #[error("Failed to parse PSBT: {0}")]
    PsbtParseError(String),

    #[error("Unrecognized format: input is neither a valid PSBT nor transaction")]
    UnrecognizedFormat,
}

type Error = SignedImportError;
type Result<T, E = Error> = std::result::Result<T, E>;

impl SignedTransactionOrPsbt {
    /// Try to parse from a string input (base64 or hex encoded)
    pub fn try_parse(input: &str) -> Result<Self> {
        let input = input.trim();

        // try PSBT parsing first (more specific detection)
        if let Ok(psbt) = Self::try_parse_psbt_string(input) {
            return Ok(psbt);
        }

        // fall back to raw transaction parsing
        if let Ok(txn) = BitcoinTransaction::try_from_str(input) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        Err(Error::UnrecognizedFormat)
    }

    /// Try to parse from raw bytes
    pub fn try_from_bytes(data: &[u8]) -> Result<Self> {
        // check for PSBT magic bytes first
        if data.len() >= PSBT_MAGIC.len() && &data[..PSBT_MAGIC.len()] == PSBT_MAGIC {
            let psbt =
                Psbt::try_new(data.to_vec()).map_err(|e| Error::PsbtParseError(e.to_string()))?;
            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        // try parsing as raw transaction
        if let Ok(txn) = BitcoinTransaction::try_from_data(data) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        // maybe the bytes are actually string encoded (UTF-8 base64/hex)
        if let Ok(utf8_str) = std::str::from_utf8(data) {
            return Self::try_parse(utf8_str);
        }

        Err(Error::UnrecognizedFormat)
    }

    /// Try to parse from an NFC message
    pub fn try_from_nfc_message(nfc_message: &NfcMessage) -> Result<Self> {
        match nfc_message {
            NfcMessage::String(string) => Self::try_parse(string),
            NfcMessage::Data(data) => Self::try_from_bytes(data),
            NfcMessage::Both(string, data) => {
                Self::try_from_bytes(data).or_else(|_| Self::try_parse(string))
            }
        }
    }

    /// Try to parse PSBT from string (base64 or hex)
    fn try_parse_psbt_string(input: &str) -> Result<Self> {
        // base64-encoded PSBT detection (case-sensitive for base64)
        if input.starts_with(PSBT_BASE64_PREFIX) {
            let bytes = BASE64_STANDARD
                .decode(input)
                .map_err(|e| Error::Base64DecodeError(e.to_string()))?;

            let psbt = Psbt::try_new(bytes).map_err(|e| Error::PsbtParseError(e.to_string()))?;

            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        // hex-encoded PSBT detection (case-insensitive)
        let input_lower = input.to_ascii_lowercase();
        if input_lower.starts_with(PSBT_HEX_PREFIX) {
            let bytes = hex::decode(input).map_err(|e| Error::HexDecodeError(e.to_string()))?;

            let psbt = Psbt::try_new(bytes).map_err(|e| Error::PsbtParseError(e.to_string()))?;

            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        Err(Error::UnrecognizedFormat)
    }

    /// Get the transaction ID (works for both variants)
    pub fn tx_id(&self) -> TxId {
        match self {
            Self::Transaction(txn) => txn.tx_id(),
            Self::SignedPsbt(psbt) => psbt.tx_id(),
        }
    }

    /// Returns true if this is a signed PSBT
    pub fn is_psbt(&self) -> bool {
        matches!(self, Self::SignedPsbt(_))
    }

    /// Returns true if this is a finalized transaction
    pub fn is_transaction(&self) -> bool {
        matches!(self, Self::Transaction(_))
    }

    /// Get the inner transaction if this is a Transaction variant
    pub fn transaction(&self) -> Option<Arc<BitcoinTransaction>> {
        match self {
            Self::Transaction(txn) => Some(txn.clone()),
            Self::SignedPsbt(_) => None,
        }
    }

    /// Get the inner PSBT if this is a Psbt variant
    pub fn psbt(&self) -> Option<Arc<Psbt>> {
        match self {
            Self::SignedPsbt(psbt) => Some(psbt.clone()),
            Self::Transaction(_) => None,
        }
    }
}

// UniFFI standalone functions for fallible constructors
// (UniFFI only supports fallible constructors for Objects, not Enums)

/// Parse from string input (base64 or hex encoded)
#[uniffi::export(name = "signedTransactionOrPsbtTryParse")]
pub fn signed_transaction_or_psbt_try_parse(input: String) -> Result<SignedTransactionOrPsbt> {
    SignedTransactionOrPsbt::try_parse(&input)
}

/// Parse from raw bytes
#[uniffi::export(name = "signedTransactionOrPsbtTryFromBytes")]
pub fn signed_transaction_or_psbt_try_from_bytes(data: Vec<u8>) -> Result<SignedTransactionOrPsbt> {
    SignedTransactionOrPsbt::try_from_bytes(&data)
}

/// Parse from an NFC message
#[uniffi::export(name = "signedTransactionOrPsbtTryFromNfcMessage")]
pub fn signed_transaction_or_psbt_try_from_nfc_message(
    nfc_message: Arc<NfcMessage>,
) -> Result<SignedTransactionOrPsbt> {
    SignedTransactionOrPsbt::try_from_nfc_message(&nfc_message)
}

// UniFFI methods for the enum
#[uniffi::export]
impl SignedTransactionOrPsbt {
    /// Get the transaction ID
    #[uniffi::method(name = "txId")]
    pub fn ffi_tx_id(&self) -> TxId {
        self.tx_id()
    }

    /// Returns true if this is a signed PSBT
    #[uniffi::method(name = "isPsbt")]
    pub fn ffi_is_psbt(&self) -> bool {
        self.is_psbt()
    }

    /// Returns true if this is a finalized transaction
    #[uniffi::method(name = "isTransaction")]
    pub fn ffi_is_transaction(&self) -> bool {
        self.is_transaction()
    }

    /// Get the inner transaction (returns None if PSBT)
    #[uniffi::method(name = "transaction")]
    pub fn ffi_transaction(&self) -> Option<Arc<BitcoinTransaction>> {
        self.transaction()
    }

    /// Get the inner PSBT (returns None if Transaction)
    #[uniffi::method(name = "psbt")]
    pub fn ffi_psbt(&self) -> Option<Arc<Psbt>> {
        self.psbt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sample PSBT from cove-ur tests (hex encoded)
    const TEST_PSBT_HEX: &str = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";

    #[test]
    fn test_parse_psbt_hex() {
        let result = SignedTransactionOrPsbt::try_parse(TEST_PSBT_HEX);
        assert!(result.is_ok(), "Failed to parse hex PSBT: {:?}", result);
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_parse_psbt_base64() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let base64 = BASE64_STANDARD.encode(&bytes);

        let result = SignedTransactionOrPsbt::try_parse(&base64);
        assert!(result.is_ok(), "Failed to parse base64 PSBT: {:?}", result);
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_parse_psbt_bytes() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();

        let result = SignedTransactionOrPsbt::try_from_bytes(&bytes);
        assert!(result.is_ok(), "Failed to parse PSBT bytes: {:?}", result);
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_tx_id_from_psbt() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let parsed = SignedTransactionOrPsbt::try_from_bytes(&bytes).unwrap();

        // verify tx_id can be retrieved
        let _tx_id = parsed.tx_id();
    }

    #[test]
    fn test_psbt_accessors() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let parsed = SignedTransactionOrPsbt::try_from_bytes(&bytes).unwrap();

        assert!(parsed.is_psbt());
        assert!(!parsed.is_transaction());
        assert!(parsed.psbt().is_some());
        assert!(parsed.transaction().is_none());
    }

    #[test]
    fn test_invalid_input() {
        let result = SignedTransactionOrPsbt::try_parse("invalid data");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SignedImportError::UnrecognizedFormat));
    }

    #[test]
    fn test_empty_input() {
        let result = SignedTransactionOrPsbt::try_parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_whitespace_handling() {
        let psbt_with_whitespace = format!("  {}  ", TEST_PSBT_HEX);
        let result = SignedTransactionOrPsbt::try_parse(&psbt_with_whitespace);
        assert!(result.is_ok(), "Should handle whitespace: {:?}", result);
        assert!(result.unwrap().is_psbt());
    }
}
