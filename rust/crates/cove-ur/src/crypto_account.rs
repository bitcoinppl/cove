//! crypto-account: Account descriptor with multiple output descriptors
//! BCR-2020-015: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-015-account.md

use minicbor::{Decoder, data::Tag};
use pubport::descriptor::ScriptType;

use crate::{
    crypto_hdkey::CryptoHdkey,
    error::*,
    registry::{
        CRYPTO_ACCOUNT, CRYPTO_HDKEY, CRYPTO_OUTPUT, PAY_TO_PUBKEY_HASH, SCRIPT_HASH, TAPROOT,
        WITNESS_PUBKEY_HASH,
    },
};

/// crypto-account: Account with multiple output descriptors for different script types
/// BCR-2020-015
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoAccount {
    /// Master key fingerprint (4 bytes)
    pub master_fingerprint: [u8; 4],
    /// Output descriptors for different script types
    pub output_descriptors: Vec<OutputDescriptor>,
}

/// An output descriptor containing a script type and HD key
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDescriptor {
    /// Script type (P2PKH, P2WPKH, P2SH-P2WPKH, P2TR)
    pub script_type: ScriptType,
    /// The HD key for this descriptor
    pub hdkey: CryptoHdkey,
}

impl CryptoAccount {
    /// Decode from CBOR bytes (with tag 311)
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);

        // read and verify tag 311
        let tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

        if tag != Tag::new(CRYPTO_ACCOUNT) {
            return Err(UrError::InvalidTag { expected: CRYPTO_ACCOUNT, actual: tag.as_u64() });
        }

        Self::decode_inner(&mut decoder, cbor)
    }

    /// Decode from CBOR bytes without the outer tag (for when tag is already consumed)
    pub fn from_cbor_untagged(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);
        Self::decode_inner(&mut decoder, cbor)
    }

    fn decode_inner(decoder: &mut Decoder, cbor: &[u8]) -> Result<Self> {
        // read map
        let map_len = decoder
            .map()
            .map_err(|e| UrError::CborDecodeError(e.to_string()))?
            .ok_or_else(|| UrError::CborDecodeError("Expected definite-length map".to_string()))?;

        let mut master_fingerprint = None;
        let mut output_descriptors = Vec::new();

        for _ in 0..map_len {
            let key = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

            match key {
                1 => {
                    // master fingerprint as uint32
                    let fp = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                    master_fingerprint = Some(fp.to_be_bytes());
                }
                2 => {
                    // array of output descriptors
                    let arr_len = decoder
                        .array()
                        .map_err(|e| UrError::CborDecodeError(e.to_string()))?
                        .ok_or_else(|| {
                            UrError::CborDecodeError("Expected definite-length array".to_string())
                        })?;

                    for _ in 0..arr_len {
                        // each descriptor is wrapped in script type tags then hdkey tag
                        let pos = decoder.position();
                        if let Some(descriptor) = decode_output_descriptor(&cbor[pos..])? {
                            output_descriptors.push(descriptor);
                        }
                        // skip over the descriptor structure
                        decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                    }
                }
                _ => {
                    // skip unknown fields
                    decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                }
            }
        }

        let master_fingerprint = master_fingerprint
            .ok_or_else(|| UrError::MissingField("master_fingerprint".to_string()))?;

        Ok(Self { master_fingerprint, output_descriptors })
    }

    /// Get the preferred output descriptor (P2WPKH > P2SH-P2WPKH > P2PKH)
    /// Returns None if only P2TR is available
    pub fn get_preferred_descriptor(&self) -> Option<&OutputDescriptor> {
        self.output_descriptors
            .iter()
            .find(|d| d.script_type == ScriptType::P2wpkh)
            .or_else(|| {
                self.output_descriptors.iter().find(|d| d.script_type == ScriptType::P2shP2wpkh)
            })
            .or_else(|| self.output_descriptors.iter().find(|d| d.script_type == ScriptType::P2pkh))
    }

    /// Check if this account only has P2TR (taproot) descriptors
    pub fn is_taproot_only(&self) -> bool {
        !self.output_descriptors.is_empty()
            && self.output_descriptors.iter().all(|d| d.script_type == ScriptType::P2tr)
    }
}

/// Decode an output descriptor from CBOR
/// Returns None for unsupported script types, Some for supported ones
///
/// BCR-2020-015 structure: tag(308) tag(script_type) tag(303) {hdkey_map}
/// Where:
/// - 308 = crypto-output wrapper
/// - script_type = 403 (P2PKH), 404 (P2WPKH), 400 (P2SH), 409 (P2TR)
/// - 303 = crypto-hdkey
pub(crate) fn decode_output_descriptor(cbor: &[u8]) -> Result<Option<OutputDescriptor>> {
    let mut decoder = Decoder::new(cbor);

    // read the first tag - might be crypto-output (308) wrapper or direct script type
    let first_tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

    // if it's crypto-output wrapper (308), read the next tag for script type
    let script_type_tag = if first_tag.as_u64() == CRYPTO_OUTPUT {
        decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?
    } else {
        first_tag
    };

    let script_type = match script_type_tag.as_u64() {
        PAY_TO_PUBKEY_HASH => {
            // P2PKH (BIP44)
            ScriptType::P2pkh
        }
        WITNESS_PUBKEY_HASH => {
            // P2WPKH (BIP84)
            ScriptType::P2wpkh
        }
        TAPROOT => {
            // P2TR (BIP86) - supported but Cove doesn't use it yet
            ScriptType::P2tr
        }
        SCRIPT_HASH => {
            // P2SH wrapper - check for nested P2WPKH
            let nested_tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

            if nested_tag.as_u64() == WITNESS_PUBKEY_HASH {
                // P2SH-P2WPKH (BIP49)
                ScriptType::P2shP2wpkh
            } else {
                // some other P2SH type - skip
                return Ok(None);
            }
        }
        _ => {
            // unknown script type - skip
            return Ok(None);
        }
    };

    // now read the hdkey tag
    let hdkey_tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

    if hdkey_tag.as_u64() != CRYPTO_HDKEY {
        return Err(UrError::InvalidTag { expected: CRYPTO_HDKEY, actual: hdkey_tag.as_u64() });
    }

    // decode the hdkey (without tag since we already consumed it)
    let pos = decoder.position();
    let hdkey = decode_hdkey_untagged(&cbor[pos..])?;

    Ok(Some(OutputDescriptor { script_type, hdkey }))
}

/// Decode a CryptoHdkey without the outer tag
fn decode_hdkey_untagged(cbor: &[u8]) -> Result<CryptoHdkey> {
    let mut decoder = Decoder::new(cbor);

    // read map
    let map_len = decoder
        .map()
        .map_err(|e| UrError::CborDecodeError(e.to_string()))?
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
        let key = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

        match key {
            1 => {
                is_master =
                    Some(decoder.bool().map_err(|e| UrError::CborDecodeError(e.to_string()))?);
            }
            2 => {
                is_private =
                    Some(decoder.bool().map_err(|e| UrError::CborDecodeError(e.to_string()))?);
            }
            3 => {
                key_data = Some(
                    decoder.bytes().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_vec(),
                );
            }
            4 => {
                chain_code = Some(
                    decoder.bytes().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_vec(),
                );
            }
            5 => {
                // use_info - skip for now
                let pos = decoder.position();
                use_info = Some(crate::coin_info::CryptoCoinInfo::from_cbor(&cbor[pos..]).ok());
                decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
            }
            6 => {
                // origin - skip for now
                let pos = decoder.position();
                origin = Some(crate::keypath::CryptoKeypath::from_cbor(&cbor[pos..]).ok());
                decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
            }
            7 => {
                // children - skip for now
                let pos = decoder.position();
                children = Some(crate::keypath::CryptoKeypath::from_cbor(&cbor[pos..]).ok());
                decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
            }
            8 => {
                let fp = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
                parent_fingerprint = Some(fp.to_be_bytes());
            }
            9 => {
                name = Some(
                    decoder.str().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_string(),
                );
            }
            10 => {
                source = Some(
                    decoder.str().map_err(|e| UrError::CborDecodeError(e.to_string()))?.to_string(),
                );
            }
            _ => {
                decoder.skip().map_err(|e| UrError::CborDecodeError(e.to_string()))?;
            }
        }
    }

    // is_master and is_private default to false if not present (BCR-2020-007)
    let is_master = is_master.unwrap_or(false);
    let is_private = is_private.unwrap_or(false);
    let key_data = key_data.ok_or_else(|| UrError::MissingField("key_data".to_string()))?;

    // validate key data length
    // private keys are 32 bytes, public keys are 33 bytes (compressed)
    let expected_len = if is_private { 32 } else { 33 };
    if key_data.len() != expected_len {
        return Err(UrError::InvalidKeyDataLength {
            expected: expected_len as u64,
            actual: key_data.len() as u64,
        });
    }

    Ok(CryptoHdkey {
        is_master,
        is_private,
        key_data,
        chain_code,
        use_info: use_info.flatten(),
        origin: origin.flatten(),
        children: children.flatten(),
        parent_fingerprint,
        name,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BCR-2020-015 official test vector
    /// BIP39 seed: "shield group erode awake lock sausage cash glare wave crew flame glove"
    /// Master fingerprint: 37b5eed4
    /// Contains: P2PKH (44'), P2SH-P2WPKH (49'), P2WPKH (84'), P2TR (86'), multisig keys
    const BCR_SPEC_CBOR_HEX: &str = "a2011a37b5eed40287d90134d90193d9012fa403582103eb3e2863911826374de86c231a4b76f0b89dfa174afb78d7f478199884d9dd320458206456a5df2db0f6d9af72b2a1af4b25f45200ed6fcc29c3440b311d4796b70b5b06d90130a20186182cf500f500f5021a37b5eed4081a99f9cdf7d90134d90190d90194d9012fa403582102c7e4823730f6ee2cf864e2c352060a88e60b51a84e89e4c8c75ec22590ad6b690458209d2f86043276f9251a4a4f577166a5abeb16b6ec61e226b5b8fa11038bfda42d06d90130a201861831f500f500f5021a37b5eed4081aa80f7cdbd90134d90194d9012fa403582103fd433450b6924b4f7efdd5d1ed017d364be95ab2b592dc8bddb3b00c1c24f63f04582072ede7334d5acf91c6fda622c205199c595a31f9218ed30792d301d5ee9e3a8806d90130a201861854f500f500f5021a37b5eed4081a0d5de1d7d90134d90190d9019ad9012fa4035821035ccd58b63a2cdc23d0812710603592e7457573211880cb59b1ef012e168e059a04582088d3299b448f87215d96b0c226235afc027f9e7dc700284f3e912a34daeb1a2306d90130a20182182df5021a37b5eed4081a37b5eed4d90134d90190d90191d9019ad9012fa4035821032c78ebfcabdac6d735a0820ef8732f2821b4fb84cd5d6b26526938f90c0507110458207953efe16a73e5d3f9f2d4c6e49bd88e22093bbd85be5a7e862a4b98a16e0ab606d90130a201881830f500f500f501f5021a37b5eed4081a59b69b2ad90134d90191d9019ad9012fa40358210260563ee80c26844621b06b74070baf0e23fb76ce439d0237e87502ebbd3ca3460458202fa0e41c9dc43dc4518659bfcef935ba8101b57dbc0812805dd983bc1d34b81306d90130a201881830f500f500f502f5021a37b5eed4081a59b69b2ad90134d90199d9012fa403582102bbb97cf9efa176b738efd6ee1d4d0fa391a973394fbc16e4c5e78e536cd14d2d0458204b4693e1f794206ed1355b838da24949a92b63d02e58910bf3bd3d9c242281e606d90130a201861856f500f500f5021a37b5eed4081acec7070c";

    #[test]
    fn test_crypto_account_bcr_spec_vector() {
        // decode CBOR hex
        let cbor = hex::decode(BCR_SPEC_CBOR_HEX).unwrap();

        // parse the crypto-account (no outer tag 311 in this CBOR)
        let account = CryptoAccount::from_cbor_untagged(&cbor).unwrap();

        // verify master fingerprint = 37b5eed4
        assert_eq!(account.master_fingerprint, [0x37, 0xb5, 0xee, 0xd4]);

        // verify we have output descriptors
        assert!(!account.output_descriptors.is_empty(), "Should have output descriptors");

        // collect script types found
        let script_types: Vec<_> =
            account.output_descriptors.iter().map(|d| &d.script_type).collect();

        // verify we have the expected script types (at minimum P2PKH and P2WPKH)
        assert!(script_types.contains(&&ScriptType::P2pkh), "Should contain P2PKH descriptor");
        assert!(script_types.contains(&&ScriptType::P2wpkh), "Should contain P2WPKH descriptor");

        // verify get_preferred_descriptor returns P2WPKH (highest priority)
        let preferred =
            account.get_preferred_descriptor().expect("Should have preferred descriptor");
        assert_eq!(
            preferred.script_type,
            ScriptType::P2wpkh,
            "Preferred descriptor should be P2WPKH"
        );

        // verify is_taproot_only returns false (we have non-taproot descriptors)
        assert!(
            !account.is_taproot_only(),
            "Should not be taproot-only since we have P2PKH and P2WPKH"
        );

        // verify each output descriptor has valid hdkey data
        for descriptor in &account.output_descriptors {
            assert!(!descriptor.hdkey.key_data.is_empty(), "HD key should have key data");
            // public keys are 33 bytes compressed
            assert_eq!(descriptor.hdkey.key_data.len(), 33, "Public key should be 33 bytes");
        }
    }

    #[test]
    fn test_crypto_account_taproot_only() {
        // create a crypto-account with only P2TR descriptors
        // this tests the is_taproot_only() function

        // for this test, we'll create one manually since the BCR spec vector has multiple types
        let taproot_account = CryptoAccount {
            master_fingerprint: [0x12, 0x34, 0x56, 0x78],
            output_descriptors: vec![OutputDescriptor {
                script_type: ScriptType::P2tr,
                hdkey: CryptoHdkey {
                    is_master: false,
                    is_private: false,
                    key_data: vec![0x02; 33], // dummy 33-byte key
                    chain_code: Some(vec![0x00; 32]),
                    use_info: None,
                    origin: None,
                    children: None,
                    parent_fingerprint: None,
                    name: None,
                    source: None,
                },
            }],
        };

        assert!(taproot_account.is_taproot_only(), "Account with only P2TR should be taproot-only");
        assert!(
            taproot_account.get_preferred_descriptor().is_none(),
            "Taproot-only account should return None for preferred descriptor"
        );
    }

    #[test]
    fn test_crypto_account_empty() {
        // test an account with no output descriptors
        let empty_account = CryptoAccount {
            master_fingerprint: [0x00, 0x00, 0x00, 0x00],
            output_descriptors: vec![],
        };

        assert!(!empty_account.is_taproot_only(), "Empty account should not be taproot-only");
        assert!(
            empty_account.get_preferred_descriptor().is_none(),
            "Empty account should return None for preferred descriptor"
        );
    }
}
