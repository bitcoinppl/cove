pub mod client;
pub mod client_builder;

use crate::node_connect::{
    BITCOIN_ELECTRUM, NodeSelection, SIGNET_ESPLORA, TESTNET_ESPLORA, TESTNET4_ESPLORA,
};

use client::NodeClient;
use cove_types::network::Network;

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

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct NodeConnectionIdentity {
    network: Network,
    api_type: ApiType,
    url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to check node url: {0}")]
    CheckUrlError(#[from] client::Error),
}

impl Node {
    pub(crate) fn connection_identity(&self) -> NodeConnectionIdentity {
        NodeConnectionIdentity {
            network: self.network,
            api_type: self.api_type,
            url: self.url.clone(),
        }
    }

    pub fn default(network: Network) -> Self {
        match network {
            Network::Bitcoin => {
                let (name, url) = BITCOIN_ELECTRUM[0];

                Self {
                    name: name.to_string(),
                    network,
                    api_type: ApiType::Electrum,
                    url: url.to_string(),
                }
            }
            Network::Testnet => {
                let (name, url) = TESTNET_ESPLORA[0];
                Self {
                    name: name.to_string(),
                    network,
                    api_type: ApiType::Electrum,
                    url: url.to_string(),
                }
            }

            Network::Signet => {
                let (name, url) = SIGNET_ESPLORA[0];
                Self {
                    name: name.to_string(),
                    network,
                    api_type: ApiType::Esplora,
                    url: url.to_string(),
                }
            }

            Network::Testnet4 => {
                let (name, url) = TESTNET4_ESPLORA[0];
                Self {
                    name: name.to_string(),
                    network,
                    api_type: ApiType::Esplora,
                    url: url.to_string(),
                }
            }
        }
    }

    pub const fn new_electrum(name: String, url: String, network: Network) -> Self {
        Self { name, network, api_type: ApiType::Electrum, url }
    }

    pub const fn new_esplora(name: String, url: String, network: Network) -> Self {
        Self { name, network, api_type: ApiType::Esplora, url }
    }

    pub async fn check_url(&self) -> Result<(), Error> {
        let client = NodeClient::new(self).await?;
        client.check_url().await?;

        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Node {
        Node::new_esplora(
            "Primary".to_string(),
            "https://example.com/api".to_string(),
            Network::Bitcoin,
        )
    }

    #[test]
    fn display_name_does_not_change_connection_identity() {
        let node = node();
        let renamed = Node { name: "Renamed".to_string(), ..node.clone() };

        assert_ne!(node, renamed);
        assert_eq!(node.connection_identity(), renamed.connection_identity());
    }

    #[test]
    fn connection_fields_change_connection_identity() {
        let node = node();
        let different_url = Node { url: "https://other.example/api".to_string(), ..node.clone() };
        let different_api = Node { api_type: ApiType::Electrum, ..node.clone() };
        let different_network = Node { network: Network::Signet, ..node.clone() };

        assert_ne!(node.connection_identity(), different_url.connection_identity());
        assert_ne!(node.connection_identity(), different_api.connection_identity());
        assert_ne!(node.connection_identity(), different_network.connection_identity());
    }
}
