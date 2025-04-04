use bdk_chain::bitcoin::params::Params;
use bdk_wallet::bitcoin;
use bitcoin::NetworkKind;
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    Eq,
    PartialEq,
    uniffi::Enum,
    derive_more::Display,
    strum::EnumIter,
    Serialize,
    Deserialize,
)]
pub enum Network {
    Bitcoin,
    Testnet,
    Signet,
}

use strum::IntoEnumIterator;

#[uniffi::export]
fn network_to_string(network: Network) -> String {
    network.to_string()
}

#[uniffi::export]
fn all_networks() -> Vec<Network> {
    Network::iter().collect()
}

impl TryFrom<&str> for Network {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bitcoin" | "Bitcoin" => Ok(Network::Bitcoin),
            "testnet" | "Testnet" => Ok(Network::Testnet),
            "signet" | "Signet" => Ok(Network::Signet),
            "mutinynet" | "Mutinynet" => Ok(Network::Signet),
            _ => Err(format!("Unknown network: {}", value)),
        }
    }
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => bitcoin::Network::Bitcoin,
            Network::Testnet => bitcoin::Network::Testnet,
            Network::Signet => bitcoin::Network::Signet,
        }
    }
}

impl From<bitcoin::Network> for Network {
    fn from(network: bitcoin::Network) -> Self {
        match network {
            bitcoin::Network::Bitcoin => Network::Bitcoin,
            bitcoin::Network::Testnet => Network::Testnet,
            bitcoin::Network::Signet => Network::Signet,
            network => panic!("unsupported network: {network:?}"),
        }
    }
}

impl From<Network> for Params {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => Params::MAINNET,
            Network::Testnet => Params::TESTNET3,
            Network::Signet => Params::SIGNET,
        }
    }
}

impl From<Network> for NetworkKind {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => NetworkKind::Main,
            _ => NetworkKind::Test,
        }
    }
}
