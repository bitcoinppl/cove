pub mod client;
pub mod client_builder;

use crate::node_connect::{
    BITCOIN_ELECTRUM, NodeSelection, SIGNET_ESPLORA, TESTNET_ESPLORA, TESTNET4_ESPLORA,
};
use crate::tor::RouteError;

use std::time::Duration;

use client::NodeClient;
use cove_http::NetworkRoute;
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

/// What a connection to a node depends on, so a cached client is reused only while it matches
///
/// The route is part of it: turning Tor on or off changes every node's identity, so each
/// cached client is dropped and rebuilt on the new route instead of keeping its old connection
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct NodeConnectionIdentity {
    network: Network,
    api_type: ApiType,
    url: String,
    /// `None` when no route is available, which matches no client
    route: Option<NetworkRoute>,
}

/// How long a node check waits before reporting the node as unreachable
const CHECK_URL_TIMEOUT: Duration = Duration::from_secs(20);

/// A node check over Tor, which can include building a circuit to an onion service
const TOR_CHECK_URL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to check node url: {0}")]
    CheckUrlError(#[from] client::Error),

    #[error("node did not respond within {} seconds", .0.as_secs())]
    CheckUrlTimedOut(Duration),
}

impl Node {
    /// The identity a client built for this node on the current route would have
    pub(crate) fn connection_identity(&self) -> NodeConnectionIdentity {
        self.connection_identity_on(self.route().ok())
    }

    pub(crate) fn connection_identity_on(
        &self,
        route: Option<NetworkRoute>,
    ) -> NodeConnectionIdentity {
        NodeConnectionIdentity {
            network: self.network,
            api_type: self.api_type,
            url: self.url.clone(),
            route,
        }
    }

    /// How long a check of this node may take: `direct` off Tor, and longer on Tor, where the
    /// first circuit to a node or an onion service takes several round trips
    pub(crate) fn check_timeout(&self, direct: Duration) -> Duration {
        match self.route() {
            Ok(route) if route.is_tor() => TOR_CHECK_URL_TIMEOUT,
            _ => direct,
        }
    }

    /// The route a new connection to this node takes
    pub(crate) fn route(&self) -> Result<NetworkRoute, RouteError> {
        let host =
            url::Url::parse(&self.url).ok().and_then(|url| url.host_str().map(str::to_owned));
        crate::tor::route_for_host(host.as_deref().unwrap_or_default())
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
        // neither client sets a timeout on a direct route, so a host that never answers would
        // hold the settings check until the OS gives up on the connection
        let timeout = self.check_timeout(CHECK_URL_TIMEOUT);

        async {
            let client = NodeClient::new(self).await?;
            client.check_url().await?;

            Ok(())
        }
        .with_timeout(timeout)
        .await
        .map_err(|_| Error::CheckUrlTimedOut(timeout))?
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
