use minicbor::{Decoder, Encoder, data::Tag};

use crate::{error::*, registry::CRYPTO_COIN_INFO};

/// crypto-coin-info: Cryptocurrency coin info
/// BCR-2020-007: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-007-hdkey.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoCoinInfo {
    /// SLIP-44 coin type (optional)
    pub coin_type: Option<u32>,
    /// Network type: 0=mainnet, 1=testnet (optional)
    pub network: Option<u32>,
}

impl CryptoCoinInfo {
    /// Create new CryptoCoinInfo
    pub fn new(coin_type: Option<u32>, network: Option<u32>) -> Self {
        Self { coin_type, network }
    }

    /// Encode as tagged CBOR
    /// CBOR structure: #6.305({?1: uint, ?2: uint})
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();
        let mut encoder = Encoder::new(&mut buffer);

        // write tag 305
        encoder
            .tag(Tag::new(CRYPTO_COIN_INFO))
            .map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        // count non-None fields
        let field_count = self.coin_type.is_some() as usize + self.network.is_some() as usize;

        // write map header
        encoder.map(field_count as u64).map_err(|e| UrError::CborEncodeError(e.to_string()))?;

        // write coin_type if present (key 1)
        if let Some(coin_type) = self.coin_type {
            encoder.u32(1).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
            encoder.u32(coin_type).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        }

        // write network if present (key 2)
        if let Some(network) = self.network {
            encoder.u32(2).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
            encoder.u32(network).map_err(|e| UrError::CborEncodeError(e.to_string()))?;
        }

        Ok(buffer)
    }

    /// Decode from tagged CBOR
    pub fn from_cbor(cbor: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(cbor);

        // read and verify tag 305
        let tag = decoder.tag().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

        if tag != Tag::new(CRYPTO_COIN_INFO) {
            return Err(UrError::InvalidTag { expected: CRYPTO_COIN_INFO, actual: tag.as_u64() });
        }

        // read map
        let map_len = decoder
            .map()
            .map_err(|e| UrError::CborDecodeError(e.to_string()))?
            .ok_or_else(|| UrError::CborDecodeError("Expected definite-length map".to_string()))?;

        let mut coin_type = None;
        let mut network = None;

        for _ in 0..map_len {
            let key = decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?;

            match key {
                1 => {
                    coin_type =
                        Some(decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?);
                }
                2 => {
                    network =
                        Some(decoder.u32().map_err(|e| UrError::CborDecodeError(e.to_string()))?);
                }
                _ => {
                    return Err(UrError::InvalidField(format!(
                        "Unknown key in coin-info: {}",
                        key
                    )));
                }
            }
        }

        Ok(Self { coin_type, network })
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
}
