use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::wallet::metadata::WalletType;
use cove_types::network::Network;

pub const PAYLOAD_VERSION: u32 = 1;

/// Top-level backup payload, serialized to JSON before compression and encryption
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    /// Format version, currently PAYLOAD_VERSION
    pub version: u32,
    /// Unix timestamp (seconds) when backup was created
    pub created_at: u64,
    /// All wallet data
    pub wallets: Vec<WalletBackup>,
    /// App settings
    pub settings: AppSettings,
}

impl BackupPayload {
    pub fn new(wallets: Vec<WalletBackup>, settings: AppSettings) -> Self {
        Self {
            version: PAYLOAD_VERSION,
            created_at: jiff::Timestamp::now().as_second().try_into().unwrap_or_else(|e| {
                tracing::warn!("timestamp conversion failed, using epoch: {e}");
                0
            }),
            wallets,
            settings,
        }
    }

    /// Deserialize from JSON bytes and validate in one step
    pub fn decode(bytes: &[u8]) -> Result<Self, super::error::BackupError> {
        let payload: Self = serde_json::from_slice(bytes)
            .map_err(|e| super::error::BackupError::Deserialization(e.to_string()))?;
        payload.validate()?;
        Ok(payload)
    }

    /// Validate the payload after deserialization
    pub fn validate(&self) -> Result<(), super::error::BackupError> {
        if self.version > PAYLOAD_VERSION {
            return Err(super::error::BackupError::UnsupportedPayloadVersion(self.version));
        }
        Ok(())
    }
}

/// Per-wallet backup data
#[derive(Debug, Serialize, Deserialize)]
pub struct WalletBackup {
    /// WalletMetadata serialized as JSON value for forward compatibility
    pub metadata: serde_json::Value,
    /// Secret key material, varies by wallet type
    pub secret: WalletSecret,
    /// Public descriptor pair (external + internal), always present or absent together
    pub descriptors: Option<DescriptorPair>,
    /// Extended public key string
    pub xpub: Option<String>,
    /// BIP-329 labels as JSONL string
    pub labels_jsonl: Option<String>,
}

/// External and internal public descriptor strings, always paired
#[derive(Debug, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct DescriptorPair {
    pub external: String,
    pub internal: String,
}

impl Drop for WalletBackup {
    fn drop(&mut self) {
        // serde_json::Value doesn't implement Zeroize, replace with Null to drop the JSON tree
        self.metadata = serde_json::Value::Null;
        self.secret.zeroize();

        if let Some(ref mut xpub) = self.xpub {
            xpub.zeroize();
        }
        if let Some(ref mut labels) = self.labels_jsonl {
            labels.zeroize();
        }
        if let Some(ref mut descs) = self.descriptors {
            descs.zeroize();
        }
    }
}

/// Secret material for a wallet, depends on wallet type
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub enum WalletSecret {
    /// Hot wallet BIP-39 mnemonic
    Mnemonic(String),
    /// TapSigner encrypted backup bytes
    TapSignerBackup(Vec<u8>),
    /// No secret material (xpub-only / watch-only)
    None,
    /// Unrecognized variant from a newer backup version
    #[serde(other)]
    Unknown,
}

impl std::fmt::Debug for WalletSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mnemonic(_) => write!(f, "Mnemonic(****)"),
            Self::TapSignerBackup(_) => write!(f, "TapSignerBackup(****)"),
            Self::None => write!(f, "None"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// App-level settings to back up (excludes security-sensitive items)
#[derive(Debug, Serialize, Deserialize)]
pub struct AppSettings {
    /// Backed up for completeness but intentionally not restored (device-specific)
    pub selected_network: Option<String>,
    pub selected_fiat_currency: Option<String>,
    pub color_scheme: Option<String>,
    /// Per-network node configuration: (network_string, node_config_json)
    pub selected_nodes: Vec<(String, String)>,
}

/// Result of a successful backup export
#[derive(Debug, uniffi::Record)]
pub struct BackupResult {
    pub data: Vec<u8>,
    pub filename: String,
    pub warnings: Vec<String>,
}

/// Report of what happened during a backup import
#[derive(Debug, Default, uniffi::Record)]
pub struct BackupImportReport {
    pub wallets_imported: u32,
    pub imported_wallet_names: Vec<String>,
    pub wallets_skipped: u32,
    pub skipped_wallet_names: Vec<String>,
    pub wallets_failed: u32,
    pub failed_wallet_names: Vec<String>,
    pub failed_wallet_errors: Vec<String>,
    pub wallets_with_labels_imported: u32,
    pub labels_failed_wallet_names: Vec<String>,
    pub labels_failed_errors: Vec<String>,
    pub settings_restored: bool,
    pub settings_error: Option<String>,
    /// Wallets imported with degraded functionality (e.g. unknown secret type)
    pub degraded_wallet_names: Vec<String>,
    /// Warnings about partial cleanup failures (orphaned keychain entries, etc)
    pub cleanup_warnings: Vec<String>,
}

impl BackupImportReport {
    /// Derive counts from list lengths to prevent desync
    pub fn finalize(mut self) -> Self {
        self.wallets_imported = self.imported_wallet_names.len() as u32;
        self.wallets_skipped = self.skipped_wallet_names.len() as u32;
        self.wallets_failed = self.failed_wallet_names.len() as u32;
        self
    }
}

#[derive(Debug, uniffi::Enum)]
pub enum WalletSecretType {
    Mnemonic,
    TapSignerBackup,
    None,
    Unknown,
}

#[uniffi::export]
impl WalletSecretType {
    pub fn display_name(&self) -> String {
        match self {
            Self::Mnemonic => "Mnemonic",
            Self::TapSignerBackup => "TapSigner",
            Self::None => "Xpub Only",
            Self::Unknown => "Unknown",
        }
        .to_string()
    }
}

impl From<&WalletSecret> for WalletSecretType {
    fn from(secret: &WalletSecret) -> Self {
        match secret {
            WalletSecret::Mnemonic(_) => Self::Mnemonic,
            WalletSecret::TapSignerBackup(_) => Self::TapSignerBackup,
            WalletSecret::None => Self::None,
            WalletSecret::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, uniffi::Record)]
pub struct BackupVerifyReport {
    pub created_at: u64,
    pub wallet_count: u32,
    pub wallets: Vec<BackupWalletSummary>,
    pub fiat_currency: Option<String>,
    pub color_scheme: Option<String>,
    pub node_config_count: u32,
}

#[derive(Debug, uniffi::Record)]
pub struct BackupWalletSummary {
    pub name: String,
    pub network: Network,
    pub wallet_type: WalletType,
    pub fingerprint: Option<String>,
    pub secret_type: WalletSecretType,
    pub has_xpub: bool,
    pub has_descriptors: bool,
    pub label_count: u32,
    pub already_on_device: bool,
    pub warning: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> BackupPayload {
        BackupPayload {
            version: PAYLOAD_VERSION,
            created_at: 1700000000,
            wallets: vec![WalletBackup {
                metadata: serde_json::json!({"name": "test wallet", "id": "abc123"}),
                secret: WalletSecret::Mnemonic("abandon abandon abandon".to_string()),
                descriptors: Some(DescriptorPair {
                    external: "wpkh([abc/84'/0'/0']xpub/0/*)".to_string(),
                    internal: "wpkh([abc/84'/0'/0']xpub/1/*)".to_string(),
                }),
                xpub: Some("xpub661MyMwAqRbcF...".to_string()),
                labels_jsonl: Some(
                    "{\"type\":\"tx\",\"ref\":\"abc\",\"label\":\"test\"}".to_string(),
                ),
            }],
            settings: AppSettings {
                selected_network: Some("bitcoin".to_string()),
                selected_fiat_currency: Some("USD".to_string()),
                color_scheme: Some("dark".to_string()),
                selected_nodes: vec![(
                    "bitcoin".to_string(),
                    "{\"url\":\"localhost\"}".to_string(),
                )],
            },
        }
    }

    #[test]
    fn backup_payload_json_round_trip() {
        let payload = sample_payload();

        let json = serde_json::to_vec(&payload).unwrap();
        let decoded: BackupPayload = serde_json::from_slice(&json).unwrap();

        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.created_at, 1700000000);
        assert_eq!(decoded.wallets.len(), 1);
        assert_eq!(decoded.settings.selected_network.as_deref(), Some("bitcoin"));
    }

    #[test]
    fn cold_wallet_tap_signer_round_trip() {
        use std::sync::Arc;

        use bitcoin::secp256k1::PublicKey;
        use cove_tap_card::{TapSigner, TapSignerState};

        use crate::wallet::WalletAddressType;
        use crate::wallet::metadata::{
            DiscoveryState, FiatOrBtc, HardwareWalletMetadata, InternalOnlyMetadata, WalletColor,
            WalletMetadata, WalletMode, WalletType,
        };

        // secp256k1 generator point G (a valid compressed pubkey)
        let pubkey = PublicKey::from_slice(&[
            0x02, 0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE,
            0x87, 0x0B, 0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81,
            0x5B, 0x16, 0xF8, 0x17, 0x98,
        ])
        .unwrap();

        let tap_signer = Arc::new(TapSigner {
            state: TapSignerState::Sealed,
            card_ident: "TEST-IDENT".to_string(),
            nonce: "deadbeef".to_string(),
            signature: "cafebabe".to_string(),
            pubkey: Arc::new(pubkey),
        });

        let metadata = WalletMetadata {
            id: "test-tap-signer-wallet".into(),
            name: "TapSigner Wallet".to_string(),
            color: WalletColor::Blue,
            verified: true,
            network: Network::Bitcoin,
            master_fingerprint: None,
            selected_unit: Default::default(),
            sensitive_visible: true,
            details_expanded: false,
            wallet_type: WalletType::Cold,
            wallet_mode: WalletMode::Main,
            discovery_state: DiscoveryState::Single,
            address_type: WalletAddressType::NativeSegwit,
            fiat_or_btc: FiatOrBtc::Btc,
            origin: None,
            hardware_metadata: Some(HardwareWalletMetadata::TapSigner(tap_signer)),
            show_labels: true,
            internal: InternalOnlyMetadata::default(),
        };

        // step 1: serialize metadata to Value (same as export.rs:54)
        let metadata_value = serde_json::to_value(&metadata).unwrap();

        // step 2: build a BackupPayload containing this wallet
        let payload = BackupPayload {
            version: PAYLOAD_VERSION,
            created_at: 1700000000,
            wallets: vec![WalletBackup {
                metadata: metadata_value,
                secret: WalletSecret::TapSignerBackup(vec![1, 2, 3]),
                descriptors: None,
                xpub: None,
                labels_jsonl: None,
            }],
            settings: AppSettings {
                selected_network: None,
                selected_fiat_currency: None,
                color_scheme: None,
                selected_nodes: vec![],
            },
        };

        // step 3: JSON round-trip (simulates export → import)
        let json = serde_json::to_vec(&payload).unwrap();
        let restored: BackupPayload = serde_json::from_slice(&json).unwrap();

        // step 4: convert metadata Value back to WalletMetadata (same as verify.rs / import.rs)
        let restored_metadata: WalletMetadata =
            serde_json::from_value(restored.wallets[0].metadata.clone())
                .expect("WalletMetadata with TapSigner should round-trip through JSON");

        // step 5: verify the TapSigner data survived
        let hw = restored_metadata.hardware_metadata.expect("hardware_metadata should be present");
        match hw {
            HardwareWalletMetadata::TapSigner(ts) => {
                assert_eq!(*ts.pubkey, pubkey);
                assert_eq!(ts.card_ident, "TEST-IDENT");
                assert_eq!(ts.state, TapSignerState::Sealed);
            }
        }
        assert_eq!(restored_metadata.wallet_type, WalletType::Cold);
        assert_eq!(restored_metadata.name, "TapSigner Wallet");
    }

    #[test]
    fn json_zstd_round_trip() {
        let payload = sample_payload();

        let json = serde_json::to_vec(&payload).unwrap();
        let compressed = super::super::crypto::compress(&json).unwrap();
        let decompressed = super::super::crypto::decompress(&compressed).unwrap();
        let decoded: BackupPayload = serde_json::from_slice(&decompressed).unwrap();

        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.wallets.len(), 1);

        match &decoded.wallets[0].secret {
            WalletSecret::Mnemonic(m) => assert_eq!(m, "abandon abandon abandon"),
            _ => panic!("expected Mnemonic"),
        }
    }
}
