//! crypto-keypath: BIP32 derivation path
//! BCR-2020-007: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-007-hdkey.md

use bitcoin::bip32::ChildNumber;
use minicbor::{Decode, Encode};

use crate::error::*;

/// BIP32 hardened derivation flag (bit 31 set)
pub const HARDENED_FLAG: u32 = 0x8000_0000;

/// Mask to extract the index without the hardened flag
pub const INDEX_MASK: u32 = 0x7FFF_FFFF;

/// Check if a BIP32 path component is hardened
pub fn is_hardened(component: u32) -> bool {
    component & HARDENED_FLAG != 0
}

/// Extract the index from a BIP32 path component (strips hardened flag)
pub fn component_index(component: u32) -> u32 {
    component & INDEX_MASK
}

/// crypto-keypath: BIP32 derivation path
/// CBOR structure: #6.304({1: [uint*], ?2: uint, ?3: uint})
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cbor(tag(304), map)]
pub struct CryptoKeypath {
    /// Path components (with hardened bit set for hardened derivation)
    #[n(1)]
    #[cbor(with = "crate::cbor::keypath_components")]
    pub components: Vec<u32>,

    /// Source key fingerprint (optional, 4 bytes encoded as u32)
    #[n(2)]
    #[cbor(with = "crate::cbor::fingerprint")]
    pub source_fingerprint: Option<[u8; 4]>,

    /// Depth in the key tree (optional)
    #[n(3)]
    pub depth: Option<u32>,
}

impl CryptoKeypath {
    /// Create new CryptoKeypath
    pub fn new(
        components: Vec<u32>,
        source_fingerprint: Option<[u8; 4]>,
        depth: Option<u32>,
    ) -> Self {
        Self { components, source_fingerprint, depth }
    }

    /// Get the last path component as a ChildNumber (for xpub construction)
    pub fn last_child_number(&self) -> ChildNumber {
        self.components
            .last()
            .and_then(|&component| {
                let index = component_index(component);
                if is_hardened(component) {
                    ChildNumber::from_hardened_idx(index).ok()
                } else {
                    ChildNumber::from_normal_idx(index).ok()
                }
            })
            .unwrap_or(ChildNumber::from(0))
    }

    /// Convert components to derivation path string (e.g., "84h/0h/0h")
    pub fn to_path_string(&self) -> String {
        self.components
            .iter()
            .map(|&component| {
                if is_hardened(component) {
                    format!("{}h", component_index(component))
                } else {
                    format!("{component}")
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Encode as tagged CBOR
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        minicbor::to_vec(self).map_err(|e| UrError::CborEncodeError(e.to_string()))
    }

    /// Decode from tagged CBOR
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        minicbor::decode(cbor).map_err_cbor_decode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypath_simple() {
        // m/44'/0'/0'/0/0
        let components = vec![
            0x80000000 + 44, // 44' (hardened)
            0x80000000 + 0,  // 0' (hardened)
            0x80000000 + 0,  // 0' (hardened)
            0,               // 0
            0,               // 0
        ];

        let keypath = CryptoKeypath::new(components.clone(), None, None);

        let cbor = keypath.to_cbor().unwrap();
        let decoded = CryptoKeypath::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.components, components);
    }

    #[test]
    fn test_keypath_with_fingerprint() {
        let components = vec![0x80000000 + 44];
        let fingerprint = [0x12, 0x34, 0x56, 0x78];

        let keypath = CryptoKeypath::new(components.clone(), Some(fingerprint), Some(1));

        let cbor = keypath.to_cbor().unwrap();
        let decoded = CryptoKeypath::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.components, components);
        assert_eq!(decoded.source_fingerprint, Some(fingerprint));
        assert_eq!(decoded.depth, Some(1));
    }

    #[test]
    fn test_to_path_string() {
        // m/84'/0'/0'
        let components = vec![
            0x80000000 + 84, // 84' (hardened)
            0x80000000 + 0,  // 0' (hardened)
            0x80000000 + 0,  // 0' (hardened)
        ];
        let keypath = CryptoKeypath::new(components, None, None);
        assert_eq!(keypath.to_path_string(), "84h/0h/0h");

        // m/44'/0'/0'/0/0 (mixed hardened and non-hardened)
        let components = vec![
            0x80000000 + 44, // 44' (hardened)
            0x80000000 + 0,  // 0' (hardened)
            0x80000000 + 0,  // 0' (hardened)
            0,               // 0
            0,               // 0
        ];
        let keypath = CryptoKeypath::new(components, None, None);
        assert_eq!(keypath.to_path_string(), "44h/0h/0h/0/0");
    }

    /// Test decoding index/hardened pair format (format 2)
    /// BCR-2020-007 allows: [84, true, 0, true, 0, true]
    #[test]
    fn test_keypath_pair_format_decode() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // encode as index/hardened pairs: m/84'/0'/0'
        encoder.tag(Tag::new(304)).unwrap();
        encoder.map(1).unwrap();
        encoder.u32(1).unwrap();
        encoder.array(6).unwrap(); // 3 components * 2 elements each
        encoder.u32(84).unwrap();
        encoder.bool(true).unwrap(); // 84'
        encoder.u32(0).unwrap();
        encoder.bool(true).unwrap(); // 0'
        encoder.u32(0).unwrap();
        encoder.bool(true).unwrap(); // 0'

        let decoded = CryptoKeypath::from_cbor(&cbor).unwrap();

        // should decode to same result as integer format
        assert_eq!(decoded.components, vec![0x80000054, 0x80000000, 0x80000000]);
        assert_eq!(decoded.to_path_string(), "84h/0h/0h");
    }

    /// Test that we always encode in plain integer format (format 1)
    #[test]
    fn test_keypath_encodes_integer_format() {
        let keypath = CryptoKeypath::new(vec![0x80000054, 0x80000000, 0x80000000], None, None);

        let cbor = keypath.to_cbor().unwrap();

        // decode manually to verify format
        let mut decoder = minicbor::Decoder::new(&cbor);
        decoder.tag().unwrap(); // skip tag
        decoder.map().unwrap(); // skip map header
        decoder.u32().unwrap(); // skip key 1

        let arr_len = decoder.array().unwrap().unwrap();
        assert_eq!(arr_len, 3); // 3 integers, not 6 for pairs

        // first element should be the integer with hardened bit
        let first = decoder.u32().unwrap();
        assert_eq!(first, 0x80000054);
    }

    /// Test forward compatibility: unknown keys should be ignored
    #[test]
    fn test_keypath_ignores_unknown_keys() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // create valid crypto-keypath with an unknown key (99)
        encoder.tag(Tag::new(304)).unwrap();
        encoder.map(2).unwrap();
        encoder.u32(1).unwrap();
        encoder.array(1).unwrap();
        encoder.u32(0x80000054).unwrap(); // 84'
        encoder.u32(99).unwrap(); // unknown key
        encoder.str("future field").unwrap();

        let result = CryptoKeypath::from_cbor(&cbor);
        assert!(result.is_ok(), "Should ignore unknown key 99");
        assert_eq!(result.unwrap().components, vec![0x80000054]);
    }
}
