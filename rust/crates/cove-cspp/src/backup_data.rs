use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::serde_helpers::{base64_serde, hex_array};

/// Record ID for the master key in cloud backup
pub const MASTER_KEY_RECORD_ID: &str = "cspp-master-key-v1";

/// Filename prefix for the master key file in a namespace directory
pub const MASTER_KEY_FILE_PREFIX: &str = "masterkey-";

/// Filename prefix for wallet backup files in a namespace directory
pub const WALLET_FILE_PREFIX: &str = "wallet-";

/// Subdirectory name for namespace directories within the iCloud Data folder
pub const NAMESPACES_SUBDIRECTORY: &str = "cspp-namespaces";

/// Deterministic cloud record ID for a wallet: SHA-256(wallet_id) hex-encoded
pub fn wallet_record_id(wallet_id: &str) -> String {
    let hash = Sha256::digest(wallet_id.as_bytes());
    hex::encode(hash)
}

/// Cloud filename for the master key: masterkey-{SHA256(MASTER_KEY_RECORD_ID)}.json
pub fn master_key_filename() -> String {
    let hash = Sha256::digest(MASTER_KEY_RECORD_ID.as_bytes());
    format!("{MASTER_KEY_FILE_PREFIX}{}.json", hex::encode(hash))
}

/// Cloud filename for a wallet backup: wallet-{SHA256(wallet_id)}.json
pub fn wallet_filename(wallet_id: &str) -> String {
    wallet_filename_from_record_id(&wallet_record_id(wallet_id))
}

/// Cloud filename for a wallet backup from a precomputed record id
pub fn wallet_filename_from_record_id(record_id: &str) -> String {
    format!("{WALLET_FILE_PREFIX}{record_id}.json")
}

/// Extract the wallet record ID (SHA256 hex) from a wallet filename
///
/// Returns None if the filename doesn't match the expected format
pub fn wallet_record_id_from_filename(filename: &str) -> Option<&str> {
    filename.strip_prefix(WALLET_FILE_PREFIX).and_then(|rest| rest.strip_suffix(".json"))
}

/// Supported encrypted master key backup versions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterKeyBackupVersion {
    V1,
}

impl MasterKeyBackupVersion {
    /// Returns the serialized backup version
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::V1 => 1,
        }
    }
}

/// Encrypted master key backup version not supported by this app
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedMasterKeyBackupVersion(pub u32);

impl TryFrom<u32> for MasterKeyBackupVersion {
    type Error = UnsupportedMasterKeyBackupVersion;

    fn try_from(version: u32) -> Result<Self, Self::Error> {
        match version {
            1 => Ok(Self::V1),
            version => Err(UnsupportedMasterKeyBackupVersion(version)),
        }
    }
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
    #[serde(with = "base64_serde::option")]
    pub labels_zstd_jsonl: Option<Vec<u8>>,
    pub labels_count: u32,
    pub labels_hash: Option<String>,
    pub labels_uncompressed_size: Option<u32>,
    pub content_revision_hash: String,
    pub updated_at: u64,
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

impl EncryptedMasterKeyBackup {
    /// Returns the parsed backup version when supported by this app
    pub fn backup_version(
        &self,
    ) -> Result<MasterKeyBackupVersion, UnsupportedMasterKeyBackupVersion> {
        self.version.try_into()
    }
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
            labels_zstd_jsonl: Some(vec![1, 2, 3]),
            labels_count: 2,
            labels_hash: Some("labels-hash".to_string()),
            labels_uncompressed_size: Some(123),
            content_revision_hash: "content-hash".to_string(),
            updated_at: 42,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let decoded: WalletEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.wallet_id, "test-wallet");
        assert!(
            matches!(decoded.secret, WalletSecret::Mnemonic(ref m) if m == "abandon abandon abandon")
        );
        assert!(decoded.descriptors.is_some());
        assert_eq!(decoded.wallet_mode, WalletMode::Main);
        assert_eq!(decoded.labels_zstd_jsonl, Some(vec![1, 2, 3]));
        assert_eq!(decoded.labels_count, 2);
        assert_eq!(decoded.labels_hash.as_deref(), Some("labels-hash"));
        assert_eq!(decoded.labels_uncompressed_size, Some(123));
        assert_eq!(decoded.content_revision_hash, "content-hash");
        assert_eq!(decoded.updated_at, 42);
    }

    #[test]
    fn wallet_entry_json_rejects_missing_new_fields() {
        let json = serde_json::json!({
            "wallet_id": "test-wallet",
            "secret": "WatchOnly",
            "metadata": {"name": "Test Wallet"},
            "descriptors": null,
            "xpub": null,
            "wallet_mode": "Main"
        });

        let error = serde_json::from_value::<WalletEntry>(json).unwrap_err();

        assert!(error.to_string().contains("labels_zstd_jsonl"));
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
    fn master_key_filename_format() {
        let filename = master_key_filename();
        assert!(filename.starts_with("masterkey-"));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn wallet_filename_format() {
        let filename = wallet_filename("my-wallet-123");
        assert!(filename.starts_with("wallet-"));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn wallet_filename_from_record_id_format() {
        let filename = wallet_filename_from_record_id("abc123");
        assert_eq!(filename, "wallet-abc123.json");
    }

    #[test]
    fn wallet_record_id_from_filename_roundtrip() {
        let wallet_id = "my-wallet-123";
        let record_id = wallet_record_id(wallet_id);
        let filename = wallet_filename(wallet_id);
        let extracted = wallet_record_id_from_filename(&filename).unwrap();
        assert_eq!(extracted, record_id);
    }

    #[test]
    fn wallet_record_id_from_filename_rejects_masterkey() {
        let filename = master_key_filename();
        assert!(wallet_record_id_from_filename(&filename).is_none());
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
