//! HKDF-SHA256 key derivation for the CSPP protocol
//!
//! See: <https://praveenperera.com/blog/passkey-prf-bitcoin-wallet-backup/>

use hkdf::Hkdf;
use sha2::Sha256;

/// HKDF info string for deriving the critical data encryption key
pub const CRITICAL_DATA_INFO: &[u8] = b"cspp:v1:critical";

/// HKDF info string for deriving the sensitive data encryption key
pub const SENSITIVE_DATA_INFO: &[u8] = b"cspp:v1:sensitive";

/// HKDF info string for deriving per-wallet encryption keys
pub const WALLET_KEY_INFO: &[u8] = b"cspp:v1:wallet";

/// HKDF info string for deriving the namespace identifier from a master key
pub const NAMESPACE_ID_INFO: &[u8] = b"cspp:v1:namespace-id";

/// Derive the critical data encryption key
pub fn derive_critical_data_key(master_key: &[u8; 32]) -> [u8; 32] {
    derive_key(master_key, CRITICAL_DATA_INFO)
}

/// Derive the sensitive data encryption key
pub fn derive_sensitive_data_key(master_key: &[u8; 32]) -> [u8; 32] {
    derive_key(master_key, SENSITIVE_DATA_INFO)
}

/// Derive a wallet backup encryption key
///
/// Uses the wallet salt as the HKDF salt, the critical data key as input keying
/// material, and `cspp:v1:wallet` as the info string
pub fn derive_wallet_key(critical_data_key: &[u8; 32], wallet_salt: &[u8; 32]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(wallet_salt), critical_data_key);
    let mut output = [0u8; 32];
    hkdf.expand(WALLET_KEY_INFO, &mut output)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    output
}

/// Derive a 32-char hex namespace ID from a master key
///
/// Uses HKDF-SHA256 with the master key, takes the first 16 bytes, and hex-encodes
/// them to produce a deterministic, irreversible, unique-per-master-key identifier
pub fn derive_namespace_id(master_key: &[u8; 32]) -> String {
    let full = derive_key(master_key, NAMESPACE_ID_INFO);
    hex::encode(&full[..16])
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

    #[test]
    fn wallet_key_differs_from_other_derived_keys() {
        let master = [42u8; 32];
        let critical = derive_critical_data_key(&master);
        let sensitive = derive_sensitive_data_key(&master);
        let wallet_salt = [1u8; 32];
        let wallet = derive_wallet_key(&critical, &wallet_salt);

        assert_ne!(wallet, critical);
        assert_ne!(wallet, sensitive);
    }

    #[test]
    fn different_wallet_salts_produce_different_keys() {
        let critical = derive_critical_data_key(&[42u8; 32]);
        let salt_a = [1u8; 32];
        let salt_b = [2u8; 32];

        assert_ne!(derive_wallet_key(&critical, &salt_a), derive_wallet_key(&critical, &salt_b));
    }

    #[test]
    fn namespace_id_is_deterministic() {
        let master_key = [42u8; 32];
        let id1 = derive_namespace_id(&master_key);
        let id2 = derive_namespace_id(&master_key);
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 32); // 16 bytes hex-encoded = 32 chars
    }

    #[test]
    fn different_master_keys_produce_different_namespace_ids() {
        let id_a = derive_namespace_id(&[1u8; 32]);
        let id_b = derive_namespace_id(&[2u8; 32]);
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn namespace_id_differs_from_other_derived_keys() {
        let master = [42u8; 32];
        let ns_id = derive_namespace_id(&master);
        let critical = hex::encode(derive_critical_data_key(&master));
        let sensitive = hex::encode(derive_sensitive_data_key(&master));
        assert_ne!(ns_id, &critical[..32]);
        assert_ne!(ns_id, &sensitive[..32]);
    }
}
