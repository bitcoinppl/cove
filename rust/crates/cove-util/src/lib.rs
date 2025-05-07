pub mod encryption;
pub mod format;

use bitcoin::secp256k1::hashes::sha256::Hash as Sha256Hash;
use std::hash::{DefaultHasher, Hasher as _};

uniffi::setup_scaffolding!();

pub fn calculate_hash<T>(t: &T) -> u64
where
    T: std::hash::Hash + ?Sized,
{
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

pub fn generate_random_chain_code() -> [u8; 32] {
    use rand::Rng as _;

    let rng = &mut rand::rng();
    let mut chain_code = [0u8; 32];
    rng.fill(&mut chain_code);

    chain_code
}

pub fn sha256_hash(bytes: &[u8]) -> Sha256Hash {
    use bitcoin::hashes::Hash as _;
    Sha256Hash::hash(bytes)
}

pub fn message_digest(message: &[u8]) -> bitcoin::secp256k1::Message {
    let hash = sha256_hash(message);
    let digest: &[u8; 32] = hash.as_ref();
    bitcoin::secp256k1::Message::from_digest_slice(digest).expect("just hashed so hash is 32 bytes")
}

pub fn split_at_decimal_point(amount: &str) -> (&str, &str, &str) {
    let decimal_index = match memchr::memchr(b'.', amount.as_bytes()) {
        Some(decimal_index) => decimal_index,
        None => return (amount, "", ""),
    };

    let before_decimal = &amount[..decimal_index];
    let after_decimal = &amount[decimal_index + 1..];
    (before_decimal, ".", after_decimal)
}

mod ffi {
    #[uniffi::export]
    fn hex_encode(bytes: Vec<u8>) -> String {
        hex::encode(bytes)
    }

    #[uniffi::export]
    fn hex_decode(hex: &str) -> Option<Vec<u8>> {
        hex::decode(hex).ok()
    }

    #[uniffi::export]
    pub fn generate_random_chain_code() -> String {
        use rand::Rng as _;

        let rng = &mut rand::rng();
        let mut chain_code = [0u8; 32];
        rng.fill(&mut chain_code);

        hex::encode(chain_code)
    }

    #[uniffi::export]
    fn hex_to_utf8_string(hex: &str) -> Option<String> {
        let bytes = hex_decode(hex)?;
        String::from_utf8(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_at_decimal_point() {
        let amount = "0.00";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "0");
        assert_eq!(after_decimal, "00");

        let amount = "0.01";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "0");
        assert_eq!(after_decimal, "01");

        let amount = "0.1";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "0");
        assert_eq!(after_decimal, "1");

        let amount = "0.12";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "0");
        assert_eq!(after_decimal, "12");

        let amount = "0.123";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "0");
        assert_eq!(after_decimal, "123");

        let amount = "3856.1234";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "3856");
        assert_eq!(after_decimal, "1234");

        let amount = "1234.0";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "1234");
        assert_eq!(after_decimal, "0");

        let amount = "1234.00";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "1234");
        assert_eq!(after_decimal, "00");

        let amount = "1234.000";
        let (before_decimal, _decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "1234");
        assert_eq!(after_decimal, "000");

        let amount = "1234.";
        let (before_decimal, decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "1234");
        assert_eq!(decimal, ".");
        assert_eq!(after_decimal, "");

        let amount = "1234";
        let (before_decimal, decimal, after_decimal) = split_at_decimal_point(amount);
        assert_eq!(before_decimal, "1234");
        assert_eq!(decimal, "");
        assert_eq!(after_decimal, "");
    }
}
