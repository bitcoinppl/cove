use std::sync::Arc;

use bdk_chain::spk_client::{SyncRequest, SyncResult};
use bdk_electrum::electrum_client::{self, ElectrumApi};
use bdk_esplora::{esplora_client, EsploraAsyncExt as _};
use bdk_wallet::{
    chain::{
        spk_client::{FullScanRequest, FullScanResult},
        ConfirmationBlockTime, TxGraph,
    },
    KeychainKind,
};
use tracing::debug;

use crate::node::Node;

use super::ApiType;

const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;

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
    EsploraConnectError(esplora_client::Error),

    #[error("failed to connect to node: {0}")]
    ElectrumConnectError(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    ElectrumScanError(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    EsploraScanError(Box<esplora_client::Error>),
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
                client
                    .get_height()
                    .await
                    .map_err(Error::EsploraConnectError)?;
            }

            NodeClient::Electrum(client) => {
                let client = client.clone();
                crate::unblock::run_blocking(move || client.inner.ping())
                    .await
                    .map_err(Error::ElectrumConnectError)?;
            }
        }

        Ok(())
    }

    pub async fn start_wallet_scan(
        &self,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        full_scan_request: FullScanRequest<KeychainKind>,
    ) -> Result<FullScanResult<KeychainKind>, Error> {
        if let NodeClient::Electrum(client) = self {
            debug!("start populate_tx_cache");
            let client = client.clone();
            let tx_graph = tx_graph.clone();
            crate::unblock::run_blocking(move || client.populate_tx_cache(tx_graph)).await;
            debug!("populate_tx_cache done");
        }

        let full_scan_result = match self {
            NodeClient::Esplora(client) => {
                debug!("starting esplora full scan");
                client
                    .full_scan(full_scan_request, STOP_GAP, BATCH_SIZE)
                    .await
                    .map_err(Error::EsploraScanError)?
            }

            NodeClient::Electrum(client) => {
                debug!("starting electrum full scan");
                let client = client.clone();
                crate::unblock::run_blocking(move || {
                    client.full_scan(full_scan_request, STOP_GAP, BATCH_SIZE, false)
                })
                .await
                .map_err(Error::ElectrumScanError)?
            }
        };

        Ok(full_scan_result)
    }

    pub async fn sync(
        &self,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        scan_request: SyncRequest,
    ) -> Result<SyncResult, Error> {
        if let NodeClient::Electrum(client) = self {
            debug!("start populate_tx_cache");
            let client = client.clone();
            let tx_graph = tx_graph.clone();
            crate::unblock::run_blocking(move || client.populate_tx_cache(tx_graph)).await;
            debug!("populate_tx_cache done");
        }

        let scan_result = match self {
            NodeClient::Esplora(client) => {
                debug!("starting esplora sync");
                client
                    .sync(scan_request, BATCH_SIZE)
                    .await
                    .map_err(Error::EsploraScanError)?
            }

            NodeClient::Electrum(client) => {
                debug!("starting electrum sync");
                let client = client.clone();

                crate::unblock::run_blocking(move || client.sync(scan_request, BATCH_SIZE, false))
                    .await
                    .map_err(Error::ElectrumScanError)?
            }
        };

        Ok(scan_result)
    }
}
