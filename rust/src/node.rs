use crate::{
    network::Network,
    node_connect::{NodeSelection, BITCOIN_ESPLORA},
};

use bdk_electrum::electrum_client::{self, ElectrumApi};
use bdk_esplora::esplora_client;
use eyre::Context as _;

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

    pub async fn check_url(&self) -> eyre::Result<()> {
        match self.api_type {
            ApiType::Esplora => {
                let client = esplora_client::Builder::new(&self.url)
                    .build_async()
                    .wrap_err("failed to create esplora client")?;

                client
                    .get_height()
                    .await
                    .wrap_err("failed to connect to esplora node")?;

                Ok(())
            }

            ApiType::Electrum => {
                println!("checking electrum node at {:?}", self.url);
                let url = self.url.strip_suffix('/').unwrap_or(&self.url);

                let client = electrum_client::Client::new(url)
                    .wrap_err("failed to create electrum client")?;

                crate::unblock::run_blocking(move || client.ping())
                    .await
                    .wrap_err("failed to connect to electrum node")?;

                Ok(())
            }

            ApiType::Rpc => {
                // TODO: implement rpc check, with auth
                todo!()
            }
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
