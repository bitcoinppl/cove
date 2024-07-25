use std::sync::Arc;

use bdk_electrum::{
    bdk_chain::TxGraph,
    electrum_client::{self, ElectrumApi},
};
use bdk_esplora::esplora_client;

use crate::{node::Node, wallet::Wallet};

use super::ApiType;

pub enum NodeClient {
    Esplora(esplora_client::r#async::AsyncClient),
    Electrum(Arc<bdk_electrum::BdkElectrumClient<electrum_client::Client>>),
}

impl core::fmt::Debug for NodeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeClient::Esplora(_) => write!(f, "Esplora"),
            NodeClient::Electrum(_) => write!(f, "Electrum"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to create node client: {0}")]
    CreateEsploraClientError(esplora_client::Error),

    #[error("failed to create node client: {0}")]
    CreateElectrumClientError(electrum_client::Error),

    #[error("failed to connect to node: {0}")]
    EsploraConnectError(#[from] esplora_client::Error),

    #[error("failed to connect to node: {0}")]
    ElectrumConnectError(#[from] electrum_client::Error),
}

impl NodeClient {
    pub async fn new_from_node(node: &Node) -> Result<Self, Error> {
        match node.api_type {
            ApiType::Esplora => {
                let client = esplora_client::Builder::new(&node.url)
                    .build_async()
                    .map_err(Error::CreateEsploraClientError)?;

                Ok(Self::Esplora(client))
            }

            ApiType::Electrum => {
                let url = node.url.strip_suffix('/').unwrap_or(&node.url);

                let client =
                    electrum_client::Client::new(url).map_err(Error::CreateElectrumClientError)?;

                let bdk_client = bdk_electrum::BdkElectrumClient::new(client);

                Ok(Self::Electrum(bdk_client.into()))
            }

            ApiType::Rpc => {
                // TODO: implement rpc check, with auth
                todo!()
            }
        }
    }

    pub async fn check_url(&self) -> Result<(), Error> {
        match self {
            NodeClient::Esplora(client) => {
                client.get_height().await?;
            }

            NodeClient::Electrum(client) => {
                let client = client.clone();
                crate::unblock::run_blocking(move || client.inner.ping()).await?;
            }
        }

        Ok(())
    }

    pub async fn start_wallet_scan(&self, wallet: &Wallet) -> Result<(), Error> {
        if let NodeClient::Electrum(client) = self {
            let client = client.clone();
            // crate::unblock::run_blocking(move || client.populate_tx_cache(wallet.tx_graph())).await;
        }

        Ok(())
    }
}
