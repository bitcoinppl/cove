//! Parsing for signed transaction imports from hardware wallets
//!
//! Hardware wallets may return either:
//! - A signed but un-finalized PSBT (per BIP174 standard)
//! - A finalized raw Bitcoin transaction (hex encoded)
//!
//! This module provides [`SignedTransactionOrPsbt`] to detect and parse both formats.

use cove_util::result_ext::ResultExt as _;
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

    #[error("PSBT has no signatures — sign it with your hardware wallet before importing")]
    NotSigned,
}

type Error = SignedImportError;
type Result<T, E = Error> = std::result::Result<T, E>;

pub fn psbt_has_signatures(psbt: &Psbt) -> bool {
    psbt.0.inputs.iter().any(|input| {
        !input.partial_sigs.is_empty()
            || input.tap_key_sig.is_some()
            || !input.tap_script_sigs.is_empty()
            || input.final_script_sig.as_ref().is_some_and(|s| !s.is_empty())
            || input.final_script_witness.as_ref().is_some_and(|w| !w.is_empty())
    })
}

impl SignedTransactionOrPsbt {
    /// Try to parse from a string input (base64 or hex encoded)
    pub fn try_parse(input: &str) -> Result<Self> {
        let input = input.trim();

        // try PSBT first, propagate NotSigned so caller gets a clear error
        match Self::try_parse_psbt_string(input) {
            Ok(psbt) => return Ok(psbt),
            Err(Error::NotSigned) => return Err(Error::NotSigned),
            Err(_) => {}
        }

        // fall back to transaction parsing
        if let Ok(txn) = BitcoinTransaction::try_from_str(input) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        Err(Error::UnrecognizedFormat)
    }

    /// Try to parse from raw bytes
    pub fn try_from_bytes(data: &[u8]) -> Result<Self> {
        // check for PSBT magic bytes first
        if data.len() >= PSBT_MAGIC.len() && &data[..PSBT_MAGIC.len()] == PSBT_MAGIC {
            let psbt = Psbt::try_new(data.to_vec()).map_err_str(Error::PsbtParseError)?;
            if !psbt_has_signatures(&psbt) {
                return Err(Error::NotSigned);
            }
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
            NfcMessage::Both(string, data) => Self::try_from_bytes(data).or_else(|e| match e {
                Error::NotSigned => Err(Error::NotSigned),
                _ => Self::try_parse(string),
            }),
        }
    }

    /// Try to parse PSBT from string (base64 or hex)
    fn try_parse_psbt_string(input: &str) -> Result<Self> {
        // base64-encoded PSBT detection (case-sensitive for base64)
        if input.starts_with(PSBT_BASE64_PREFIX) {
            let bytes = BASE64_STANDARD.decode(input).map_err_str(Error::Base64DecodeError)?;

            let psbt = Psbt::try_new(bytes).map_err_str(Error::PsbtParseError)?;
            if !psbt_has_signatures(&psbt) {
                return Err(Error::NotSigned);
            }

            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        // hex-encoded PSBT detection (case-insensitive)
        let input_lower = input.to_ascii_lowercase();
        if input_lower.starts_with(PSBT_HEX_PREFIX) {
            let bytes = hex::decode(input).map_err_str(Error::HexDecodeError)?;

            let psbt = Psbt::try_new(bytes).map_err_str(Error::PsbtParseError)?;
            if !psbt_has_signatures(&psbt) {
                return Err(Error::NotSigned);
            }

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
    pub const fn is_psbt(&self) -> bool {
        matches!(self, Self::SignedPsbt(_))
    }

    /// Returns true if this is a finalized transaction
    pub const fn is_transaction(&self) -> bool {
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

#[uniffi::export]
fn signed_transaction_or_psbt_try_parse(input: String) -> Result<SignedTransactionOrPsbt> {
    SignedTransactionOrPsbt::try_parse(&input)
}

#[uniffi::export]
fn signed_transaction_or_psbt_try_from_bytes(data: Vec<u8>) -> Result<SignedTransactionOrPsbt> {
    SignedTransactionOrPsbt::try_from_bytes(&data)
}

#[uniffi::export]
fn signed_transaction_or_psbt_try_from_nfc_message(
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
    pub const fn ffi_is_psbt(&self) -> bool {
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

    // unsigned PSBT two inputs, no signatures
    const TEST_PSBT_HEX: &str = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";

    // inject a dummy witness push so psbt_has_signatures returns true
    fn make_signed_psbt_bytes() -> Vec<u8> {
        let mut psbt = Psbt::try_new(hex::decode(TEST_PSBT_HEX).unwrap()).unwrap();
        for input in &mut psbt.0.inputs {
            let mut witness = bitcoin::Witness::new();
            witness.push([0x01u8]);
            input.final_script_witness = Some(witness);
        }
        psbt.0.serialize()
    }

    #[test]
    fn test_parse_signed_psbt_hex() {
        let result = SignedTransactionOrPsbt::try_parse(&hex::encode(make_signed_psbt_bytes()));
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_parse_signed_psbt_base64() {
        let base64 = BASE64_STANDARD.encode(make_signed_psbt_bytes());
        let result = SignedTransactionOrPsbt::try_parse(&base64);
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_parse_signed_psbt_bytes() {
        let result = SignedTransactionOrPsbt::try_from_bytes(&make_signed_psbt_bytes());
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_tx_id_from_signed_psbt() {
        let parsed = SignedTransactionOrPsbt::try_from_bytes(&make_signed_psbt_bytes()).unwrap();
        let _tx_id = parsed.tx_id();
    }

    #[test]
    fn test_psbt_accessors() {
        let parsed = SignedTransactionOrPsbt::try_from_bytes(&make_signed_psbt_bytes()).unwrap();
        assert!(parsed.is_psbt());
        assert!(!parsed.is_transaction());
        assert!(parsed.psbt().is_some());
        assert!(parsed.transaction().is_none());
    }

    #[test]
    fn test_whitespace_handling() {
        let padded = format!("  {}  ", hex::encode(make_signed_psbt_bytes()));
        let result = SignedTransactionOrPsbt::try_parse(&padded);
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().is_psbt());
    }

    #[test]
    fn test_unsigned_psbt_hex_returns_not_signed() {
        let result = SignedTransactionOrPsbt::try_parse(TEST_PSBT_HEX);
        assert!(matches!(result, Err(SignedImportError::NotSigned)), "{result:?}");
    }

    #[test]
    fn test_unsigned_psbt_base64_returns_not_signed() {
        let base64 = BASE64_STANDARD.encode(hex::decode(TEST_PSBT_HEX).unwrap());
        let result = SignedTransactionOrPsbt::try_parse(&base64);
        assert!(matches!(result, Err(SignedImportError::NotSigned)), "{result:?}");
    }

    #[test]
    fn test_unsigned_psbt_bytes_returns_not_signed() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let result = SignedTransactionOrPsbt::try_from_bytes(&bytes);
        assert!(matches!(result, Err(SignedImportError::NotSigned)), "{result:?}");
    }

    #[test]
    fn test_invalid_input() {
        let result = SignedTransactionOrPsbt::try_parse("invalid data");
        assert!(matches!(result, Err(SignedImportError::UnrecognizedFormat)));
    }

    #[test]
    fn test_empty_input() {
        assert!(SignedTransactionOrPsbt::try_parse("").is_err());
    }
}
