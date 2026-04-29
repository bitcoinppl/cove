use std::{
    hash::{Hash, Hasher as _},
    sync::Arc,
    time::Duration,
};

use derive_more::Display;
use serde::{Deserialize, Serialize};

use super::{AddressInfo, WalletAddressType, fingerprint::Fingerprint};
use crate::transaction::Unit;
use crate::{database::Database, network::Network};
use cove_tap_card::TapSigner;

pub use cove_types::{BlockSizeLast, WalletId};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
#[uniffi::export(Eq, Hash)]
pub struct WalletMetadata {
    pub id: WalletId,
    pub name: String,
    pub color: WalletColor,
    pub verified: bool,
    pub network: Network,

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
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub birthday: Option<WalletBirthday>,

    /// Metadata data specific to different hardware wallets
    #[serde(default)]
    pub hardware_metadata: Option<HardwareWalletMetadata>,

    /// Show labels for transactions in the transaction list
    /// If false, we only show either `Sent` or `Received` labels
    #[serde(default = "default_true")]
    pub show_labels: bool,

    // internal only metadata, don't use in the UI
    // note: maybe better to use a separate table for this
    #[serde(default)]
    pub internal: InternalOnlyMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
#[serde(default)]
pub struct InternalOnlyMetadata {
    #[serde(default)]
    pub address_index: Option<cove_types::AddressIndex>,

    #[serde(default)]
    /// this is the last time the wallet was scanned, this includes the initial scna, expanded scan, and incremental scan
    pub last_scan_finished: Option<Duration>,

    #[serde(default)]
    pub last_height_fetched: Option<cove_types::BlockSizeLast>,

    #[serde(default)]
    /// this is the time that a full expanded scan was completed, this should only happen once
    pub performed_full_scan_at: Option<u64>,

    // the type of store used for the wallet
    #[serde(default = "file_store_default")]
    pub store_type: StoreType,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum StoreType {
    #[default]
    Sqlite,
    FileStore,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletBirthday {
    BlockHeight(u64),
    Timestamp(u64),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
#[uniffi::export(Eq, Hash)]
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
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum, Display,
)]
#[uniffi::export(Display)]
pub enum WalletType {
    #[default]
    Hot,
    Cold,
    XpubOnly,
    WatchOnly,
}

#[uniffi::export]
impl WalletType {
    pub fn display_name(&self) -> String {
        match self {
            Self::Hot => "Hot",
            Self::Cold => "Cold",
            Self::XpubOnly => "Xpub Only",
            Self::WatchOnly => "Watch Only",
        }
        .to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum HardwareWalletMetadata {
    TapSigner(Arc<TapSigner>),
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    PartialEq,
    Display,
    uniffi::Enum,
    strum::EnumIter,
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
    pub fn new(name: impl Into<String>, fingerprint: Option<impl Into<Arc<Fingerprint>>>) -> Self {
        let network = Database::global().global_config.selected_network();
        let wallet_mode = Database::global().global_config.wallet_mode();

        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::random(),
            master_fingerprint: fingerprint.map(Into::into),
            origin: None,
            birthday: None,
            verified: false,
            network,
            fiat_or_btc: FiatOrBtc::Btc,
            selected_unit: Unit::default(),
            sensitive_visible: true,
            details_expanded: false,
            address_type: WalletAddressType::default(),
            wallet_type: WalletType::Hot,
            wallet_mode,
            hardware_metadata: None,
            show_labels: true,
            internal: InternalOnlyMetadata::default(),
            discovery_state: DiscoveryState::default(),
        }
    }

    pub fn new_for_hardware(
        id: WalletId,
        name: impl Into<String>,
        fingerprint: Option<Arc<Fingerprint>>,
    ) -> Self {
        let me = Self::new(name, fingerprint);

        Self { id, verified: true, wallet_type: WalletType::Cold, ..me }
    }

    pub fn new_imported_from_mnemonic(
        name: impl Into<String>,
        network: Network,
        fingerprint: impl Into<Arc<Fingerprint>>,
    ) -> Self {
        let mut me = Self::new(name, Some(fingerprint));
        me.discovery_state = DiscoveryState::StartedMnemonic;

        Self { network, verified: true, ..me }
    }

    /// Creates metadata for a wallet generated by Cove
    ///
    /// Cove-created wallets get a birthday because the app knows when the wallet first exists
    pub fn new_cove_created_wallet(
        name: impl Into<String>,
        fingerprint: Option<impl Into<Arc<Fingerprint>>>,
    ) -> Self {
        let mut me = Self::new(name, fingerprint);
        me.set_creation_birthday();
        me
    }

    pub fn set_creation_birthday(&mut self) {
        self.birthday = Some(created_wallet_birthday(self.network, self.wallet_mode));
    }

    pub fn set_tap_signer_setup_birthday(&mut self, card_birth_height: Option<u64>) {
        self.birthday =
            tap_signer_setup_birthday(self.network, self.wallet_mode, card_birth_height);
    }

    pub fn set_tap_signer_import_birthday(&mut self, card_birth_height: Option<u64>) {
        self.birthday = card_birth_height.map(WalletBirthday::BlockHeight);
    }

    pub fn matches_fingerprint(&self, fingerprint: Fingerprint) -> bool {
        let Some(wallet_fingerprint) = self.master_fingerprint.as_ref() else { return false };
        wallet_fingerprint.as_ref() == &fingerprint
    }

    pub fn preview_new() -> Self {
        Self {
            id: WalletId::preview_new_random(),
            name: "Test Wallet".to_string(),
            master_fingerprint: Some(Arc::new(Fingerprint::default())),
            origin: None,
            birthday: None,
            color: WalletColor::random(),
            verified: false,
            network: Network::Bitcoin,
            fiat_or_btc: FiatOrBtc::Btc,
            address_type: WalletAddressType::default(),
            selected_unit: Unit::default(),
            sensitive_visible: true,
            details_expanded: false,
            hardware_metadata: None,
            wallet_type: WalletType::Hot,
            wallet_mode: WalletMode::Main,
            show_labels: true,
            internal: InternalOnlyMetadata::default(),
            discovery_state: DiscoveryState::default(),
        }
    }
}

pub fn created_wallet_birthday(network: Network, mode: WalletMode) -> WalletBirthday {
    cheap_current_block_height(network, mode)
        .map(WalletBirthday::BlockHeight)
        .unwrap_or_else(|| WalletBirthday::Timestamp(current_timestamp()))
}

pub fn tap_signer_setup_birthday(
    network: Network,
    mode: WalletMode,
    card_birth_height: Option<u64>,
) -> Option<WalletBirthday> {
    cheap_current_block_height(network, mode)
        .map(WalletBirthday::BlockHeight)
        .or_else(|| card_birth_height.map(WalletBirthday::BlockHeight))
}

pub fn tap_signer_import_birthday(
    network: Network,
    mode: WalletMode,
    card_birth_height: Option<u64>,
) -> WalletBirthday {
    card_birth_height
        .map(WalletBirthday::BlockHeight)
        .or_else(|| tap_signer_setup_birthday(network, mode, None))
        .unwrap_or_else(|| WalletBirthday::Timestamp(current_timestamp()))
}

fn current_timestamp() -> u64 {
    jiff::Timestamp::now().as_second().try_into().unwrap_or(0)
}

fn cheap_current_block_height(network: Network, mode: WalletMode) -> Option<u64> {
    Database::global()
        .wallets
        .get_all(network, mode)
        .ok()?
        .into_iter()
        .filter_map(|wallet| wallet.internal.last_height_fetched.map(|height| height.block_height))
        .max()
}

impl InternalOnlyMetadata {
    pub fn last_seen_address_index(&self, addreses: &[AddressInfo]) -> Option<usize> {
        let address_index = self.address_index.as_ref()?;
        let address_list_hash = cove_util::calculate_hash(addreses);

        // different address list, return none
        if address_index.address_list_hash != address_list_hash {
            return None;
        }

        Some(address_index.last_seen_index as usize)
    }

    pub fn set_last_seen_address_index(&mut self, addreses: &[AddressInfo], index: usize) {
        let address_list_hash = cove_util::calculate_hash(addreses);

        self.address_index =
            Some(cove_types::AddressIndex { last_seen_index: index as u8, address_list_hash });
    }
}

impl HardwareWalletMetadata {
    pub const fn is_tap_signer(&self) -> bool {
        matches!(self, Self::TapSigner(_))
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

impl WalletColor {
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::WBeige,
            Self::WPastelBlue,
            Self::WPastelNavy,
            Self::WPastelRed,
            Self::WPastelYellow,
            Self::WPastelTeal,
            Self::Blue,
            Self::Green,
            Self::Orange,
            Self::Purple,
        ]
    }

    pub fn random() -> Self {
        let options = Self::defaults();

        use rand::RngExt;
        let random_index = rand::rng().random_range(0..options.len());
        options[random_index]
    }
}

#[uniffi::export]
fn default_wallet_colors() -> Vec<WalletColor> {
    WalletColor::defaults()
}

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

fn default_address_type() -> WalletAddressType {
    Default::default()
}

#[uniffi::export]
impl WalletMetadata {
    fn is_equal(&self, other: WalletMetadata) -> bool {
        self == &other
    }

    #[uniffi::method(name = "stableHash")]
    fn stable_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[uniffi::export]
fn wallet_metadata_preview() -> WalletMetadata {
    WalletMetadata::preview_new()
}

const fn file_store_default() -> StoreType {
    StoreType::FileStore
}

#[uniffi::export]
impl HardwareWalletMetadata {
    #[uniffi::method(name = "isTapSigner")]
    fn ffi_is_tap_signer(&self) -> bool {
        self.is_tap_signer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_birthday_deserializes_to_none() {
        let metadata = WalletMetadata::preview_new();
        let mut value = serde_json::to_value(metadata).unwrap();
        value.as_object_mut().unwrap().remove("birthday");

        let deserialized: WalletMetadata = serde_json::from_value(value).unwrap();

        assert_eq!(deserialized.birthday, None);
    }

    #[test]
    fn birthday_round_trips() {
        for birthday in
            [WalletBirthday::BlockHeight(800_000), WalletBirthday::Timestamp(1_700_000_000)]
        {
            let mut metadata = WalletMetadata::preview_new();
            metadata.birthday = Some(birthday);

            let deserialized: WalletMetadata =
                serde_json::from_value(serde_json::to_value(metadata).unwrap()).unwrap();

            assert_eq!(deserialized.birthday, Some(birthday));
        }
    }
}
