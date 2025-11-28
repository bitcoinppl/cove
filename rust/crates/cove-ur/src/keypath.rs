use bitcoin::bip32::ChildNumber;
use minicbor::{Decoder, Encoder, data::Tag};

use crate::{error::*, registry::CRYPTO_KEYPATH};

/// BIP32 hardened derivation flag (bit 31 set)
const HARDENED_FLAG: u32 = 0x8000_0000;

/// Mask to extract the index without the hardened flag
const INDEX_MASK: u32 = 0x7FFF_FFFF;

/// Check if a BIP32 path component is hardened
fn is_hardened(component: u32) -> bool {
    component & HARDENED_FLAG != 0
}

/// Extract the index from a BIP32 path component (strips hardened flag)
fn component_index(component: u32) -> u32 {
    component & INDEX_MASK
}

/// crypto-keypath: BIP32 derivation path
/// BCR-2020-007: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-007-hdkey.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoKeypath {
    /// Path components (with hardened bit set for hardened derivation)
    pub components: Vec<u32>,
    /// Source key fingerprint (optional, 4 bytes)
    pub source_fingerprint: Option<[u8; 4]>,
    /// Depth in the key tree (optional)
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
    /// CBOR structure: #6.304({1: [uint*], ?2: uint, ?3: uint})
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();
        let mut encoder = Encoder::new(&mut buffer);

        // write tag 304
        encoder
            .tag(Tag::new(CRYPTO_KEYPATH))
            .map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        // count fields: components (always present) + optional fields
        let field_count =
            1 + self.source_fingerprint.is_some() as usize + self.depth.is_some() as usize;

        // write map header
        encoder.map(field_count as u64).map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        // write components array (key 1)
        encoder.u32(1).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        encoder
            .array(self.components.len() as u64)
            .map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        for component in &self.components {
            encoder.u32(*component).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        }

        // write source_fingerprint if present (key 2)
        if let Some(fingerprint) = &self.source_fingerprint {
            encoder.u32(2).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
            encoder
                .u32(u32::from_be_bytes(*fingerprint))
                .map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        }

        // write depth if present (key 3)
        if let Some(depth) = self.depth {
            encoder.u32(3).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
            encoder.u32(depth).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        }

        Ok(buffer)
    }

    /// Decode from tagged CBOR
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);

        // read and verify tag 304
        let tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

        if tag != Tag::new(CRYPTO_KEYPATH) {
            return Err(UrError::InvalidTag { expected: CRYPTO_KEYPATH, actual: tag.as_u64() });
        }

        // read map
        let map_len = decoder
            .map()
            .map_err(|e| UrError::CborDecodeError(e.to_string()))?
            .ok_or_else(|| UrError::CborDecodeError("Expected definite-length map".to_string()))?;

        let mut components = None;
        let mut source_fingerprint = None;
        let mut depth = None;

        for _ in 0..map_len {
            let key = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

            match key {
                1 => {
                    // read components array
                    // BCR-2020-007 allows two formats:
                    // 1. Plain integers with hardened bit: [0x80000054, 0x80000000, ...]
                    // 2. Index/hardened pairs: [84, true, 0, true, ...]
                    let array_len = decoder
                        .array()
                        .map_err(|e| UrError::CborDecodeError(e.to_string()))?
                        .ok_or_else(|| {
                            UrError::CborDecodeError("Expected definite-length array".to_string())
                        })?;

                    let mut comp = Vec::new();
                    let mut i = 0;
                    while i < array_len {
                        // read the index value
                        let index =
                            decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                        i += 1;

                        // check if next element is a boolean (hardened flag)
                        if i < array_len {
                            // peek at next CBOR type
                            let next_type = decoder
                                .datatype()
                                .map_err(|e| UrError::CborDecodeError(e.to_string()))?;

                            if next_type == minicbor::data::Type::Bool {
                                // [index, hardened] pair format
                                let hardened = decoder
                                    .bool()
                                    .map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                                let component = if hardened { index | HARDENED_FLAG } else { index };
                                comp.push(component);
                                i += 1;
                            } else {
                                // plain integer format (hardened bit already in value)
                                comp.push(index);
                            }
                        } else {
                            // last element, just push as-is
                            comp.push(index);
                        }
                    }
                    components = Some(comp);
                }
                2 => {
                    let fp = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                    source_fingerprint = Some(fp.to_be_bytes());
                }
                3 => {
                    depth =
                        Some(decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?);
                }
                _ => {
                    return Err(UrError::InvalidField(format!("Unknown key in keypath: {}", key)));
                }
            }
        }

        let components =
            components.ok_or_else(|| UrError::MissingField("components".to_string()))?;

        Ok(Self { components, source_fingerprint, depth })
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
}
