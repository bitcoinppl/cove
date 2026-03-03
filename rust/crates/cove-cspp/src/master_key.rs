use rand::RngExt as _;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Generate a new random master key
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Construct from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive the encryption key for non-critical sensitive data (e.g. not mnemonics or private keys)
    pub fn sensitive_data_key(&self) -> [u8; 32] {
        crate::key_derivation::derive_sensitive_data_key(&self.0)
    }

    /// Derive the critical data encryption key
    pub fn critical_data_key(&self) -> [u8; 32] {
        crate::key_derivation::derive_critical_data_key(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_32_bytes() {
        let key = MasterKey::generate();
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn from_bytes_roundtrip() {
        let bytes = [42u8; 32];
        let key = MasterKey::from_bytes(bytes);
        assert_eq!(*key.as_bytes(), bytes);
    }

    #[test]
    fn sensitive_data_key_derivation() {
        let key = MasterKey::generate();
        let derived1 = key.sensitive_data_key();
        let derived2 = key.sensitive_data_key();
        assert_eq!(derived1, derived2);
    }

    #[test]
    fn critical_data_key_derivation() {
        let key = MasterKey::generate();
        let derived1 = key.critical_data_key();
        let derived2 = key.critical_data_key();
        assert_eq!(derived1, derived2);
    }
}
