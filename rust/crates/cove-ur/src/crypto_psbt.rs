use bitcoin::psbt::Psbt as BdkPsbt;
use foundation_ur::{UR, bytewords};
use minicbor::{Decoder, Encoder, data::Tag};

use crate::{error::*, registry::CRYPTO_PSBT};

/// crypto-psbt: PSBT encoded as CBOR byte string with tag 310
/// BCR-2020-006: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-006-urtypes.md
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct CryptoPsbt {
    psbt: BdkPsbt,
}

impl CryptoPsbt {
    /// Create from bitcoin PSBT
    pub fn new(psbt: BdkPsbt) -> Self {
        Self { psbt }
    }

    /// Create from PSBT bytes
    pub fn from_bytes(psbt_bytes: &[u8]) -> Result<Self> {
        let psbt = BdkPsbt::deserialize(psbt_bytes)
            .map_err(|e| UrError::CborDecodeError(format!("Invalid PSBT: {}", e)))?;
        Ok(Self { psbt })
    }

    /// Get the underlying PSBT
    pub fn psbt(&self) -> &BdkPsbt {
        &self.psbt
    }

    /// Get PSBT as bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        self.psbt.serialize()
    }

    /// Encode as tagged CBOR bytes
    /// CBOR structure: #6.310(bytes)
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let psbt_bytes = self.psbt.serialize();

        let mut buffer = Vec::new();
        let mut encoder = Encoder::new(&mut buffer);

        // write tag 310
        encoder.tag(Tag::new(CRYPTO_PSBT)).map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        // write PSBT as byte string
        encoder.bytes(&psbt_bytes).map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        Ok(buffer)
    }

    /// Decode from tagged CBOR bytes
    /// Supports both tagged (#6.310) and untagged CBOR for interoperability
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);

        // try to read tag - if present, verify it's 310
        let psbt_bytes = match decoder.tag() {
            Ok(tag) => {
                // tagged format: verify tag 310
                if tag != Tag::new(CRYPTO_PSBT) {
                    return Err(UrError::InvalidTag {
                        expected: CRYPTO_PSBT,
                        actual: tag.as_u64(),
                    });
                }

                // read byte string after tag
                decoder.bytes().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_vec()
            }
            Err(_) => {
                // untagged format: try to read as byte string directly
                // reset decoder to start
                decoder = Decoder::new(cbor);
                decoder.bytes().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_vec()
            }
        };

        // deserialize PSBT
        let psbt = BdkPsbt::deserialize(&psbt_bytes)
            .map_err(|e| UrError::CborDecodeError(format!("Invalid PSBT: {}", e)))?;

        Ok(Self { psbt })
    }

    /// Encode as UR string (for single-part UR)
    pub fn to_ur_string(&self) -> Result<String> {
        let cbor = self.to_cbor()?;
        let ur = UR::new("crypto-psbt", &cbor);
        Ok(ur.to_string())
    }

    /// Decode from UR string
    pub fn from_ur_string(ur: &str) -> Result<Self> {
        let ur = UR::parse(ur).map_err(|e| UrError::UrParseError(e.to_string()))?;

        // verify UR type
        if ur.as_type() != "crypto-psbt" {
            return Err(UrError::InvalidField(format!(
                "Expected crypto-psbt, got {}",
                ur.as_type()
            )));
        }

        // extract bytewords string from single-part UR and decode to bytes
        let cbor = match ur {
            UR::SinglePart { message, .. } => {
                // decode bytewords to bytes (UR uses minimal style)
                bytewords::decode(message, bytewords::Style::Minimal)
                    .map_err(|e| UrError::UrParseError(format!("Bytewords decode error: {}", e)))?
            }
            UR::SinglePartDeserialized { message, .. } => {
                // already deserialized
                message.to_vec()
            }
            _ => {
                return Err(UrError::InvalidField(
                    "Expected single-part UR, got multi-part".to_string(),
                ));
            }
        };

        Self::from_cbor(&cbor)
    }
}

#[uniffi::export]
impl CryptoPsbt {
    /// Create from PSBT bytes
    #[uniffi::constructor]
    pub fn from_psbt_bytes(psbt_bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes(&psbt_bytes)
    }

    /// Get PSBT as bytes
    pub fn to_psbt_bytes(&self) -> Vec<u8> {
        self.to_bytes()
    }

    /// Encode as CBOR for UR
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.to_cbor()
    }

    /// Decode from CBOR
    #[uniffi::constructor]
    pub fn decode(cbor: Vec<u8>) -> Result<Self> {
        Self::from_cbor(&cbor)
    }

    /// Encode as UR string
    pub fn to_ur(&self) -> Result<String> {
        self.to_ur_string()
    }

    /// Decode from UR string
    #[uniffi::constructor]
    pub fn from_ur(ur: String) -> Result<Self> {
        Self::from_ur_string(&ur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test PSBT from BDK example
    const TEST_PSBT_HEX: &str = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";

    fn test_psbt() -> BdkPsbt {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        BdkPsbt::deserialize(&bytes).unwrap()
    }

    #[test]
    fn test_crypto_psbt_new() {
        let psbt = test_psbt();
        let crypto_psbt = CryptoPsbt::new(psbt.clone());
        assert_eq!(crypto_psbt.psbt(), &psbt);
    }

    #[test]
    fn test_crypto_psbt_from_bytes() {
        let bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let crypto_psbt = CryptoPsbt::from_bytes(&bytes).unwrap();
        assert_eq!(crypto_psbt.to_bytes(), bytes);
    }

    #[test]
    fn test_crypto_psbt_cbor_roundtrip() {
        let psbt = test_psbt();
        let crypto_psbt = CryptoPsbt::new(psbt.clone());

        // encode to CBOR
        let cbor = crypto_psbt.to_cbor().unwrap();

        // decode from CBOR
        let decoded = CryptoPsbt::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.psbt(), &psbt);
    }

    #[test]
    fn test_crypto_psbt_cbor_has_tag() {
        let psbt = test_psbt();
        let crypto_psbt = CryptoPsbt::new(psbt);

        let cbor = crypto_psbt.to_cbor().unwrap();

        // verify CBOR starts with tag 310
        // CBOR tag 310 = 0xD9 0x01 0x36
        assert_eq!(cbor[0], 0xD9); // major type 6 (tag), additional info 25 (2-byte uint16)
        assert_eq!(cbor[1], 0x01);
        assert_eq!(cbor[2], 0x36); // 0x0136 = 310
    }

    #[test]
    fn test_crypto_psbt_untagged_cbor() {
        let psbt = test_psbt();
        let psbt_bytes = psbt.serialize();

        // create untagged CBOR (just the byte string, no tag 310)
        let mut untagged_cbor = Vec::new();
        let mut encoder = Encoder::new(&mut untagged_cbor);
        encoder.bytes(&psbt_bytes).unwrap();

        // should still decode successfully
        let decoded = CryptoPsbt::from_cbor(&untagged_cbor).unwrap();
        assert_eq!(decoded.psbt(), &psbt);
    }

    #[test]
    fn test_crypto_psbt_ur_string_roundtrip() {
        let psbt = test_psbt();
        let crypto_psbt = CryptoPsbt::new(psbt.clone());

        // encode to UR string
        let ur_string = crypto_psbt.to_ur_string().unwrap();

        // should start with "ur:crypto-psbt/"
        assert!(ur_string.starts_with("ur:crypto-psbt/"));

        // decode from UR string
        let decoded = CryptoPsbt::from_ur_string(&ur_string).unwrap();

        assert_eq!(decoded.psbt(), &psbt);
    }

    #[test]
    fn test_crypto_psbt_wrong_ur_type() {
        // try to decode a UR with wrong type
        let result = CryptoPsbt::from_ur_string("ur:crypto-seed/test");
        assert!(result.is_err());
    }

    /// Test with real Jade hardware wallet UR:CRYPTO-PSBT vector
    /// Source: Jade QR code research documentation
    #[test]
    fn test_crypto_psbt_jade_vector() {
        // real UR:CRYPTO-PSBT from Jade hardware wallet
        const JADE_UR: &str = "UR:CRYPTO-PSBT/HDRKJOJKIDJYZMADAEGMAOAEAEAEADHEHKDKKPZCNETTHHMYVWDNMNADWZSPBBSFSWCADNCWFWQDKSIAZTGHTIJPCFNSZTAEAEAEAEAEZMZMZMZMADRFAOAEAEAEAEAEAECMAEBBKSHGTECWKIEMMKFYVSZMVWFNWYNSZECNWLUYLNTTAEAEAEAEAEADADCTKNAXAEAEAEAEAEAECMAEBBWLLFESBGNYAXMYGOCHKKCFVYKNHDRHIOIYOSOLYNCPAMAXLEOLSRPRFEINGEPYDSMYBEVECFGSVOWZBKAMZOSRAXWLRLHTDPSOLOPYWMGOMSGLCSLGISMNNBGHAEAELAAEAEAELAAEAEAELAAEAEAEAEAOAEAEAEAEAEBYBTDEIE";

        // decode the UR string (case-insensitive)
        let crypto_psbt = CryptoPsbt::from_ur_string(&JADE_UR.to_lowercase()).unwrap();

        // get the PSBT bytes
        let psbt_bytes = crypto_psbt.to_bytes();

        // verify PSBT magic bytes: 0x70736274ff ("psbt" + 0xff)
        assert!(psbt_bytes.len() >= 5, "PSBT too short");
        assert_eq!(&psbt_bytes[0..5], &[0x70, 0x73, 0x62, 0x74, 0xff], "Invalid PSBT magic bytes");

        // roundtrip: encode back to UR and verify it decodes to same PSBT
        let ur_string = crypto_psbt.to_ur_string().unwrap();
        let decoded = CryptoPsbt::from_ur_string(&ur_string).unwrap();
        assert_eq!(decoded.to_bytes(), psbt_bytes);
    }

    /// Test malformed CBOR: wrong tag
    #[test]
    fn test_crypto_psbt_wrong_tag() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // use wrong tag (300 instead of 310)
        encoder.tag(Tag::new(300)).unwrap();
        encoder.bytes(&[0x70, 0x73, 0x62, 0x74, 0xff]).unwrap();

        let result = CryptoPsbt::from_cbor(&cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::InvalidTag { expected: 310, actual: 300 }));
    }

    /// Test malformed CBOR: invalid PSBT data
    #[test]
    fn test_crypto_psbt_invalid_psbt_data() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // correct tag but invalid PSBT bytes (missing magic)
        encoder.tag(Tag::new(310)).unwrap();
        encoder.bytes(&[0x00, 0x00, 0x00]).unwrap();

        let result = CryptoPsbt::from_cbor(&cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }

    /// Test malformed CBOR: corrupted structure
    #[test]
    fn test_crypto_psbt_corrupted_cbor() {
        // completely invalid CBOR
        let invalid_cbor = vec![0xFF, 0xFF, 0xFF];
        let result = CryptoPsbt::from_cbor(&invalid_cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }

    /// Test malformed CBOR: truncated data
    #[test]
    fn test_crypto_psbt_truncated_cbor() {
        let psbt = test_psbt();
        let crypto_psbt = CryptoPsbt::new(psbt);
        let cbor = crypto_psbt.to_cbor().unwrap();

        // truncate the CBOR data
        let truncated = &cbor[..cbor.len() - 10];
        let result = CryptoPsbt::from_cbor(truncated);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }
}
