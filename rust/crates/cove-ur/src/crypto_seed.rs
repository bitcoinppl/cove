//! crypto-seed: BIP39 seed with optional metadata
//! BCR-2020-006: <https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-006-urtypes.md>

use bip39::Mnemonic;
use minicbor::{Decode, Encode};

use crate::{
    error::{Result, ToUrError, UrError},
    registry::VALID_BIP39_ENTROPY_LENGTHS,
};

/// Internal CBOR representation with derive macros
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cbor(tag(300), map)]
struct CryptoSeedCbor {
    #[n(1)]
    #[cbor(with = "minicbor::bytes")]
    payload: Vec<u8>,

    #[n(2)]
    creation_date: Option<u64>,

    #[n(3)]
    name: Option<String>,

    #[n(4)]
    note: Option<String>,
}

/// crypto-seed: BIP39 seed with optional metadata
/// BCR-2020-006: <https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-006-urtypes.md>
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct CryptoSeed {
    /// Seed entropy bytes (16, 20, 24, 28, or 32 bytes for BIP39)
    pub payload: Vec<u8>,
    /// Creation timestamp (optional, CBOR date)
    pub creation_date: Option<u64>,
    /// Name/label (optional)
    pub name: Option<String>,
    /// Note/description (optional)
    pub note: Option<String>,
}

impl CryptoSeed {
    /// Create from entropy bytes
    #[must_use]
    pub const fn new(payload: Vec<u8>) -> Self {
        Self { payload, creation_date: None, name: None, note: None }
    }

    /// Create from BIP39 mnemonic
    #[must_use]
    pub fn from_mnemonic(mnemonic: &Mnemonic) -> Self {
        Self::new(mnemonic.to_entropy())
    }

    /// Create with all fields
    #[must_use]
    pub const fn with_metadata(
        payload: Vec<u8>,
        name: Option<String>,
        note: Option<String>,
        creation_date: Option<u64>,
    ) -> Self {
        Self { payload, creation_date, name, note }
    }

    /// Get the payload as BIP39 mnemonic
    ///
    /// # Errors
    /// Returns error if entropy is invalid for BIP39
    pub fn to_mnemonic(&self) -> Result<Mnemonic> {
        Mnemonic::from_entropy(&self.payload)
            .map_err(|e| UrError::InvalidField(format!("Invalid BIP39 entropy: {e}")))
    }

    /// Encode as tagged CBOR
    /// CBOR structure: #6.300({1: bytes, ?2: uint, ?3: text, ?4: text})
    ///
    /// # Errors
    /// Returns error if CBOR encoding fails
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let cbor = CryptoSeedCbor {
            payload: self.payload.clone(),
            creation_date: self.creation_date,
            name: self.name.clone(),
            note: self.note.clone(),
        };
        minicbor::to_vec(&cbor).map_err(|e| UrError::CborEncodeError(e.to_string()))
    }

    /// Decode from tagged CBOR
    ///
    /// # Errors
    /// Returns error if CBOR decoding fails or payload length is invalid
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let decoded: CryptoSeedCbor = minicbor::decode(cbor).map_err_cbor_decode()?;

        // validate payload is not empty (required field)
        if decoded.payload.is_empty() {
            return Err(UrError::MissingField("payload".to_string()));
        }

        // validate payload length for BIP39 (128-256 bits in 32-bit increments)
        if !VALID_BIP39_ENTROPY_LENGTHS.contains(&decoded.payload.len()) {
            return Err(UrError::InvalidPayloadLength(format!(
                "Expected 16, 20, 24, 28, or 32 bytes, got {}",
                decoded.payload.len()
            )));
        }

        Ok(Self {
            payload: decoded.payload,
            creation_date: decoded.creation_date,
            name: decoded.name,
            note: decoded.note,
        })
    }
}

#[uniffi::export]
impl CryptoSeed {
    /// Create from entropy bytes with optional metadata
    ///
    /// # Errors
    /// Returns error if entropy is invalid for BIP39
    #[uniffi::constructor]
    pub fn from_entropy_with_metadata(
        payload: Vec<u8>,
        name: Option<String>,
        note: Option<String>,
        creation_date: Option<u64>,
    ) -> Result<Self> {
        let seed = Self::with_metadata(payload, name, note, creation_date);
        // validate by trying to create mnemonic
        let _ = seed.to_mnemonic()?;
        Ok(seed)
    }

    /// Create from entropy bytes
    ///
    /// # Errors
    /// Returns error if entropy is invalid for BIP39
    #[uniffi::constructor]
    pub fn from_entropy(payload: Vec<u8>) -> Result<Self> {
        Self::from_entropy_with_metadata(payload, None, None, None)
    }

    /// Get entropy bytes
    pub fn entropy(&self) -> Vec<u8> {
        self.payload.clone()
    }

    /// Get name if present
    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }

    /// Get note if present
    pub fn get_note(&self) -> Option<String> {
        self.note.clone()
    }

    /// Get creation date if present
    pub const fn get_creation_date(&self) -> Option<u64> {
        self.creation_date
    }

    /// Encode as CBOR for UR
    ///
    /// # Errors
    /// Returns error if CBOR encoding fails
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.to_cbor()
    }

    /// Decode from CBOR
    ///
    /// # Errors
    /// Returns error if CBOR decoding fails or payload length is invalid
    #[uniffi::constructor]
    #[allow(clippy::needless_pass_by_value)]
    pub fn decode(cbor: Vec<u8>) -> Result<Self> {
        Self::from_cbor(&cbor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_crypto_seed_new() {
        // 16-byte entropy (128 bits = 12 word mnemonic)
        let entropy = vec![0x12; 16];
        let seed = CryptoSeed::new(entropy.clone());

        assert_eq!(seed.payload, entropy);
        assert!(seed.creation_date.is_none());
        assert!(seed.name.is_none());
        assert!(seed.note.is_none());
    }

    #[test]
    fn test_crypto_seed_from_mnemonic() {
        let mnemonic_str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_str(mnemonic_str).unwrap();

        let seed = CryptoSeed::from_mnemonic(&mnemonic);
        let recovered_mnemonic = seed.to_mnemonic().unwrap();

        assert_eq!(recovered_mnemonic.to_string(), mnemonic.to_string());
    }

    #[test]
    fn test_crypto_seed_cbor_roundtrip() {
        let entropy = vec![0xAB; 16];
        let seed = CryptoSeed::new(entropy.clone());

        let cbor = seed.to_cbor().unwrap();
        let decoded = CryptoSeed::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.payload, entropy);
    }

    #[test]
    fn test_crypto_seed_with_metadata() {
        let entropy = vec![0xCD; 16];
        let seed = CryptoSeed::with_metadata(
            entropy.clone(),
            Some("Test Wallet".to_string()),
            Some("Testing metadata".to_string()),
            Some(1234567890),
        );

        let cbor = seed.to_cbor().unwrap();
        let decoded = CryptoSeed::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.payload, entropy);
        assert_eq!(decoded.name, Some("Test Wallet".to_string()));
        assert_eq!(decoded.note, Some("Testing metadata".to_string()));
        assert_eq!(decoded.creation_date, Some(1234567890));
    }

    #[test]
    fn test_crypto_seed_invalid_length() {
        // 15 bytes is invalid for BIP39
        let entropy = vec![0xFF; 15];
        let seed = CryptoSeed::new(entropy);

        let cbor = seed.to_cbor().unwrap();
        let result = CryptoSeed::from_cbor(&cbor);

        assert!(result.is_err());
        match result {
            Err(UrError::InvalidPayloadLength(_)) => {}
            _ => panic!("Expected InvalidPayloadLength error"),
        }
    }

    #[test]
    fn test_crypto_seed_cbor_has_tag() {
        let entropy = vec![0xEF; 16];
        let seed = CryptoSeed::new(entropy);

        let cbor = seed.to_cbor().unwrap();

        // verify CBOR starts with tag 300
        // CBOR tag 300 = 0xD9 0x01 0x2C
        assert_eq!(cbor[0], 0xD9); // major type 6 (tag), additional info 25 (2-byte uint16)
        assert_eq!(cbor[1], 0x01);
        assert_eq!(cbor[2], 0x2C); // 0x012C = 300
    }

    /// Test malformed CBOR: wrong tag
    #[test]
    fn test_crypto_seed_wrong_tag() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // use wrong tag (310 instead of 300)
        encoder.tag(Tag::new(310)).unwrap();
        encoder.map(1).unwrap();
        encoder.u32(1).unwrap();
        encoder.bytes(&[0xAB; 16]).unwrap();

        let result = CryptoSeed::from_cbor(&cbor);
        assert!(result.is_err());
        // derive macro returns generic decode error, not specific tag error
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }

    /// Test malformed CBOR: missing required field
    #[test]
    fn test_crypto_seed_missing_payload() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // correct tag but missing payload field
        encoder.tag(Tag::new(300)).unwrap();
        encoder.map(1).unwrap();
        encoder.u32(2).unwrap(); // field 2 (creation_date) instead of 1 (payload)
        encoder.u64(12345).unwrap();

        let result = CryptoSeed::from_cbor(&cbor);
        assert!(result.is_err());
        // minicbor derive catches missing required fields at decode time
        match result.unwrap_err() {
            UrError::CborDecodeError(msg) => {
                assert!(
                    msg.contains("missing value") || msg.contains("payload"),
                    "Error should mention missing payload: {}",
                    msg
                );
            }
            UrError::MissingField(_) | UrError::InvalidPayloadLength(_) => {
                // also acceptable
            }
            e => panic!(
                "Expected CborDecodeError, MissingField, or InvalidPayloadLength error, got: {:?}",
                e
            ),
        }
    }

    /// Test malformed CBOR: corrupted CBOR structure
    #[test]
    fn test_crypto_seed_corrupted_cbor() {
        // completely invalid CBOR
        let invalid_cbor = vec![0xFF, 0xFF, 0xFF];
        let result = CryptoSeed::from_cbor(&invalid_cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }

    /// Test malformed CBOR: truncated data
    #[test]
    fn test_crypto_seed_truncated_cbor() {
        let entropy = vec![0xCD; 16];
        let seed = CryptoSeed::new(entropy);
        let cbor = seed.to_cbor().unwrap();

        // truncate the CBOR data
        let truncated = &cbor[..cbor.len() - 5];
        let result = CryptoSeed::from_cbor(truncated);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }

    /// Test forward compatibility: unknown keys should be ignored
    #[test]
    fn test_crypto_seed_ignores_unknown_keys() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // create valid crypto-seed with an unknown key (99)
        encoder.tag(Tag::new(300)).unwrap();
        encoder.map(2).unwrap();
        encoder.u32(1).unwrap();
        encoder.bytes(&[0xAB; 16]).unwrap(); // payload
        encoder.u32(99).unwrap(); // unknown key
        encoder.str("future field").unwrap();

        let result = CryptoSeed::from_cbor(&cbor);
        assert!(result.is_ok(), "Should ignore unknown key 99");
        assert_eq!(result.unwrap().payload, vec![0xAB; 16]);
    }
}
