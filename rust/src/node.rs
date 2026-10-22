pub mod client;
pub mod client_builder;

use crate::node_connect::{
    BITCOIN_ELECTRUM, NodeSelection, SIGNET_ESPLORA, TESTNET_ESPLORA, TESTNET4_ESPLORA,
};

use std::time::Duration;

use client::NodeClient;
use cove_tokio::FutureTimeoutExt as _;
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

/// How long a node check waits before reporting the node as unreachable
const CHECK_URL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to check node url: {0}")]
    CheckUrlError(#[from] client::Error),

    #[error("node did not respond within {} seconds", .0.as_secs())]
    CheckUrlTimedOut(Duration),
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

    /// Connects to the node and reads its height, failing when the node doesn't answer in time
    pub async fn check_url(&self) -> Result<(), Error> {
        // neither client sets a timeout, so a host that never answers would hold the settings
        // check until the OS gives up on the connection
        async {
            let client = NodeClient::new(self).await?;
            client.check_url().await?;

            Ok(())
        }
        .with_timeout(CHECK_URL_TIMEOUT)
        .await
        .map_err(|_| Error::CheckUrlTimedOut(CHECK_URL_TIMEOUT))?
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

    // regression for #928: a node that accepts the connection but never answers kept the
    // settings check loading forever; paused time lets the wait run without real delay
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn check_url_gives_up_on_a_node_that_never_responds() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api", listener.local_addr().unwrap());
        let node = Node::new_esplora("Silent".to_string(), url, Network::Signet);

        let result = node
            .check_url()
            .with_timeout(std::time::Duration::from_secs(600))
            .await
            .expect("check_url never returned for a node that never responds");

        assert!(result.is_err(), "a node that never responds must fail the check");
        drop(listener);
    }
}
