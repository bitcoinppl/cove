use std::{hash::Hash, sync::Arc, time::Duration};

use crate::transaction::Unit;
use macros::{impl_default_for, new_type};
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};

use crate::{database::Database, network::Network};

use super::{fingerprint::Fingerprint, AddressInfo, WalletAddressType};

new_type!(WalletId, String);
impl_default_for!(WalletId);
impl WalletId {
    pub fn new() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }

    pub fn preview_new() -> Self {
        Self("testtesttest".to_string())
    }

    pub fn preview_new_random() -> Self {
        // random string id
        let rng = rand::thread_rng();
        let random_string: String = rng
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        Self(random_string)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct WalletMetadata {
    pub id: WalletId,
    pub name: String,
    pub color: WalletColor,
    pub verified: bool,
    pub network: Network,
    pub performed_full_scan: bool,
    #[serde(default)]
    pub master_fingerprint: Option<Arc<Fingerprint>>,
    #[serde(default)]
    pub selected_unit: Unit,
    #[serde(default = "default_true")]
    pub sensitive_visible: bool,
    #[serde(default = "default_false")]
    pub details_expanded: bool,
    #[serde(default)]
    pub wallet_type: WalletType,
    #[serde(default)]
    pub wallet_mode: WalletMode,
    #[serde(default)]
    pub discovery_state: DiscoveryState,
    #[serde(default = "default_address_type")]
    pub address_type: WalletAddressType,
    #[serde(default)]
    pub fiat_or_btc: FiatOrBtc,

    // internal only metadata, don't use in the UI
    // note: maybe better to use a separate table for this
    #[serde(default)]
    pub internal: InternalOnlyMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
#[serde(default)]
pub struct InternalOnlyMetadata {
    pub address_index: Option<AddressIndex>,
    pub last_scan_finished: Option<Duration>,
    pub last_height_fetched: Option<BlockSizeLast>,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record,
)]
pub struct BlockSizeLast {
    pub block_height: u64,
    pub last_seen: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct AddressIndex {
    pub last_seen_index: u8,
    pub address_list_hash: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum DiscoveryState {
    #[default]
    Single,
    StartedJson(Arc<FoundJson>),
    StartedMnemonic,
    FoundAddressesFromJson(Vec<FoundAddress>, Arc<FoundJson>),
    FoundAddressesFromMnemonic(Vec<FoundAddress>),
    NoneFound,
    ChoseAdressType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct FoundAddress {
    pub type_: WalletAddressType,
    pub first_address: String,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    PartialEq,
    uniffi::Object,
    derive_more::Into,
    derive_more::From,
    derive_more::Deref,
    derive_more::AsRef,
)]
pub struct FoundJson(pub pubport::formats::Json);

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum,
)]
pub enum WalletType {
    #[default]
    Hot,
    Cold,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum,
)]
pub enum WalletMode {
    #[default]
    Main,
    Decoy,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum,
)]
pub enum FiatOrBtc {
    #[default]
    Btc,
    Fiat,
}

impl WalletMetadata {
    pub fn new(name: impl Into<String>, fingerprint: impl Into<Arc<Fingerprint>>) -> Self {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::random(),
            master_fingerprint: Some(fingerprint.into()),
            verified: false,
            network,
            performed_full_scan: false,
            fiat_or_btc: FiatOrBtc::Btc,
            selected_unit: Unit::default(),
            sensitive_visible: true,
            details_expanded: false,
            address_type: WalletAddressType::default(),
            wallet_type: WalletType::Hot,
            wallet_mode,
            internal: InternalOnlyMetadata::default(),
            discovery_state: DiscoveryState::default(),
        }
    }

    pub fn new_with_id(
        id: WalletId,
        name: impl Into<String>,
        fingerprint: Option<Arc<Fingerprint>>,
    ) -> Self {
        let me = Self::new(
            name,
            fingerprint
                .map(Into::into)
                .unwrap_or_else(|| Arc::new(Fingerprint::default())),
        );

        Self {
            id,
            verified: true,
            wallet_type: WalletType::Cold,
            ..me
        }
    }

    pub fn new_imported_from_mnemonic(
        name: impl Into<String>,
        network: Network,
        fingerprint: impl Into<Arc<Fingerprint>>,
    ) -> Self {
        let mut me = Self::new(name, fingerprint);
        me.discovery_state = DiscoveryState::StartedMnemonic;

        Self {
            network,
            verified: true,
            ..me
        }
    }

    pub fn preview_new() -> Self {
        Self {
            id: WalletId::preview_new_random(),
            name: "Test Wallet".to_string(),
            master_fingerprint: Some(Arc::new(Fingerprint::default())),
            color: WalletColor::random(),
            verified: false,
            network: Network::Bitcoin,
            performed_full_scan: false,
            fiat_or_btc: FiatOrBtc::Btc,
            address_type: WalletAddressType::default(),
            selected_unit: Unit::default(),
            sensitive_visible: true,
            details_expanded: false,
            wallet_type: WalletType::Hot,
            wallet_mode: WalletMode::Main,
            internal: InternalOnlyMetadata::default(),
            discovery_state: DiscoveryState::default(),
        }
    }

    pub fn internal(&self) -> &InternalOnlyMetadata {
        &self.internal
    }

    pub fn internal_mut(&mut self) -> &mut InternalOnlyMetadata {
        &mut self.internal
    }
}

impl InternalOnlyMetadata {
    pub fn last_seen_address_index(&self, addreses: &[AddressInfo]) -> Option<usize> {
        let address_index = self.address_index.as_ref()?;
        let address_list_hash = crate::util::calculate_hash(addreses);

        // different address list, return none
        if address_index.address_list_hash != address_list_hash {
            return None;
        }

        Some(address_index.last_seen_index as usize)
    }

    pub fn set_last_seen_address_index(&mut self, addreses: &[AddressInfo], index: usize) {
        let address_list_hash = crate::util::calculate_hash(addreses);

        self.address_index = Some(AddressIndex {
            last_seen_index: index as u8,
            address_list_hash,
        });
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletColor {
    Red,
    Blue,
    Green,
    Yellow,
    Orange,
    Purple,
    Pink,
    CoolGray,
    Custom { r: u8, g: u8, b: u8 },

    // prefixed with W to avoid conflicts with swift asset color name
    WAlmostGray,
    WAlmostWhite,
    WBeige,
    WPastelBlue,
    WPastelNavy,
    WPastelRed,
    WPastelYellow,
    WLightMint,
    WPastelTeal,
    WLightPastelYellow,
}

#[uniffi::export]
fn default_wallet_colors() -> Vec<WalletColor> {
    vec![
        WalletColor::WBeige,
        WalletColor::WPastelBlue,
        WalletColor::WPastelNavy,
        WalletColor::WPastelRed,
        WalletColor::WPastelYellow,
        WalletColor::WPastelTeal,
        WalletColor::Blue,
        WalletColor::Green,
        WalletColor::Orange,
        WalletColor::Purple,
    ]
}

impl WalletColor {
    pub fn random() -> Self {
        let options = default_wallet_colors();

        let random_index = rand::thread_rng().gen_range(0..options.len());
        options[random_index]
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    true
}

fn default_address_type() -> WalletAddressType {
    Default::default()
}

// MARK: PREVIEW ONLY

#[uniffi::export]
fn wallet_metadata_preview() -> WalletMetadata {
    WalletMetadata::preview_new()
}
