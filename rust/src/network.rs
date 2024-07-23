use bdk_wallet::bitcoin;
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
}

mod ffi {
    use super::Network;
    use strum::IntoEnumIterator;

    #[uniffi::export]
    pub fn network_to_string(network: Network) -> String {
        network.to_string()
    }

    #[uniffi::export]
    pub fn all_networks() -> Vec<Network> {
        Network::iter().collect()
    }
}

impl TryFrom<&str> for Network {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bitcoin" | "Bitcoin" => Ok(Network::Bitcoin),
            "testnet" | "Testnet" => Ok(Network::Testnet),
            _ => Err(format!("Unknown network: {}", value)),
        }
    }
}

impl From<Network> for bitcoin::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => bitcoin::Network::Bitcoin,
            Network::Testnet => bitcoin::Network::Testnet,
        }
    }
}

impl From<bitcoin::Network> for Network {
    fn from(network: bitcoin::Network) -> Self {
        match network {
            bitcoin::Network::Bitcoin => Network::Bitcoin,
            bitcoin::Network::Testnet => Network::Testnet,
            network => panic!("unsupported network: {network:?}"),
        }
    }
}
