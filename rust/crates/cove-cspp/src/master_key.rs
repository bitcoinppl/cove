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

    /// Derive the namespace identifier for cloud backup directory namespacing
    pub fn namespace_id(&self) -> String {
        crate::key_derivation::derive_namespace_id(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_keys_different_namespace_ids() {
        let key_a = MasterKey::generate();
        let key_b = MasterKey::generate();
        assert_ne!(key_a.namespace_id(), key_b.namespace_id());
    }
}
