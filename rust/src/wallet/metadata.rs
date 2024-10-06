use std::hash::Hash;

use crate::transaction::Unit;
use macros::{impl_default_for, new_type};
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};

use crate::{database::Database, network::Network};

use super::AddressInfo;

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
    pub selected_unit: Unit,
    #[serde(default = "default_fiat_currency")]
    pub selected_fiat_currency: String,
    #[serde(default = "default_true")]
    pub sensitive_visible: bool,
    #[serde(default = "default_false")]
    pub details_expanded: bool,
    #[serde(default)]
    pub wallet_type: WalletType,

    // internal only metadata, don't use in the UI
    // note: maybe better to use a separate table for this
    #[serde(default)]
    pub internal: InternalOnlyMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct InternalOnlyMetadata {
    pub address_index: Option<AddressIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct AddressIndex {
    pub last_seen_index: u8,
    pub address_list_hash: u64,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Enum,
)]
pub enum WalletType {
    #[default]
    Hot,
    Cold,
}

mod ffi {
    use super::*;

    #[uniffi::export]
    pub fn wallet_metadata_preview() -> WalletMetadata {
        WalletMetadata::preview_new()
    }
}

impl WalletMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        let network = Database::global().global_config.selected_network();

        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::Blue,
            verified: false,
            network,
            performed_full_scan: false,
            selected_unit: Unit::default(),
            selected_fiat_currency: default_fiat_currency(),
            sensitive_visible: true,
            details_expanded: false,
            wallet_type: WalletType::Hot,
            internal: InternalOnlyMetadata::default(),
        }
    }

    pub fn new_with_id(id: WalletId, name: impl Into<String>) -> Self {
        let me = Self::new(name);

        Self {
            id,
            verified: true,
            wallet_type: WalletType::Cold,
            ..me
        }
    }

    pub fn new_imported(name: impl Into<String>, network: Network) -> Self {
        let me = Self::new(name);

        Self {
            network,
            verified: true,
            ..me
        }
    }

    pub fn preview_new() -> Self {
        Self {
            id: WalletId::preview_new(),
            name: "Test Wallet".to_string(),
            color: WalletColor::Blue,
            verified: false,
            network: Network::Bitcoin,
            performed_full_scan: false,
            selected_unit: Unit::default(),
            selected_fiat_currency: default_fiat_currency(),
            sensitive_visible: true,
            details_expanded: false,
            wallet_type: WalletType::Hot,
            internal: InternalOnlyMetadata::default(),
        }
    }

    pub fn internal(&mut self) -> &mut InternalOnlyMetadata {
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
    Custom { r: u8, g: u8, b: u8 },
}

impl WalletColor {
    pub fn random() -> Self {
        let options = [
            WalletColor::Red,
            WalletColor::Blue,
            WalletColor::Green,
            WalletColor::Yellow,
            WalletColor::Orange,
            WalletColor::Purple,
            WalletColor::Pink,
        ];

        let random_index = rand::thread_rng().gen_range(0..options.len());
        options[random_index]
    }
}

fn default_fiat_currency() -> String {
    "USD".to_string()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    true
}
