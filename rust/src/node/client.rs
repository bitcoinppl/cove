pub mod electrum;
pub mod esplora;

use std::sync::Arc;

use bdk_chain::{
    bitcoin::Address,
    spk_client::{SyncRequest, SyncResponse},
};
use bdk_electrum::electrum_client;
use bdk_esplora::esplora_client;
use bdk_wallet::{
    KeychainKind,
    chain::{
        ConfirmationBlockTime, TxGraph,
        spk_client::{FullScanRequest, FullScanResponse},
    },
};
use bitcoin::{Transaction, Txid};
use tracing::debug;

use crate::node::Node;

use super::{ApiType, client_builder::NodeClientBuilder};

const ELECTRUM_BATCH_SIZE: usize = 10;
const ESPLORA_BATCH_SIZE: usize = 1;

#[derive(Clone)]
pub enum NodeClient {
    Esplora(self::esplora::EsploraClient),
    Electrum(self::electrum::ElectrumClient),
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
    CreateEsploraClient(esplora_client::Error),

    #[error("failed to create node client: {0}")]
    CreateElectrumClient(electrum_client::Error),

    #[error("failed to connect to node: {0}")]
    EsploraConnect(esplora_client::Error),

    #[error("failed to connect to node: {0}")]
    ElectrumConnect(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    ElectrumScan(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    EsploraScan(Box<esplora_client::Error>),

    #[error("failed to get a address: {0}")]
    EsploraAddress(esplora_client::Error),

    #[error("failed to get a address: {0}")]
    ElectrumAddress(electrum_client::Error),

    #[error("failed to broadcast transaction: {0}")]
    EsploraBroadcast(esplora_client::Error),

    #[error("failed to broadcast transaction: {0}")]
    ElectrumBroadcast(electrum_client::Error),

    #[error("failed to get transaction: {0}")]
    EsploraGetTransaction(esplora_client::Error),

    #[error("failed to get transaction: {0}")]
    ElectrumGetTransaction(electrum_client::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeClientOptions {
    pub batch_size: usize,
}

impl NodeClient {
    pub async fn new(node: &Node) -> Result<Self, Error> {
        match node.api_type {
            ApiType::Esplora => {
                let client = esplora::EsploraClient::new_from_node(node)?;
                Ok(Self::Esplora(client))
            }

            ApiType::Electrum => {
                let client = electrum::ElectrumClient::new_from_node(node)?;
                Ok(Self::Electrum(client))
            }

            ApiType::Rpc => {
                // TODO: implement rpc check, with auth
                todo!()
            }
        }
    }

    pub async fn try_from_builder(builder: &NodeClientBuilder) -> Result<Self, Error> {
        let node_client = NodeClient::new_with_options(&builder.node, builder.options).await?;
        Ok(node_client)
    }

    pub async fn new_with_options(node: &Node, options: NodeClientOptions) -> Result<Self, Error> {
        match node.api_type {
            ApiType::Esplora => {
                let client = esplora::EsploraClient::new_from_node_and_options(node, options)?;
                Ok(Self::Esplora(client))
            }

            ApiType::Electrum => {
                let client = electrum::ElectrumClient::new_from_node_and_options(node, options)?;
                Ok(Self::Electrum(client))
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
                client.get_height().await?;
            }
        }

        Ok(())
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        match self {
            NodeClient::Esplora(client) => {
                let height = client.get_height().await?;
                Ok(height as usize)
            }

            NodeClient::Electrum(client) => {
                let height = client.get_height().await?;
                Ok(height)
            }
        }
    }

    pub async fn start_wallet_scan(
        &self,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        full_scan_request: FullScanRequest<KeychainKind>,
        stop_gap: usize,
    ) -> Result<FullScanResponse<KeychainKind>, Error> {
        let full_scan_result = match self {
            NodeClient::Esplora(client) => {
                debug!("starting esplora full scan");
                client.full_scan(full_scan_request, stop_gap).await?
            }

            NodeClient::Electrum(client) => {
                debug!("starting electrum full scan");
                client
                    .full_scan(full_scan_request, tx_graph, stop_gap)
                    .await?
            }
        };

        Ok(full_scan_result)
    }

    pub async fn sync(
        &self,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        scan_request: SyncRequest<(KeychainKind, u32)>,
    ) -> Result<SyncResponse, Error> {
        let scan_result = match self {
            NodeClient::Esplora(client) => client.sync(scan_request).await?,
            NodeClient::Electrum(client) => client.sync(scan_request, tx_graph).await?,
        };

        Ok(scan_result)
    }

    pub async fn get_transaction(
        &self,
        txid: Arc<Txid>,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        match self {
            NodeClient::Esplora(client) => client.get_transaction(&txid).await,
            NodeClient::Electrum(client) => client.get_transaction(txid).await,
        }
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        match self {
            NodeClient::Esplora(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }

            NodeClient::Electrum(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }
        }
    }

    pub async fn broadcast_transaction(&self, txn: Transaction) -> Result<Txid, Error> {
        match self {
            NodeClient::Esplora(client) => client.broadcast_transaction(txn).await,
            NodeClient::Electrum(client) => client.broadcast_transaction(txn).await,
        }
    }
}
