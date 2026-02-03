//! UR (Uniform Resources) support for scanning and generating QR codes.

/// Supported UR types for Bitcoin operations
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum UrType {
    /// crypto-psbt - Partially Signed Bitcoin Transaction
    CryptoPsbt,
    /// crypto-seed - BIP39 seed
    CryptoSeed,
    /// crypto-hdkey - HD key (xpub/xprv)
    CryptoHdkey,
    /// crypto-account - Account descriptor
    CryptoAccount,
    /// crypto-output - Output descriptor
    CryptoOutput,
    /// bytes - Raw bytes
    Bytes,
    /// Unknown type with raw string
    Unknown(String),
}

impl UrType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "crypto-psbt" => Self::CryptoPsbt,
            "crypto-seed" => Self::CryptoSeed,
            "crypto-hdkey" => Self::CryptoHdkey,
            "crypto-account" => Self::CryptoAccount,
            "crypto-output" => Self::CryptoOutput,
            "bytes" => Self::Bytes,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Result of a completed UR decode
#[derive(Debug, Clone, uniffi::Object)]
pub struct UrResult {
    /// Raw decoded bytes (CBOR encoded for crypto-* types)
    pub data: Vec<u8>,
    /// The UR type
    pub ur_type: UrType,
}

#[uniffi::export]
impl UrResult {
    #[uniffi::constructor]
    pub const fn new(data: Vec<u8>, ur_type: UrType) -> Self {
        Self { data, ur_type }
    }

    #[uniffi::method]
    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }

    #[uniffi::method]
    pub fn ur_type(&self) -> UrType {
        self.ur_type.clone()
    }

    #[uniffi::method]
    pub const fn is_psbt(&self) -> bool {
        matches!(self.ur_type, UrType::CryptoPsbt)
    }

    #[uniffi::method]
    pub const fn is_seed(&self) -> bool {
        matches!(self.ur_type, UrType::CryptoSeed)
    }

    #[uniffi::method]
    pub const fn is_hdkey(&self) -> bool {
        matches!(self.ur_type, UrType::CryptoHdkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ur_type_from_str() {
        assert_eq!(UrType::from_str("crypto-psbt"), UrType::CryptoPsbt);
        assert_eq!(UrType::from_str("CRYPTO-PSBT"), UrType::CryptoPsbt);
        assert_eq!(UrType::from_str("crypto-seed"), UrType::CryptoSeed);
        assert_eq!(UrType::from_str("crypto-hdkey"), UrType::CryptoHdkey);
        assert_eq!(UrType::from_str("crypto-account"), UrType::CryptoAccount);
        assert_eq!(UrType::from_str("crypto-output"), UrType::CryptoOutput);
        assert_eq!(UrType::from_str("bytes"), UrType::Bytes);
        assert!(matches!(UrType::from_str("unknown-type"), UrType::Unknown(_)));
    }

    #[test]
    fn test_ur_result_helpers() {
        let psbt = UrResult::new(vec![1, 2, 3], UrType::CryptoPsbt);
        assert!(psbt.is_psbt());
        assert!(!psbt.is_seed());
        assert!(!psbt.is_hdkey());

        let seed = UrResult::new(vec![4, 5, 6], UrType::CryptoSeed);
        assert!(!seed.is_psbt());
        assert!(seed.is_seed());
        assert!(!seed.is_hdkey());

        let hdkey = UrResult::new(vec![7, 8, 9], UrType::CryptoHdkey);
        assert!(!hdkey.is_psbt());
        assert!(!hdkey.is_seed());
        assert!(hdkey.is_hdkey());
    }

    #[test]
    fn test_ur_result_data_access() {
        let data = vec![1, 2, 3, 4, 5];
        let result = UrResult::new(data.clone(), UrType::Bytes);
        assert_eq!(result.data(), data);
        assert_eq!(result.ur_type(), UrType::Bytes);
    }
}
