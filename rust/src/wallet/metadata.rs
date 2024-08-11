use crate::transaction::Unit;
use nid::Nanoid;
use rand::Rng as _;
use serde::{Deserialize, Serialize};

use crate::{database::Database, impl_default_for, network::Network, new_type};

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
            sensitive_visible: false,
        }
    }

    pub fn new_imported(name: impl Into<String>, network: Network) -> Self {
        Self {
            id: WalletId::new(),
            name: name.into(),
            color: WalletColor::Blue,
            verified: true,
            network,
            performed_full_scan: false,
            selected_unit: Unit::default(),
            selected_fiat_currency: default_fiat_currency(),
            sensitive_visible: false,
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
            sensitive_visible: false,
        }
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
