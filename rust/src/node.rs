pub mod client;
pub mod client_builder;

use crate::node_connect::{
    BITCOIN_ELECTRUM, NodeSelection, SIGNET_ESPLORA, TESTNET_ELECTRUM, TESTNET4_ESPLORA,
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
pub struct TorConfig {
    /// Whether TOR proxy is enabled
    pub enabled: bool,
    /// SOCKS5 proxy address (e.g. "127.0.0.1:9050")
    pub proxy_address: String,
}

impl Default for TorConfig {
    fn default() -> Self {
        Self { enabled: false, proxy_address: "127.0.0.1:9050".to_string() }
    }
}

impl TorConfig {
    /// Returns the SOCKS5 proxy URL with remote DNS resolution (socks5h).
    /// The 'h' variant resolves hostnames through the proxy, which is
    /// required for .onion addresses that cannot be resolved locally.
    pub fn socks5_url(&self) -> String {
        format!("socks5h://{}", self.proxy_address)
    }
}

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, uniffi::Record, serde::Serialize, serde::Deserialize,
)]
pub struct Node {
    pub name: String,
    pub network: Network,
    pub api_type: ApiType,
    pub url: String,
    #[serde(default)]
    pub tor: TorConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to check node url: {0}")]
    CheckUrlError(#[from] client::Error),
}

impl Node {
    pub fn default(network: Network) -> Self {
        match network {
            Network::Bitcoin => {
                let (name, url) = BITCOIN_ELECTRUM[0];
                Self::new_electrum(name.to_string(), url.to_string(), network)
            }
            Network::Testnet => {
                let (name, url) = TESTNET_ELECTRUM[0];
                Self::new_electrum(name.to_string(), url.to_string(), network)
            }

            Network::Signet => {
                let (name, url) = SIGNET_ESPLORA[0];
                Self::new_esplora(name.to_string(), url.to_string(), network)
            }

            Network::Testnet4 => {
                let (name, url) = TESTNET4_ESPLORA[0];
                Self::new_esplora(name.to_string(), url.to_string(), network)
            }
        }
    }

    pub fn new_electrum(name: String, url: String, network: Network) -> Self {
        Self { name, network, api_type: ApiType::Electrum, url, tor: TorConfig::default() }
    }

    pub fn new_esplora(name: String, url: String, network: Network) -> Self {
        Self { name, network, api_type: ApiType::Esplora, url, tor: TorConfig::default() }
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
