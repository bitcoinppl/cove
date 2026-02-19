//! HKDF-SHA256 key derivation for the CSPP protocol
//!
//! See: <https://praveenperera.com/blog/passkey-prf-bitcoin-wallet-backup/>

use hkdf::Hkdf;
use sha2::Sha256;

/// HKDF info string for deriving the critical data encryption key
pub const CRITICAL_DATA_INFO: &[u8] = b"cspp:v1:critical";

/// HKDF info string for deriving the sensitive data encryption key
pub const SENSITIVE_DATA_INFO: &[u8] = b"cspp:v1:sensitive";

/// Derive the critical data encryption key
pub fn derive_critical_data_key(master_key: &[u8; 32]) -> [u8; 32] {
    derive_key(master_key, CRITICAL_DATA_INFO)
}

/// Derive the sensitive data encryption key
pub fn derive_sensitive_data_key(master_key: &[u8; 32]) -> [u8; 32] {
    derive_key(master_key, SENSITIVE_DATA_INFO)
}

// derive a 32-byte key from a master key using HKDF-SHA256
fn derive_key(master_key: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, master_key);
    let mut output = [0u8; 32];
    hkdf.expand(info, &mut output).expect("32 bytes is a valid HKDF-SHA256 output length");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_derivation() {
        let master_key = [42u8; 32];
        let key1 = derive_sensitive_data_key(&master_key);
        let key2 = derive_sensitive_data_key(&master_key);
        assert_eq!(key1, key2);

        let key1 = derive_critical_data_key(&master_key);
        let key2 = derive_critical_data_key(&master_key);
        assert_eq!(key1, key2);
    }

    #[test]
    fn different_info_different_keys() {
        let master_key = [42u8; 32];
        let critical = derive_critical_data_key(&master_key);
        let sensitive = derive_sensitive_data_key(&master_key);
        assert_ne!(critical, sensitive);
    }

    #[test]
    fn different_master_different_keys() {
        let master_a = [1u8; 32];
        let master_b = [2u8; 32];
        assert_ne!(derive_sensitive_data_key(&master_a), derive_sensitive_data_key(&master_b));
        assert_ne!(derive_critical_data_key(&master_a), derive_critical_data_key(&master_b));
    }
}
