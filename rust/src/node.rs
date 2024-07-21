use crate::{
    network::Network,
    node_connect::{NodeSelection, BITCOIN_ESPLORA},
};

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    Eq,
    PartialEq,
    derive_more::Display,
    strum::EnumIter,
    uniffi::Enum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ApiType {
    Esplora,
    Electrum,
    Rpc,
}

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, uniffi::Record, serde::Serialize, serde::Deserialize,
)]
pub struct Node {
    pub name: String,
    pub network: Network,
    pub api_type: ApiType,
    pub url: String,
}

impl Default for Node {
    fn default() -> Self {
        let (name, url) = BITCOIN_ESPLORA;

        Self {
            name: name.to_string(),
            network: Network::Bitcoin,
            api_type: ApiType::Esplora,
            url: url.to_string(),
        }
    }
}

impl Node {
    pub fn new_electrum(name: String, url: String, network: Network) -> Self {
        Self {
            name,
            network,
            api_type: ApiType::Electrum,
            url,
        }
    }

    pub fn new_esplora(name: String, url: String, network: Network) -> Self {
        Self {
            name,
            network,
            api_type: ApiType::Esplora,
            url,
        }
    }
}

impl From<NodeSelection> for Node {
    fn from(node: NodeSelection) -> Self {
        match node {
            NodeSelection::Preset(node) => node,
            NodeSelection::Custom(node) => node,
        }
    }
}
