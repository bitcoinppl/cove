//! Parsing for signed transaction imports from hardware wallets
//!
//! Hardware wallets may return either:
//! - A signed but un-finalized PSBT (per BIP174 standard)
//! - A finalized raw Bitcoin transaction (hex encoded)
//!
//! This module provides [`SignedTransactionOrPsbt`] to detect and parse both formats.

use cove_util::psbt::PsbtDecodeError;
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

/// UTF-8 byte order mark that some editors write at the start of text files
const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];

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

/// Why a signed transaction import could not be parsed
///
/// Its text reaches logs and the import alert, so no variant quotes the input
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum SignedImportError {
    // the decoder errors quote a character or byte of the input, so they are dropped
    #[error("Failed to decode hex")]
    HexDecodeError,

    #[error("Failed to decode base64")]
    Base64DecodeError,

    #[error("Failed to parse PSBT: {0}")]
    PsbtParseError(#[from] PsbtDecodeError),

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
        let psbt_error = match Self::try_parse_psbt_string(input) {
            Ok(psbt) => return Ok(psbt),
            Err(Error::NotSigned) => return Err(Error::NotSigned),
            Err(error) => error,
        };

        // fall back to transaction parsing
        if let Ok(txn) = BitcoinTransaction::try_from_str(input) {
            return Ok(Self::Transaction(Arc::new(txn)));
        }

        // a PSBT-prefixed input that failed to decode keeps that cause instead of looking like
        // an unknown format, the PSBT probe only returns UnrecognizedFormat for other inputs
        Err(psbt_error)
    }

    /// Try to parse from raw bytes
    pub fn try_from_bytes(data: &[u8]) -> Result<Self> {
        // check for PSBT magic bytes first
        if data.len() >= PSBT_MAGIC.len() && &data[..PSBT_MAGIC.len()] == PSBT_MAGIC {
            let psbt = Psbt::decode(data)?;
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
        let text = data.strip_prefix(UTF8_BOM).unwrap_or(data);
        if let Ok(utf8_str) = std::str::from_utf8(text) {
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
            let bytes = BASE64_STANDARD.decode(input).map_err(|_| Error::Base64DecodeError)?;

            let psbt = Psbt::decode(&bytes)?;
            if !psbt_has_signatures(&psbt) {
                return Err(Error::NotSigned);
            }

            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        // hex-encoded PSBT detection (case-insensitive)
        let input_lower = input.to_ascii_lowercase();
        if input_lower.starts_with(PSBT_HEX_PREFIX) {
            let bytes = hex::decode(input).map_err(|_| Error::HexDecodeError)?;

            let psbt = Psbt::decode(&bytes)?;
            if !psbt_has_signatures(&psbt) {
                return Err(Error::NotSigned);
            }

            return Ok(Self::SignedPsbt(Arc::new(psbt)));
        }

        Err(Error::UnrecognizedFormat)
    }

    /// Get the unsigned transaction ID that keys the saved unsigned transaction record
    pub fn unsigned_tx_id(&self) -> TxId {
        match self {
            Self::Transaction(txn) => txn.unsigned_tx_id(),
            Self::SignedPsbt(psbt) => psbt.tx_id(),
        }
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
    /// Get the unsigned transaction ID that keys the saved unsigned transaction record
    #[uniffi::method(name = "unsignedTxId")]
    pub fn ffi_unsigned_tx_id(&self) -> TxId {
        self.unsigned_tx_id()
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
        assert!(matches!(result.unwrap(), SignedTransactionOrPsbt::SignedPsbt(_)));
    }

    #[test]
    fn test_parse_signed_psbt_base64() {
        let base64 = BASE64_STANDARD.encode(make_signed_psbt_bytes());
        let result = SignedTransactionOrPsbt::try_parse(&base64);
        assert!(result.is_ok(), "{result:?}");
        assert!(matches!(result.unwrap(), SignedTransactionOrPsbt::SignedPsbt(_)));
    }

    #[test]
    fn test_parse_signed_psbt_bytes() {
        let result = SignedTransactionOrPsbt::try_from_bytes(&make_signed_psbt_bytes());
        assert!(result.is_ok(), "{result:?}");
        assert!(matches!(result.unwrap(), SignedTransactionOrPsbt::SignedPsbt(_)));
    }

    #[test]
    fn test_unsigned_tx_id_from_signed_psbt() {
        let parsed = SignedTransactionOrPsbt::try_from_bytes(&make_signed_psbt_bytes()).unwrap();
        assert_eq!(
            parsed.unsigned_tx_id().to_string(),
            "82efd652d7ab1197f01a5f4d9a30cb4c68bb79ab6fec58dfa1bf112291d1617b"
        );
    }

    #[test]
    fn test_whitespace_handling() {
        let padded = format!("  {}  ", hex::encode(make_signed_psbt_bytes()));
        let result = SignedTransactionOrPsbt::try_parse(&padded);
        assert!(result.is_ok(), "{result:?}");
        assert!(matches!(result.unwrap(), SignedTransactionOrPsbt::SignedPsbt(_)));
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

    // Signer Lab output for Cove signet hardware-wallet drafts made on iOS and Android: signed and
    // finalized binary PSBTs, each with the txid it was broadcast as
    const SIGNED_FIXTURES: [(&str, &[u8], &str); 2] = [
        (
            "ios draft",
            include_bytes!("../test/data/signet_ios_draft_signed.psbt"),
            "c3f4d4131161cebf766953a3597b1918dbbd9e5a09cf6052a2a144301fe111f5",
        ),
        (
            "android draft",
            include_bytes!("../test/data/signet_android_draft_signed.psbt"),
            "dc5ec07343f66445b4e02edb525f1c9edf7fcf5e474711c61bf3bc29855935db",
        ),
    ];

    #[test]
    fn signed_import_file_bytes_accept_binary_and_text_forms() {
        for (draft, signed_psbt, txid) in SIGNED_FIXTURES {
            let finalized_tx =
                Psbt::try_new(signed_psbt.to_vec()).unwrap().0.extract_tx_unchecked_fee_rate();

            let binary_tx = bitcoin::consensus::serialize(&finalized_tx);

            // apps pass the picked file's raw bytes, so text files arrive here with trailing newlines
            let files = [
                ("binary psbt", signed_psbt.to_vec()),
                ("binary transaction", binary_tx.clone()),
                ("base64 psbt", format!("{}\n", BASE64_STANDARD.encode(signed_psbt)).into()),
                ("hex psbt", format!("{}\n", hex::encode(signed_psbt)).into()),
                ("hex transaction", format!("{}\n", hex::encode(&binary_tx)).into()),
                (
                    "bom base64 psbt",
                    [UTF8_BOM, BASE64_STANDARD.encode(signed_psbt).as_bytes()].concat(),
                ),
            ];

            for (form, bytes) in files {
                let parsed = SignedTransactionOrPsbt::try_from_bytes(&bytes)
                    .unwrap_or_else(|error| panic!("{draft} {form}: {error}"));

                let is_psbt = matches!(parsed, SignedTransactionOrPsbt::SignedPsbt(_));
                assert_eq!(is_psbt, form.ends_with("psbt"), "{draft} {form}: {parsed:?}");
                assert_eq!(parsed.unsigned_tx_id().to_string(), txid, "{draft} {form}");
            }
        }

        // the base64 file Signer Lab wrote for the android draft, byte for byte
        let base64_file = include_bytes!("../test/data/signet_android_draft_signed.b64");
        let parsed = SignedTransactionOrPsbt::try_from_bytes(base64_file).unwrap();
        assert_eq!(parsed.psbt().unwrap().0.serialize(), SIGNED_FIXTURES[1].1);

        // the unsigned export of the same draft is rejected with the sign-first error
        let unsigned_file = include_bytes!("../test/data/signet_android_draft_unsigned.psbt");
        let result = SignedTransactionOrPsbt::try_from_bytes(unsigned_file);
        assert!(matches!(result, Err(SignedImportError::NotSigned)), "{result:?}");
    }

    #[test]
    fn signed_import_nfc_message_accepts_every_record_shape() {
        for (draft, signed_psbt, txid) in SIGNED_FIXTURES {
            let finalized_tx =
                Psbt::try_new(signed_psbt.to_vec()).unwrap().0.extract_tx_unchecked_fee_rate();

            let binary_tx = bitcoin::consensus::serialize(&finalized_tx);
            let base64_psbt = BASE64_STANDARD.encode(signed_psbt);
            let hex_tx = hex::encode(&binary_tx);

            // the platform NFC readers hand over a text record, a binary record, or both
            let messages = [
                ("text base64 psbt", NfcMessage::String(format!("{base64_psbt}\n")), true),
                ("text hex transaction", NfcMessage::String(hex_tx.clone()), false),
                ("binary psbt", NfcMessage::Data(signed_psbt.to_vec()), true),
                ("binary transaction", NfcMessage::Data(binary_tx.clone()), false),
                (
                    "binary record of base64 text",
                    NfcMessage::Data(base64_psbt.clone().into()),
                    true,
                ),
                (
                    "both forms of a psbt",
                    NfcMessage::Both(base64_psbt.clone(), signed_psbt.to_vec()),
                    true,
                ),
                ("both forms of a transaction", NfcMessage::Both(hex_tx.clone(), binary_tx), false),
                // a binary payload that doesn't parse falls back to the text record
                (
                    "text psbt beside unreadable data",
                    NfcMessage::Both(base64_psbt, vec![0xde, 0xad, 0xbe, 0xef]),
                    true,
                ),
                (
                    "text transaction beside unreadable data",
                    NfcMessage::Both(hex_tx, vec![0xde, 0xad, 0xbe, 0xef]),
                    false,
                ),
            ];

            for (form, message, is_psbt) in messages {
                let parsed = SignedTransactionOrPsbt::try_from_nfc_message(&message)
                    .unwrap_or_else(|error| panic!("{draft} {form}: {error}"));

                assert_eq!(
                    matches!(parsed, SignedTransactionOrPsbt::SignedPsbt(_)),
                    is_psbt,
                    "{draft} {form}: {parsed:?}"
                );
                assert_eq!(parsed.unsigned_tx_id().to_string(), txid, "{draft} {form}");
            }
        }

        let garbage = [
            NfcMessage::String("not a transaction".to_string()),
            NfcMessage::Data(vec![0xde, 0xad, 0xbe, 0xef]),
            NfcMessage::Both("not a transaction".to_string(), vec![0xde, 0xad, 0xbe, 0xef]),
        ];
        for message in garbage {
            let result = SignedTransactionOrPsbt::try_from_nfc_message(&message);
            assert!(result.is_err(), "{message:?} parsed as {result:?}");
        }

        // an unsigned binary psbt keeps the sign-first error instead of trying the text record
        let unsigned = include_bytes!("../test/data/signet_android_draft_unsigned.psbt");
        let signed_text = BASE64_STANDARD.encode(SIGNED_FIXTURES[1].1);
        for message in [
            NfcMessage::Data(unsigned.to_vec()),
            NfcMessage::String(BASE64_STANDARD.encode(unsigned)),
            NfcMessage::Both(signed_text, unsigned.to_vec()),
        ] {
            let result = SignedTransactionOrPsbt::try_from_nfc_message(&message);
            assert!(matches!(result, Err(SignedImportError::NotSigned)), "{result:?}");
        }
    }

    #[test]
    fn corrupt_psbt_text_keeps_its_parse_error() {
        let base64 = SignedTransactionOrPsbt::try_parse("cHNidP8BAHECAAAAAQ");
        assert!(
            matches!(
                base64,
                Err(SignedImportError::Base64DecodeError | SignedImportError::PsbtParseError(_))
            ),
            "{base64:?}"
        );

        let hex = SignedTransactionOrPsbt::try_parse("70736274ff0100");
        assert!(matches!(hex, Err(SignedImportError::PsbtParseError(_))), "{hex:?}");
    }

    #[test]
    fn malformed_psbt_errors_never_quote_the_payload() {
        use crate::test_support::malformed_psbt::MalformedPsbt;

        let expected = [PsbtDecodeError::PreimageHashMismatch, PsbtDecodeError::InvalidKey];

        for (fixture, expected) in MalformedPsbt::all().into_iter().zip(expected) {
            fixture.assert_parser_echoes_payload();

            let base64 = BASE64_STANDARD.encode(&fixture.bytes);
            let hex = hex::encode(&fixture.bytes);
            let errors = [
                SignedTransactionOrPsbt::try_from_bytes(&fixture.bytes).unwrap_err(),
                SignedTransactionOrPsbt::try_parse(&base64).unwrap_err(),
                SignedTransactionOrPsbt::try_parse(&hex).unwrap_err(),
            ];

            for error in errors {
                assert_eq!(error, SignedImportError::PsbtParseError(expected));
                fixture.assert_absent_from("error text", &error.to_string());
                fixture.assert_absent_from("error debug", &format!("{error:?}"));
            }
        }
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
