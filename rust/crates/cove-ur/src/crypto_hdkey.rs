//! crypto-hdkey: Hierarchical Deterministic Key (BIP32)
//! BCR-2020-007: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-007-hdkey.md
//!
//! Note: This type uses manual CBOR encoding/decoding (not derive macros) because:
//! 1. Fields 5-7 (use_info, origin, children) contain embedded tagged CBOR structures
//! 2. These nested types must be pre-encoded with their own tags (305, 304)
//! 3. The raw CBOR bytes are appended directly to the buffer
//! 4. Decoding requires position tracking and recursive parsing
//! 5. Derive macros cannot express this embedded CBOR pattern cleanly

use bitcoin::bip32::{Xpriv, Xpub};
use minicbor::{Decoder, Encoder, data::Tag};

use crate::{
    coin_info::CryptoCoinInfo,
    error::*,
    keypath::CryptoKeypath,
    registry::{CRYPTO_HDKEY, hdkey_keys::*, lengths},
};

/// crypto-hdkey: Hierarchical Deterministic Key (BIP32)
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct CryptoHdkey {
    /// True if this is a master key
    pub is_master: bool,
    /// True if this contains a private key
    pub is_private: bool,
    /// Key data: 33 bytes for public key, 32 bytes for private key
    pub key_data: Vec<u8>,
    /// Chain code (32 bytes, optional)
    pub chain_code: Option<Vec<u8>>,
    /// Coin info (optional)
    pub use_info: Option<CryptoCoinInfo>,
    /// Origin path (optional)
    pub origin: Option<CryptoKeypath>,
    /// Children path (optional)
    pub children: Option<CryptoKeypath>,
    /// Parent key fingerprint (4 bytes, optional)
    pub parent_fingerprint: Option<[u8; 4]>,
    /// Name (optional)
    pub name: Option<String>,
    /// Source (optional)
    pub source: Option<String>,
}

impl CryptoHdkey {
    /// Create from extended public key (xpub)
    pub fn from_xpub(xpub: &Xpub) -> Self {
        let key_data = xpub.public_key.serialize().to_vec();
        let chain_code = Some(xpub.chain_code.to_bytes().to_vec());
        let parent_fingerprint = if xpub.parent_fingerprint.to_bytes() != [0, 0, 0, 0] {
            Some(xpub.parent_fingerprint.to_bytes())
        } else {
            None
        };

        Self {
            is_master: xpub.depth == 0,
            is_private: false,
            key_data,
            chain_code,
            use_info: None,
            origin: None,
            children: None,
            parent_fingerprint,
            name: None,
            source: None,
        }
    }

    /// Create from extended private key (xpriv)
    pub fn from_xpriv(xpriv: &Xpriv) -> Self {
        let key_data = xpriv.private_key.secret_bytes().to_vec();
        let chain_code = Some(xpriv.chain_code.to_bytes().to_vec());
        let parent_fingerprint = if xpriv.parent_fingerprint.to_bytes() != [0, 0, 0, 0] {
            Some(xpriv.parent_fingerprint.to_bytes())
        } else {
            None
        };

        Self {
            is_master: xpriv.depth == 0,
            is_private: true,
            key_data,
            chain_code,
            use_info: None,
            origin: None,
            children: None,
            parent_fingerprint,
            name: None,
            source: None,
        }
    }

    /// Encode as tagged CBOR
    /// CBOR structure: #6.303({1: bool, 2: bool, 3: bytes, ?4: bytes, ...})
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        // pre-encode embedded structures (they contain their own CBOR tags)
        let use_info_cbor = self.use_info.as_ref().map(|u| u.to_cbor()).transpose()?;
        let origin_cbor = self.origin.as_ref().map(|o| o.to_cbor()).transpose()?;
        let children_cbor = self.children.as_ref().map(|c| c.to_cbor()).transpose()?;

        let mut buffer = Vec::new();

        // count fields
        let field_count = 3 // is_master, is_private, key_data always present
            + self.chain_code.is_some() as u64
            + self.use_info.is_some() as u64
            + self.origin.is_some() as u64
            + self.children.is_some() as u64
            + self.parent_fingerprint.is_some() as u64
            + self.name.is_some() as u64
            + self.source.is_some() as u64;

        // write tag, map header, and fields 1-4 with encoder
        {
            let mut encoder = Encoder::new(&mut buffer);
            encoder.tag(Tag::new(CRYPTO_HDKEY)).map_err_cbor_encode()?;
            encoder.map(field_count).map_err_cbor_encode()?;

            // is_master (key 1)
            encoder.u32(IS_MASTER).map_err_cbor_encode()?;
            encoder.bool(self.is_master).map_err_cbor_encode()?;

            // is_private (key 2)
            encoder.u32(IS_PRIVATE).map_err_cbor_encode()?;
            encoder.bool(self.is_private).map_err_cbor_encode()?;

            // key_data (key 3)
            encoder.u32(KEY_DATA).map_err_cbor_encode()?;
            encoder.bytes(&self.key_data).map_err_cbor_encode()?;

            // chain_code if present (key 4)
            if let Some(chain_code) = &self.chain_code {
                encoder.u32(CHAIN_CODE).map_err_cbor_encode()?;
                encoder.bytes(chain_code).map_err_cbor_encode()?;
            }
        } // encoder borrow ends here

        // fields 5-7 contain pre-encoded CBOR with their own tags, so we append
        // the map key followed by raw CBOR bytes directly to the buffer
        if let Some(cbor) = use_info_cbor {
            buffer.push(USE_INFO as u8); // CBOR uint for small values
            buffer.extend_from_slice(&cbor);
        }
        if let Some(cbor) = origin_cbor {
            buffer.push(ORIGIN as u8); // CBOR uint for small values
            buffer.extend_from_slice(&cbor);
        }
        if let Some(cbor) = children_cbor {
            buffer.push(CHILDREN as u8); // CBOR uint for small values
            buffer.extend_from_slice(&cbor);
        }

        // remaining fields are simple types, use encoder
        {
            let mut encoder = Encoder::new(&mut buffer);

            if let Some(fingerprint) = &self.parent_fingerprint {
                encoder.u32(PARENT_FINGERPRINT).map_err_cbor_encode()?;
                encoder.u32(u32::from_be_bytes(*fingerprint)).map_err_cbor_encode()?;
            }
            if let Some(name) = &self.name {
                encoder.u32(NAME).map_err_cbor_encode()?;
                encoder.str(name).map_err_cbor_encode()?;
            }
            if let Some(source) = &self.source {
                encoder.u32(SOURCE).map_err_cbor_encode()?;
                encoder.str(source).map_err_cbor_encode()?;
            }
        }

        Ok(buffer)
    }

    /// Decode from tagged CBOR
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);

        // read and verify tag 303
        let tag = decoder.tag().map_err_cbor_decode()?;

        if tag != Tag::new(CRYPTO_HDKEY) {
            return Err(UrError::InvalidTag { expected: CRYPTO_HDKEY, actual: tag.as_u64() });
        }

        // read map
        let map_len = decoder
            .map()
            .map_err_cbor_decode()?
            .ok_or_else(|| UrError::CborDecodeError("Expected definite-length map".to_string()))?;

        let mut is_master = None;
        let mut is_private = None;
        let mut key_data = None;
        let mut chain_code = None;
        let mut use_info = None;
        let mut origin = None;
        let mut children = None;
        let mut parent_fingerprint = None;
        let mut name = None;
        let mut source = None;

        for _ in 0..map_len {
            let key = decoder.u32().map_err_cbor_decode()?;

            match key {
                IS_MASTER => {
                    is_master = Some(decoder.bool().map_err_cbor_decode()?);
                }
                IS_PRIVATE => {
                    is_private = Some(decoder.bool().map_err_cbor_decode()?);
                }
                KEY_DATA => {
                    key_data = Some(decoder.bytes().map_err_cbor_decode()?.to_vec());
                }
                CHAIN_CODE => {
                    chain_code = Some(decoder.bytes().map_err_cbor_decode()?.to_vec());
                }
                USE_INFO => {
                    // read embedded tagged CBOR for use_info
                    let pos = decoder.position();
                    use_info = Some(CryptoCoinInfo::from_cbor(&cbor[pos..])?);
                    // skip over the embedded structure
                    decoder.skip().map_err_cbor_decode()?;
                }
                ORIGIN => {
                    // read embedded tagged CBOR for origin
                    let pos = decoder.position();
                    origin = Some(CryptoKeypath::from_cbor(&cbor[pos..])?);
                    // skip over the embedded structure
                    decoder.skip().map_err_cbor_decode()?;
                }
                CHILDREN => {
                    // read embedded tagged CBOR for children
                    let pos = decoder.position();
                    children = Some(CryptoKeypath::from_cbor(&cbor[pos..])?);
                    // skip over the embedded structure
                    decoder.skip().map_err_cbor_decode()?;
                }
                PARENT_FINGERPRINT => {
                    let fp = decoder.u32().map_err_cbor_decode()?;
                    parent_fingerprint = Some(fp.to_be_bytes());
                }
                NAME => {
                    name = Some(decoder.str().map_err_cbor_decode()?.to_string());
                }
                SOURCE => {
                    source = Some(decoder.str().map_err_cbor_decode()?.to_string());
                }
                _ => {
                    // skip unknown fields for forward compatibility
                    decoder.skip().map_err_cbor_decode()?;
                }
            }
        }

        // is_master and is_private default to false if not present (BCR-2020-007)
        let is_master = is_master.unwrap_or(false);
        let is_private = is_private.unwrap_or(false);
        let key_data = key_data.ok_or_else(|| UrError::MissingField("key_data".to_string()))?;

        // validate key data length
        let expected_len =
            if is_private { lengths::PRIVATE_KEY } else { lengths::COMPRESSED_PUBKEY };
        if key_data.len() != expected_len {
            return Err(UrError::InvalidKeyDataLength {
                expected: expected_len as u64,
                actual: key_data.len() as u64,
            });
        }

        Ok(Self {
            is_master,
            is_private,
            key_data,
            chain_code,
            use_info,
            origin,
            children,
            parent_fingerprint,
            name,
            source,
        })
    }
}

#[uniffi::export]
impl CryptoHdkey {
    /// Encode as CBOR for UR
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.to_cbor()
    }

    /// Decode from CBOR
    #[uniffi::constructor]
    pub fn decode(cbor: Vec<u8>) -> Result<Self> {
        Self::from_cbor(&cbor)
    }
}

impl CryptoHdkey {
    /// Infer network from UR metadata, defaulting to mainnet if not available.
    ///
    /// Checks in order:
    /// 1. `use_info.network` - explicit network field (0=mainnet, 1=testnet)
    /// 2. Derivation path coin_type - second component (0=mainnet, 1=testnet)
    /// 3. Default to mainnet
    pub fn infer_network(&self) -> bitcoin::Network {
        // check use_info.network first (explicit)
        if let Some(ref use_info) = self.use_info
            && let Some(network) = use_info.network
        {
            return match network {
                0 => bitcoin::Network::Bitcoin,
                1 => bitcoin::Network::Testnet,
                _ => bitcoin::Network::Bitcoin,
            };
        }

        // check derivation path coin_type (index 1 in path like 84'/0'/0')
        if let Some(ref origin) = self.origin
            && origin.components.len() >= 2
        {
            let coin_type = origin.components[1] & 0x7FFFFFFF; // strip hardened bit
            return match coin_type {
                0 => bitcoin::Network::Bitcoin,
                1 => bitcoin::Network::Testnet,
                _ => bitcoin::Network::Bitcoin,
            };
        }

        bitcoin::Network::Bitcoin
    }

    /// Convert to xpub string (for public keys only)
    /// Note: Not exposed to uniffi because bitcoin::Network is not uniffi-compatible
    pub fn to_xpub_string(&self, network: bitcoin::Network) -> Result<String> {
        use bitcoin::bip32::{ChildNumber, Fingerprint, Xpub};
        use bitcoin::secp256k1::PublicKey;

        if self.is_private {
            return Err(UrError::InvalidOperation(
                "Cannot convert private key to xpub".to_string(),
            ));
        }

        if self.is_master {
            return Err(UrError::MasterKeyNotAllowed);
        }

        let public_key = PublicKey::from_slice(&self.key_data)
            .map_err(|e| UrError::InvalidKeyData(e.to_string()))?;

        let chain_code = self
            .chain_code
            .as_ref()
            .ok_or_else(|| UrError::MissingField("chain_code required for xpub".to_string()))?;

        let chain_code_array: [u8; 32] = chain_code
            .as_slice()
            .try_into()
            .map_err(|_| UrError::InvalidKeyData("chain_code must be 32 bytes".to_string()))?;

        let (depth, child_number) = match &self.origin {
            Some(origin) => (origin.components.len() as u8, origin.last_child_number()),
            None => (3, ChildNumber::from(0)), // account level, e.g. m/86'/0'/0'
        };
        let parent_fingerprint = self.parent_fingerprint.map(Fingerprint::from).unwrap_or_default();

        let xpub = Xpub {
            network: network.into(),
            depth,
            parent_fingerprint,
            child_number,
            chain_code: chain_code_array.into(),
            public_key,
        };

        Ok(xpub.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_crypto_hdkey_from_xpub() {
        // test xpub from BIP32 test vectors
        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        let crypto_hdkey = CryptoHdkey::from_xpub(&xpub);

        assert!(crypto_hdkey.is_master);
        assert!(!crypto_hdkey.is_private);
        assert_eq!(crypto_hdkey.key_data.len(), 33);
        assert!(crypto_hdkey.chain_code.is_some());
    }

    #[test]
    fn test_crypto_hdkey_from_xpriv() {
        // test xpriv from BIP32 test vectors
        let xpriv_str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
        let xpriv = Xpriv::from_str(xpriv_str).unwrap();

        let crypto_hdkey = CryptoHdkey::from_xpriv(&xpriv);

        assert!(crypto_hdkey.is_master);
        assert!(crypto_hdkey.is_private);
        assert_eq!(crypto_hdkey.key_data.len(), 32);
        assert!(crypto_hdkey.chain_code.is_some());
    }

    #[test]
    fn test_crypto_hdkey_cbor_roundtrip_xpub() {
        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        let crypto_hdkey = CryptoHdkey::from_xpub(&xpub);

        let cbor = crypto_hdkey.to_cbor().unwrap();
        let decoded = CryptoHdkey::from_cbor(&cbor).unwrap();

        assert_eq!(decoded, crypto_hdkey);
    }

    #[test]
    fn test_crypto_hdkey_cbor_roundtrip_xpriv() {
        let xpriv_str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
        let xpriv = Xpriv::from_str(xpriv_str).unwrap();

        let crypto_hdkey = CryptoHdkey::from_xpriv(&xpriv);

        let cbor = crypto_hdkey.to_cbor().unwrap();
        let decoded = CryptoHdkey::from_cbor(&cbor).unwrap();

        assert_eq!(decoded, crypto_hdkey);
    }

    #[test]
    fn test_infer_network_from_use_info() {
        use crate::coin_info::CryptoCoinInfo;

        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        // mainnet via use_info
        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.use_info = Some(CryptoCoinInfo::new(Some(0), Some(0)));
        assert_eq!(hdkey.infer_network(), bitcoin::Network::Bitcoin);

        // testnet via use_info
        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.use_info = Some(CryptoCoinInfo::new(Some(0), Some(1)));
        assert_eq!(hdkey.infer_network(), bitcoin::Network::Testnet);
    }

    #[test]
    fn test_infer_network_from_derivation_path() {
        use crate::keypath::CryptoKeypath;

        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        // mainnet: m/84'/0'/0'
        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.origin = Some(CryptoKeypath::new(
            vec![0x80000000 + 84, 0x80000000 + 0, 0x80000000 + 0],
            None,
            None,
        ));
        assert_eq!(hdkey.infer_network(), bitcoin::Network::Bitcoin);

        // testnet: m/84'/1'/0'
        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.origin = Some(CryptoKeypath::new(
            vec![0x80000000 + 84, 0x80000000 + 1, 0x80000000 + 0],
            None,
            None,
        ));
        assert_eq!(hdkey.infer_network(), bitcoin::Network::Testnet);
    }

    #[test]
    fn test_infer_network_defaults_to_mainnet() {
        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        // no use_info, no origin -> default to mainnet
        let hdkey = CryptoHdkey::from_xpub(&xpub);
        assert_eq!(hdkey.infer_network(), bitcoin::Network::Bitcoin);
    }

    #[test]
    fn test_to_xpub_string_rejects_master_key() {
        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.is_master = true;

        let result = hdkey.to_xpub_string(bitcoin::Network::Bitcoin);
        assert!(matches!(result, Err(crate::UrError::MasterKeyNotAllowed)));
    }

    #[test]
    fn test_to_xpub_string_defaults_to_account_level_depth() {
        let xpub_str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
        let xpub = Xpub::from_str(xpub_str).unwrap();

        // create hdkey without origin - should default to depth 3
        let mut hdkey = CryptoHdkey::from_xpub(&xpub);
        hdkey.is_master = false;
        hdkey.origin = None;

        let result = hdkey.to_xpub_string(bitcoin::Network::Bitcoin).unwrap();

        // parse the result back to verify depth is 3
        let parsed_xpub = Xpub::from_str(&result).unwrap();
        assert_eq!(parsed_xpub.depth, 3, "should default to account level depth (3)");
    }

    /// Test malformed CBOR: wrong tag
    #[test]
    fn test_crypto_hdkey_wrong_tag() {
        use crate::registry::CRYPTO_SEED;
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // use wrong tag (CRYPTO_SEED instead of CRYPTO_HDKEY)
        encoder.tag(Tag::new(CRYPTO_SEED)).unwrap();
        encoder.map(3).unwrap();
        encoder.u32(IS_MASTER).unwrap();
        encoder.bool(false).unwrap();
        encoder.u32(IS_PRIVATE).unwrap();
        encoder.bool(false).unwrap();
        encoder.u32(KEY_DATA).unwrap();
        encoder.bytes(&vec![0x02; lengths::COMPRESSED_PUBKEY]).unwrap();

        let result = CryptoHdkey::from_cbor(&cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::InvalidTag { expected: 303, actual: 300 }));
    }

    /// Test malformed CBOR: missing required field
    #[test]
    fn test_crypto_hdkey_missing_key_data() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // correct tag but missing key_data (KEY_DATA field)
        encoder.tag(Tag::new(CRYPTO_HDKEY)).unwrap();
        encoder.map(2).unwrap();
        encoder.u32(IS_MASTER).unwrap();
        encoder.bool(false).unwrap();
        encoder.u32(IS_PRIVATE).unwrap();
        encoder.bool(false).unwrap();

        let result = CryptoHdkey::from_cbor(&cbor);
        assert!(result.is_err());
        match result.unwrap_err() {
            UrError::MissingField(field) => assert_eq!(field, "key_data"),
            _ => panic!("Expected MissingField error"),
        }
    }

    /// Test malformed CBOR: invalid key data length
    #[test]
    fn test_crypto_hdkey_invalid_key_data_length() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // correct tag but wrong key data length (32 bytes for public key, should be 33)
        encoder.tag(Tag::new(CRYPTO_HDKEY)).unwrap();
        encoder.map(3).unwrap();
        encoder.u32(IS_MASTER).unwrap();
        encoder.bool(false).unwrap();
        encoder.u32(IS_PRIVATE).unwrap();
        encoder.bool(false).unwrap(); // public key
        encoder.u32(KEY_DATA).unwrap();
        encoder.bytes(&vec![0x02; lengths::PRIVATE_KEY]).unwrap(); // wrong length for public key

        let result = CryptoHdkey::from_cbor(&cbor);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            UrError::InvalidKeyDataLength { expected: 33, actual: 32 }
        ));
    }

    /// Test forward compatibility: unknown keys should be ignored
    #[test]
    fn test_crypto_hdkey_ignores_unknown_keys() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // create valid crypto-hdkey with an unknown key (99)
        encoder.tag(Tag::new(CRYPTO_HDKEY)).unwrap();
        encoder.map(4).unwrap();
        encoder.u32(IS_MASTER).unwrap();
        encoder.bool(false).unwrap(); // is_master
        encoder.u32(IS_PRIVATE).unwrap();
        encoder.bool(false).unwrap(); // is_private
        encoder.u32(KEY_DATA).unwrap();
        encoder.bytes(&vec![0x02; lengths::COMPRESSED_PUBKEY]).unwrap(); // key_data (public)
        encoder.u32(99).unwrap(); // unknown key
        encoder.str("future field").unwrap();

        let result = CryptoHdkey::from_cbor(&cbor);
        assert!(result.is_ok(), "Should ignore unknown key 99");
    }

    /// Test malformed CBOR: corrupted structure
    #[test]
    fn test_crypto_hdkey_corrupted_cbor() {
        // completely invalid CBOR
        let invalid_cbor = vec![0xFF, 0xFF, 0xFF];
        let result = CryptoHdkey::from_cbor(&invalid_cbor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UrError::CborDecodeError(_)));
    }
}
