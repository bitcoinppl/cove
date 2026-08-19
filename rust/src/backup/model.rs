use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroize;

use crate::database::global_config::CertificateTrustStore;
use crate::wallet::metadata::WalletType;
use cove_types::network::Network;
use cove_util::ResultExt as _;

mod wallet_secret;

pub use wallet_secret::WalletSecret;

pub const PAYLOAD_VERSION: u32 = 2;
const BASELINE_PAYLOAD_VERSION: u32 = 1;

/// Top-level backup payload, serialized to JSON before compression and encryption
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    /// Minimum payload version required to decode the contained wallet data
    pub version: u32,
    /// Unix timestamp (seconds) when backup was created
    pub created_at: u64,
    /// All wallet data
    pub wallets: Vec<WalletBackup>,
    /// App settings
    pub settings: AppSettings,
}

impl BackupPayload {
    /// Builds a payload using the oldest format version that can represent its wallet secrets
    pub fn try_new(
        wallets: Vec<WalletBackup>,
        settings: AppSettings,
    ) -> Result<Self, super::error::BackupError> {
        if wallets.iter().any(|wallet| matches!(&wallet.secret, WalletSecret::Unknown)) {
            return Err(super::error::BackupError::Serialization(
                "an unknown wallet secret cannot be exported".to_string(),
            ));
        }

        let version =
            if wallets.iter().any(|wallet| matches!(&wallet.secret, WalletSecret::Xprv(_))) {
                PAYLOAD_VERSION
            } else {
                BASELINE_PAYLOAD_VERSION
            };

        Ok(Self {
            version,
            created_at: jiff::Timestamp::now().as_second().try_into().unwrap_or_else(|e| {
                tracing::warn!("timestamp conversion failed, using epoch: {e}");
                0
            }),
            wallets,
            settings,
        })
    }

    /// Deserialize from JSON bytes and validate in one step
    pub fn decode(bytes: &[u8]) -> Result<Self, super::error::BackupError> {
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }

        let header: Header = serde_json::from_slice(bytes)
            .map_err_str(super::error::BackupError::Deserialization)?;
        validate_payload_version(header.version)?;

        let payload: Self = serde_json::from_slice(bytes)
            .map_err_str(super::error::BackupError::Deserialization)?;
        payload.validate()?;
        Ok(payload)
    }

    /// Validate the payload after deserialization
    pub fn validate(&self) -> Result<(), super::error::BackupError> {
        validate_payload_version(self.version)?;

        if self.version == BASELINE_PAYLOAD_VERSION
            && self.wallets.iter().any(|wallet| matches!(&wallet.secret, WalletSecret::Xprv(_)))
        {
            return Err(super::error::BackupError::Deserialization(
                "payload version 1 cannot contain an extended private key secret".to_string(),
            ));
        }

        Ok(())
    }
}

fn validate_payload_version(version: u32) -> Result<(), super::error::BackupError> {
    match version {
        BASELINE_PAYLOAD_VERSION | PAYLOAD_VERSION => Ok(()),
        0 => Err(super::error::BackupError::InvalidFormat),
        version => Err(super::error::BackupError::UnsupportedPayloadVersion(version)),
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

/// Certificate trust state at the backup boundary
#[derive(Debug, Clone, PartialEq)]
pub enum BackupCertificateTrustStore {
    /// A certificate trust map that passed all endpoint and TLS validation
    Valid(CertificateTrustStore),
    /// A trust map that was retained as invalid so other backup data can restore
    Invalid {
        /// Validation error from the certificate trust store
        error: String,
        /// Original JSON value, retained for payload round trips
        raw: serde_json::Value,
    },
}

impl Default for BackupCertificateTrustStore {
    fn default() -> Self {
        Self::Valid(CertificateTrustStore::default())
    }
}

impl BackupCertificateTrustStore {
    /// Converts a raw durable value without discarding invalid settings
    pub(crate) fn from_stored_value(raw: Option<String>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };

        match serde_json::from_str(&raw) {
            Ok(value) => Self::from_raw_value(value),
            Err(error) => {
                Self::Invalid { error: error.to_string(), raw: serde_json::Value::String(raw) }
            }
        }
    }

    fn from_raw_value(raw: serde_json::Value) -> Self {
        match serde_json::from_value::<CertificateTrustStore>(raw.clone()) {
            Ok(store) => Self::Valid(store),
            Err(error) => Self::Invalid { error: error.to_string(), raw },
        }
    }
}

impl Serialize for BackupCertificateTrustStore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Valid(store) => store.serialize(serializer),
            Self::Invalid { raw, .. } => raw.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for BackupCertificateTrustStore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = serde_json::Value::deserialize(deserializer)?;

        Ok(Self::from_raw_value(raw))
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
    /// Per-network normalized custom transaction explorer templates
    #[serde(default)]
    pub custom_block_explorers: BTreeMap<String, String>,
    /// Remembered certificate trust for custom SSL Electrum endpoints
    #[serde(default)]
    pub certificate_trust_store: BackupCertificateTrustStore,
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
    /// A BIP32 extended private key
    Xprv,
    TapSignerBackup,
    None,
    Unknown,
}

#[uniffi::export]
impl WalletSecretType {
    pub fn display_name(&self) -> String {
        match self {
            Self::Mnemonic => "Mnemonic",
            Self::Xprv => "Extended Private Key",
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
            WalletSecret::Xprv(_) => Self::Xprv,
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
                custom_block_explorers: BTreeMap::from([(
                    "Bitcoin".to_string(),
                    "https://example.com/tx/{txid}".to_string(),
                )]),
                certificate_trust_store: BackupCertificateTrustStore::default(),
            },
        }
    }

    #[test]
    fn backup_payload_json_round_trip() {
        let payload = sample_payload();

        let json = serde_json::to_vec(&payload).unwrap();
        let decoded: BackupPayload = serde_json::from_slice(&json).unwrap();

        assert_eq!(decoded.version, PAYLOAD_VERSION);
        assert_eq!(decoded.created_at, 1700000000);
        assert_eq!(decoded.wallets.len(), 1);
        assert_eq!(decoded.settings.selected_network.as_deref(), Some("bitcoin"));
        assert_eq!(
            decoded.settings.custom_block_explorers.get("Bitcoin").map(String::as_str),
            Some("https://example.com/tx/{txid}")
        );
    }

    #[test]
    fn version_one_mnemonic_payload_remains_readable() {
        let mut payload = sample_payload();
        payload.version = 1;
        let json = serde_json::to_vec(&payload).unwrap();

        let decoded = BackupPayload::decode(&json).unwrap();

        assert_eq!(decoded.version, 1);
        assert!(matches!(&decoded.wallets[0].secret, WalletSecret::Mnemonic(_)));
    }

    #[test]
    fn new_payload_uses_baseline_version_without_xprv() {
        let sample = sample_payload();

        let payload = BackupPayload::try_new(sample.wallets, sample.settings).unwrap();

        assert_eq!(payload.version, BASELINE_PAYLOAD_VERSION);
    }

    #[test]
    fn new_payload_uses_current_version_for_xprv() {
        let mut sample = sample_payload();
        sample.wallets[0].secret = WalletSecret::Xprv("xprv-test".to_string());

        let payload = BackupPayload::try_new(sample.wallets, sample.settings).unwrap();

        assert_eq!(payload.version, PAYLOAD_VERSION);
    }

    #[test]
    fn current_payload_roundtrips_xprv_secret_shape() {
        let mut payload = sample_payload();
        payload.wallets[0].secret = WalletSecret::Xprv(
            "xprv9s21ZrQH143K4BwRCYKSEPwcAMYweWkfKLURabnnv2GLNhJN1LSCgDQyGWyNcat72najQKwyshCBXWfHHVbcdxPAZPqByMyWDbWp5SjCfEa"
                .to_string(),
        );
        let json = serde_json::to_vec(&payload).unwrap();

        let decoded = BackupPayload::decode(&json).unwrap();

        assert_eq!(decoded.version, PAYLOAD_VERSION);
        assert!(matches!(&decoded.wallets[0].secret, WalletSecret::Xprv(_)));
    }

    #[test]
    fn version_one_rejects_xprv_content() {
        let mut payload = sample_payload();
        payload.version = BASELINE_PAYLOAD_VERSION;
        payload.wallets[0].secret = WalletSecret::Xprv("xprv-test".to_string());
        let json = serde_json::to_vec(&payload).unwrap();

        let error = BackupPayload::decode(&json).unwrap_err();

        assert!(matches!(
            error,
            super::super::error::BackupError::Deserialization(message)
                if message.contains("version 1")
        ));
    }

    #[test]
    fn unsupported_version_is_rejected_before_wallet_body() {
        let json = br#"{
            "version": 99,
            "created_at": 1700000000,
            "wallets": [{"secret": {"Mnemonic": 42}}],
            "settings": {}
        }"#;

        let error = BackupPayload::decode(json).unwrap_err();

        assert!(matches!(error, super::super::error::BackupError::UnsupportedPayloadVersion(99)));
    }

    #[test]
    fn zero_payload_version_is_invalid_format() {
        let error = BackupPayload::decode(br#"{"version":0}"#).unwrap_err();

        assert!(matches!(error, super::super::error::BackupError::InvalidFormat));
    }

    #[test]
    fn new_payload_rejects_unknown_secret() {
        let mut sample = sample_payload();
        sample.wallets[0].secret = WalletSecret::Unknown;

        let error = BackupPayload::try_new(sample.wallets, sample.settings).unwrap_err();

        assert!(matches!(
            error,
            super::super::error::BackupError::Serialization(message)
                if message.contains("unknown wallet secret")
        ));
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
            birthday: None,
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
                custom_block_explorers: BTreeMap::new(),
                certificate_trust_store: BackupCertificateTrustStore::default(),
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
    fn old_backup_without_custom_block_explorers_deserializes() {
        let json = serde_json::json!({
            "version": 1,
            "created_at": 1700000000_u64,
            "wallets": [],
            "settings": {
                "selected_network": "bitcoin",
                "selected_fiat_currency": "USD",
                "color_scheme": "dark",
                "selected_nodes": []
            }
        });

        let payload: BackupPayload = serde_json::from_value(json).unwrap();

        assert!(payload.settings.custom_block_explorers.is_empty());
        assert!(matches!(
            payload.settings.certificate_trust_store,
            BackupCertificateTrustStore::Valid(store) if store == CertificateTrustStore::default()
        ));
    }

    #[test]
    fn json_zstd_round_trip() {
        let payload = sample_payload();

        let json = serde_json::to_vec(&payload).unwrap();
        let compressed = crate::backup::crypto::compress(&json).unwrap();
        let decompressed = crate::backup::crypto::decompress(&compressed).unwrap();
        let decoded: BackupPayload = serde_json::from_slice(&decompressed).unwrap();

        assert_eq!(decoded.version, PAYLOAD_VERSION);
        assert_eq!(decoded.wallets.len(), 1);

        match &decoded.wallets[0].secret {
            WalletSecret::Mnemonic(m) => assert_eq!(m, "abandon abandon abandon"),
            _ => panic!("expected Mnemonic"),
        }
    }

    #[test]
    fn old_app_settings_without_custom_block_explorers_deserializes() {
        let json = serde_json::json!({
            "selected_network": "bitcoin",
            "selected_fiat_currency": "USD",
            "color_scheme": "dark",
            "selected_nodes": []
        });

        let settings: AppSettings = serde_json::from_value(json).unwrap();

        assert!(settings.custom_block_explorers.is_empty());
        assert!(matches!(
            settings.certificate_trust_store,
            BackupCertificateTrustStore::Valid(store) if store == CertificateTrustStore::default()
        ));
    }

    #[test]
    fn certificate_trust_store_roundtrips_with_the_existing_map_shape() {
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![4; 32] };
        let mut certificate_trust_store = CertificateTrustStore::default();
        certificate_trust_store
            .insert_or_match("ssl://node.example.com:50002".to_string(), trust)
            .unwrap();

        let settings = AppSettings {
            selected_network: None,
            selected_fiat_currency: None,
            color_scheme: None,
            selected_nodes: vec![],
            custom_block_explorers: BTreeMap::new(),
            certificate_trust_store: BackupCertificateTrustStore::Valid(
                certificate_trust_store.clone(),
            ),
        };
        let encoded = serde_json::to_value(&settings).unwrap();

        assert_eq!(
            encoded["certificate_trust_store"],
            serde_json::json!({
                "ssl://node.example.com:50002": {
                    "PinnedFingerprint": { "sha256": vec![4; 32] }
                }
            })
        );

        let decoded: AppSettings = serde_json::from_value(encoded).unwrap();

        assert_eq!(
            decoded.certificate_trust_store,
            BackupCertificateTrustStore::Valid(certificate_trust_store)
        );
    }

    #[test]
    fn invalid_certificate_trust_does_not_fail_payload_decode() {
        let invalid_fields = [
            (
                serde_json::json!({
                    "tcp://node.example.com:50001": {
                        "PinnedFingerprint": { "sha256": [1, 2, 3] }
                    }
                }),
                "invalid endpoint",
            ),
            (
                serde_json::json!({
                    "ssl://node.example.com:50002": {
                        "PinnedFingerprint": { "sha256": [1, 2, 3] }
                    }
                }),
                "invalid TLS trust",
            ),
            (
                serde_json::json!({
                    "ssl://node.example.com:50002": {
                        "PinnedFingerprint": { "sha256": vec![1; 32] }
                    },
                    "ssl://NODE.example.com:50002/path": {
                        "PinnedFingerprint": { "sha256": vec![2; 32] }
                    }
                }),
                "conflicts for endpoint",
            ),
        ];

        for (certificate_trust_store, expected_error) in invalid_fields {
            let expected_raw = certificate_trust_store.clone();
            let json = serde_json::json!({
                "version": PAYLOAD_VERSION,
                "created_at": 1700000000_u64,
                "wallets": [],
                "settings": {
                    "selected_nodes": [],
                    "certificate_trust_store": certificate_trust_store,
                }
            });
            let payload = BackupPayload::decode(serde_json::to_vec(&json).unwrap().as_slice())
                .expect("invalid trust must not abort backup decoding");

            assert!(matches!(
                &payload.settings.certificate_trust_store,
                BackupCertificateTrustStore::Invalid { error, .. }
                    if error.contains(expected_error)
            ));
            assert_eq!(
                serde_json::to_value(&payload).unwrap()["settings"]["certificate_trust_store"],
                expected_raw
            );
        }
    }

    #[test]
    fn raw_certificate_trust_values_keep_valid_and_invalid_states() {
        assert!(matches!(
            BackupCertificateTrustStore::from_stored_value(None),
            BackupCertificateTrustStore::Valid(store)
                if store == CertificateTrustStore::default()
        ));

        let valid = serde_json::json!({
            "ssl://node.example.com:50002": {
                "PinnedFingerprint": { "sha256": vec![5; 32] }
            }
        });
        assert!(matches!(
            BackupCertificateTrustStore::from_stored_value(Some(valid.to_string())),
            BackupCertificateTrustStore::Valid(_)
        ));

        let invalid = serde_json::json!({
            "ssl://node.example.com:50002": {
                "PinnedFingerprint": { "sha256": [1, 2, 3] }
            }
        });
        assert!(matches!(
            BackupCertificateTrustStore::from_stored_value(Some(invalid.to_string())),
            BackupCertificateTrustStore::Invalid { raw, error }
                if raw == invalid && error.contains("invalid TLS trust")
        ));

        let malformed = "{not valid json".to_string();
        assert!(matches!(
            BackupCertificateTrustStore::from_stored_value(Some(malformed.clone())),
            BackupCertificateTrustStore::Invalid { raw, error }
                if raw == serde_json::Value::String(malformed)
                    && error.contains("key must be a string")
        ));
    }

    #[test]
    fn invalid_local_trust_does_not_block_wallet_payload_construction() {
        let mut sample = sample_payload();
        sample.settings.certificate_trust_store =
            BackupCertificateTrustStore::from_stored_value(Some("{not valid json".to_string()));

        let payload = BackupPayload::try_new(sample.wallets, sample.settings).unwrap();

        assert_eq!(payload.wallets.len(), 1);
        assert!(matches!(
            payload.settings.certificate_trust_store,
            BackupCertificateTrustStore::Invalid { .. }
        ));
    }
}
