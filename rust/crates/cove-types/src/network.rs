use bdk_wallet::bitcoin;
use bdk_wallet::chain::bitcoin::params::Params;
use bitcoin::NetworkKind;
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    uniffi::Enum,
    derive_more::Display,
    strum::EnumIter,
    Serialize,
    Deserialize,
)]
#[uniffi::export(Display)]
pub enum Network {
    Bitcoin,
    Testnet,
    Testnet4,
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

impl From<Network> for u8 {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => 0,
            Network::Testnet => 1,
            Network::Testnet4 => 4,
            Network::Signet => 2,
        }
    }
}

#[allow(clippy::use_self)] // Self cannot be used when referring to external type
impl TryFrom<u8> for Network {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Bitcoin),
            1 => Ok(Self::Testnet),
            4 => Ok(Self::Testnet4),
            2 => Ok(Self::Signet),
            _ => Err(format!("Unknown network: {value}")),
        }
    }
}

#[allow(clippy::use_self)] // Self in TryFrom cannot be used with explicit type annotation
impl TryFrom<&str> for Network {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bitcoin" | "Bitcoin" => Ok(Self::Bitcoin),
            "testnet" | "Testnet" | "testnet3" | "Testnet3" => Ok(Self::Testnet),
            "testnet4" | "Testnet4" => Ok(Self::Testnet4),
            "signet" | "Signet" | "mutinynet" | "Mutinynet" => Ok(Self::Signet),
            _ => Err(format!("Unknown network: {value}")),
        }
    }
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => Self::Bitcoin,
            Network::Testnet => Self::Testnet,
            Network::Testnet4 => Self::Testnet4,
            Network::Signet => Self::Signet,
        }
    }
}

#[allow(clippy::fallible_impl_from)] // regtest is deliberately unsupported
impl From<bitcoin::Network> for Network {
    /// # Panics
    /// Panics if the network is not supported (Regtest)
    fn from(network: bitcoin::Network) -> Self {
        match network {
            bitcoin::Network::Bitcoin => Self::Bitcoin,
            bitcoin::Network::Testnet => Self::Testnet,
            bitcoin::Network::Testnet4 => Self::Testnet4,
            bitcoin::Network::Signet => Self::Signet,
            network @ bitcoin::Network::Regtest => panic!("unsupported network: {network:?}"),
        }
    }
}

impl From<Network> for Params {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => Self::MAINNET,
            Network::Testnet => Self::TESTNET3,
            Network::Testnet4 => Self::TESTNET4,
            Network::Signet => Self::SIGNET,
        }
    }
}

impl From<Network> for NetworkKind {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => Self::Main,
            Network::Testnet | Network::Testnet4 | Network::Signet => Self::Test,
        }
    }
}
