use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::serde_helpers::{base64_serde, hex_array};

/// Record ID for the master key in CloudKit
pub const MASTER_KEY_RECORD_ID: &str = "cspp-master-key-v1";

/// Record ID for the backup manifest in CloudKit
pub const MANIFEST_RECORD_ID: &str = "cspp-manifest-v1";

/// Deterministic CloudKit record ID for a wallet: SHA-256(wallet_id) hex-encoded
pub fn wallet_record_id(wallet_id: &str) -> String {
    let hash = Sha256::digest(wallet_id.as_bytes());
    hex::encode(hash)
}

/// Wallet data to be encrypted and uploaded to cloud backup
#[derive(Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct WalletEntry {
    pub wallet_id: String,
    pub secret: WalletSecret,
    #[zeroize(skip)]
    pub metadata: serde_json::Value,
    pub descriptors: Option<DescriptorPair>,
    pub xpub: Option<String>,
    #[zeroize(skip)]
    pub wallet_mode: WalletMode,
}

/// Secret material for a wallet
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub enum WalletSecret {
    Mnemonic(String),
    TapSignerBackup(Vec<u8>),
    Descriptor(String),
    WatchOnly,
}

impl std::fmt::Debug for WalletSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mnemonic(_) => write!(f, "Mnemonic(****)"),
            Self::TapSignerBackup(_) => write!(f, "TapSignerBackup(****)"),
            Self::Descriptor(_) => write!(f, "Descriptor(****)"),
            Self::WatchOnly => write!(f, "WatchOnly"),
        }
    }
}

/// External and internal public descriptor strings
#[derive(Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct DescriptorPair {
    pub external: String,
    pub internal: String,
}

/// Distinguishes main vs decoy wallets in cloud backup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WalletMode {
    Main,
    Decoy,
}

/// Encrypted wallet backup envelope, uploaded to CloudKit
#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedWalletBackup {
    pub version: u32,
    /// Random per-wallet salt for HKDF derivation
    #[serde(with = "hex_array")]
    pub wallet_salt: [u8; 32],
    /// ChaCha20-Poly1305 nonce
    #[serde(with = "hex_array")]
    pub nonce: [u8; 12],
    /// Encrypted WalletEntry JSON
    #[serde(with = "base64_serde")]
    pub ciphertext: Vec<u8>,
}

/// Encrypted master key backup envelope, uploaded to CloudKit
#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedMasterKeyBackup {
    pub version: u32,
    /// PRF salt used with the passkey to re-derive the wrapping key
    #[serde(with = "hex_array")]
    pub prf_salt: [u8; 32],
    /// ChaCha20-Poly1305 nonce
    #[serde(with = "hex_array")]
    pub nonce: [u8; 12],
    /// Encrypted master key bytes
    #[serde(with = "base64_serde")]
    pub ciphertext: Vec<u8>,
}

/// Plaintext backup manifest — no secrets, uploaded last as commit marker
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: u32,
    /// Unix timestamp when backup was created
    pub created_at: u64,
    /// Deterministic SHA-256(wallet_id) hex IDs of all backed-up wallets
    pub wallet_record_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_record_id_is_deterministic() {
        let id1 = wallet_record_id("my-wallet-123");
        let id2 = wallet_record_id("my-wallet-123");
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn different_wallet_ids_produce_different_record_ids() {
        let id1 = wallet_record_id("wallet-a");
        let id2 = wallet_record_id("wallet-b");
        assert_ne!(id1, id2);
    }

    #[test]
    fn wallet_entry_json_roundtrip() {
        let entry = WalletEntry {
            wallet_id: "test-wallet".to_string(),
            secret: WalletSecret::Mnemonic("abandon abandon abandon".to_string()),
            metadata: serde_json::json!({"name": "Test Wallet"}),
            descriptors: Some(DescriptorPair {
                external: "wpkh(xpub/0/*)".to_string(),
                internal: "wpkh(xpub/1/*)".to_string(),
            }),
            xpub: Some("xpub661MyMwAqRbcF...".to_string()),
            wallet_mode: WalletMode::Main,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let decoded: WalletEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.wallet_id, "test-wallet");
        assert!(
            matches!(decoded.secret, WalletSecret::Mnemonic(ref m) if m == "abandon abandon abandon")
        );
        assert!(decoded.descriptors.is_some());
        assert_eq!(decoded.wallet_mode, WalletMode::Main);
    }

    #[test]
    fn encrypted_wallet_backup_json_roundtrip() {
        let backup = EncryptedWalletBackup {
            version: 1,
            wallet_salt: [0xAA; 32],
            nonce: [0xBB; 12],
            ciphertext: vec![1, 2, 3, 4, 5],
        };

        let json = serde_json::to_string(&backup).unwrap();
        let decoded: EncryptedWalletBackup = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.wallet_salt, [0xAA; 32]);
        assert_eq!(decoded.nonce, [0xBB; 12]);
        assert_eq!(decoded.ciphertext, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn encrypted_master_key_backup_json_roundtrip() {
        let backup = EncryptedMasterKeyBackup {
            version: 1,
            prf_salt: [0xCC; 32],
            nonce: [0xDD; 12],
            ciphertext: vec![10, 20, 30],
        };

        let json = serde_json::to_string(&backup).unwrap();
        let decoded: EncryptedMasterKeyBackup = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.prf_salt, [0xCC; 32]);
        assert_eq!(decoded.nonce, [0xDD; 12]);
        assert_eq!(decoded.ciphertext, vec![10, 20, 30]);
    }

    #[test]
    fn backup_manifest_json_roundtrip() {
        let manifest = BackupManifest {
            version: 1,
            created_at: 1700000000,
            wallet_record_ids: vec!["abc123".to_string(), "def456".to_string()],
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let decoded: BackupManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.wallet_record_ids.len(), 2);
    }

    #[test]
    fn wallet_secret_variants_roundtrip() {
        for (secret, tag) in [
            (WalletSecret::Mnemonic("test words".to_string()), "Mnemonic"),
            (WalletSecret::TapSignerBackup(vec![1, 2, 3]), "TapSignerBackup"),
            (WalletSecret::Descriptor("wpkh(...)".to_string()), "Descriptor"),
            (WalletSecret::WatchOnly, "WatchOnly"),
        ] {
            let json = serde_json::to_string(&secret).unwrap();
            assert!(json.contains(tag), "JSON should contain tag {tag}: {json}");
            let _decoded: WalletSecret = serde_json::from_str(&json).unwrap();
        }
    }
}
