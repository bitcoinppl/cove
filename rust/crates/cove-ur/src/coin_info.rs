//! crypto-coin-info: Cryptocurrency coin info
//! BCR-2020-007: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-007-hdkey.md

use minicbor::{Decode, Encode};

use crate::error::*;

/// crypto-coin-info: Cryptocurrency coin info
/// CBOR structure: #6.305({?1: uint, ?2: uint})
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cbor(tag(305), map)]
pub struct CryptoCoinInfo {
    /// SLIP-44 coin type (optional)
    #[n(1)]
    pub coin_type: Option<u32>,

    /// Network type: 0=mainnet, 1=testnet (optional)
    #[n(2)]
    pub network: Option<u32>,
}

impl CryptoCoinInfo {
    /// Create new CryptoCoinInfo
    pub fn new(coin_type: Option<u32>, network: Option<u32>) -> Self {
        Self { coin_type, network }
    }

    /// Encode as tagged CBOR
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        minicbor::to_vec(self).map_err(|e| UrError::CborEncodeError(e.to_string()))
    }

    /// Decode from tagged CBOR
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let info: CryptoCoinInfo = minicbor::decode(cbor).map_err_cbor_decode()?;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coin_info_bitcoin_mainnet() {
        let coin_info = CryptoCoinInfo::new(Some(0), Some(0)); // Bitcoin mainnet

        let cbor = coin_info.to_cbor().unwrap();
        let decoded = CryptoCoinInfo::from_cbor(&cbor).unwrap();

        assert_eq!(decoded, coin_info);
    }

    #[test]
    fn test_coin_info_empty() {
        let coin_info = CryptoCoinInfo::new(None, None);

        let cbor = coin_info.to_cbor().unwrap();
        let decoded = CryptoCoinInfo::from_cbor(&cbor).unwrap();

        assert_eq!(decoded, coin_info);
    }

    /// Test forward compatibility: unknown keys should be ignored
    #[test]
    fn test_coin_info_ignores_unknown_keys() {
        use minicbor::{Encoder, data::Tag};

        let mut cbor = Vec::new();
        let mut encoder = Encoder::new(&mut cbor);

        // create valid crypto-coin-info with an unknown key (99)
        encoder.tag(Tag::new(305)).unwrap();
        encoder.map(3).unwrap();
        encoder.u32(1).unwrap();
        encoder.u32(0).unwrap(); // coin_type = 0 (Bitcoin)
        encoder.u32(2).unwrap();
        encoder.u32(0).unwrap(); // network = 0 (mainnet)
        encoder.u32(99).unwrap(); // unknown key
        encoder.str("future field").unwrap();

        let result = CryptoCoinInfo::from_cbor(&cbor);
        assert!(result.is_ok(), "Should ignore unknown key 99");
        let info = result.unwrap();
        assert_eq!(info.coin_type, Some(0));
        assert_eq!(info.network, Some(0));
    }

    /// Test that derive macros produce same CBOR as manual implementation
    #[test]
    fn test_cbor_output_matches_spec() {
        let coin_info = CryptoCoinInfo::new(Some(0), Some(0));
        let cbor = coin_info.to_cbor().unwrap();

        // manually verify CBOR structure: d9 0131 a2 01 00 02 00
        // d9 0131 = tag 305
        // a2 = map(2)
        // 01 00 = key 1, value 0
        // 02 00 = key 2, value 0
        assert_eq!(cbor[0], 0xd9); // tag marker
        assert_eq!(cbor[1], 0x01); // tag high byte (305 >> 8)
        assert_eq!(cbor[2], 0x31); // tag low byte (305 & 0xff)
        assert_eq!(cbor[3], 0xa2); // map(2)
    }
}
