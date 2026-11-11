pub mod electrum;
pub mod esplora;

use std::collections::BTreeMap;
use std::sync::Arc;

use bdk_electrum::electrum_client;
use bdk_esplora::esplora_client;
use bdk_wallet::chain::{
    BlockId,
    bitcoin::Address,
    spk_client::{SyncRequest, SyncResponse},
};
use bdk_wallet::{
    KeychainKind,
    chain::{
        ConfirmationBlockTime, TxGraph,
        spk_client::{FullScanRequest, FullScanResponse},
    },
};
use bitcoin::{Transaction, Txid};
use cove_bdk_progressive_scan::ScanEvent;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::node::{Node, NodeConnectionIdentity};

use super::ApiType;

const ELECTRUM_BATCH_SIZE: usize = 10;
const ESPLORA_BATCH_SIZE: usize = 1;

/// A blockchain client bound to the node that created it
#[derive(Clone)]
pub struct NodeClient {
    connection_identity: NodeConnectionIdentity,
    backend: NodeClientBackend,
}

#[derive(Clone)]
enum NodeClientBackend {
    Esplora(self::esplora::EsploraClient),
    Electrum(self::electrum::ElectrumClient),
}

impl core::fmt::Debug for NodeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            NodeClientBackend::Esplora(_) => "Esplora",
            NodeClientBackend::Electrum(_) => "Electrum",
        };

        f.debug_struct("NodeClient")
            .field("connection_identity", &self.connection_identity)
            .field("backend", &backend)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to create node client: {0}")]
    CreateEsploraClient(EsploraError),

    #[error("failed to create node client: {0}")]
    CreateElectrumClient(electrum_client::Error),

    #[error("failed to connect to node: {0}")]
    EsploraConnect(EsploraError),

    #[error("failed to connect to node: {0}")]
    ElectrumConnect(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    ElectrumScan(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    EsploraScan(EsploraError),

    #[error("failed to complete progressive wallet scan: {0}")]
    ProgressiveScan(#[from] cove_bdk_progressive_scan::Error),

    #[error("failed to get a address: {0}")]
    EsploraAddress(EsploraError),

    #[error("failed to get a address: {0}")]
    ElectrumAddress(electrum_client::Error),

    #[error("failed to broadcast transaction: {0}")]
    EsploraBroadcast(EsploraError),

    #[error("failed to broadcast transaction: {0}")]
    ElectrumBroadcast(electrum_client::Error),

    #[error("failed to get transaction: {0}")]
    EsploraGetTransaction(EsploraError),

    #[error("failed to get transaction: {0}")]
    ElectrumGetTransaction(electrum_client::Error),
}

/// An esplora client error without the request URL
///
/// A node URL can carry an access token in its path or query, and node errors reach logs, so
/// the only way in is through `From`, which drops it
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct EsploraError(esplora_client::Error);

impl From<esplora_client::Error> for EsploraError {
    fn from(error: esplora_client::Error) -> Self {
        Self(cove_bdk_progressive_scan::esplora_error_without_url(error))
    }
}

impl From<Box<esplora_client::Error>> for EsploraError {
    fn from(error: Box<esplora_client::Error>) -> Self {
        Self::from(*error)
    }
}

impl Error {
    /// A stable name for the failure, safe to log
    ///
    /// The underlying client errors can quote the node URL the user entered, including any
    /// credentials in it
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CreateEsploraClient(_) => "create_esplora_client",
            Self::CreateElectrumClient(_) => "create_electrum_client",
            Self::EsploraConnect(_) => "esplora_connect",
            Self::ElectrumConnect(_) => "electrum_connect",
            Self::ElectrumScan(_) => "electrum_scan",
            Self::EsploraScan(_) => "esplora_scan",
            Self::ProgressiveScan(_) => "progressive_scan",
            Self::EsploraAddress(_) => "esplora_address",
            Self::ElectrumAddress(_) => "electrum_address",
            Self::EsploraBroadcast(_) => "esplora_broadcast",
            Self::ElectrumBroadcast(_) => "electrum_broadcast",
            Self::EsploraGetTransaction(_) => "esplora_get_transaction",
            Self::ElectrumGetTransaction(_) => "electrum_get_transaction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeClientOptions {
    pub batch_size: usize,
}

impl NodeClient {
    pub async fn new(node: &Node) -> Result<Self, Error> {
        let backend = match node.api_type {
            ApiType::Esplora => {
                let client = esplora::EsploraClient::new_from_node(node)?;
                NodeClientBackend::Esplora(client)
            }

            ApiType::Electrum => {
                let client = electrum::ElectrumClient::new_from_node(node).await?;
                NodeClientBackend::Electrum(client)
            }
        };

        Ok(Self { connection_identity: node.connection_identity(), backend })
    }

    pub async fn new_with_options(node: &Node, options: NodeClientOptions) -> Result<Self, Error> {
        let backend = match node.api_type {
            ApiType::Esplora => {
                let client = esplora::EsploraClient::new_from_node_and_options(node, options)?;
                NodeClientBackend::Esplora(client)
            }

            ApiType::Electrum => {
                let client =
                    electrum::ElectrumClient::new_from_node_and_options(node, options).await?;
                NodeClientBackend::Electrum(client)
            }
        };

        Ok(Self { connection_identity: node.connection_identity(), backend })
    }

    pub(crate) fn connection_identity(&self) -> &NodeConnectionIdentity {
        &self.connection_identity
    }

    pub async fn check_url(&self) -> Result<(), Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => {
                client.get_height().await?;
            }

            NodeClientBackend::Electrum(client) => {
                client.get_height().await?;
            }
        }

        Ok(())
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => {
                let height = client.get_height().await?;
                Ok(height as usize)
            }

            NodeClientBackend::Electrum(client) => {
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
        let full_scan_result = match &self.backend {
            NodeClientBackend::Esplora(client) => {
                debug!("starting esplora full scan");
                client.full_scan(full_scan_request, stop_gap).await?
            }

            NodeClientBackend::Electrum(client) => {
                debug!("starting electrum full scan");
                client.full_scan(full_scan_request, tx_graph, stop_gap).await?
            }
        };

        Ok(full_scan_result)
    }

    pub async fn start_progressive_wallet_scan(
        &self,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        full_scan_request: FullScanRequest<KeychainKind>,
        last_revealed_indices: BTreeMap<KeychainKind, u32>,
        stop_gap: usize,
        events: flume::Sender<ScanEvent<KeychainKind>>,
        cancel_token: CancellationToken,
    ) -> Result<FullScanResponse<KeychainKind>, Error> {
        let full_scan_result = match &self.backend {
            NodeClientBackend::Esplora(client) => {
                debug!("starting progressive esplora full scan");
                client
                    .progressive_full_scan(
                        full_scan_request,
                        last_revealed_indices,
                        stop_gap,
                        events,
                        cancel_token,
                    )
                    .await?
            }

            NodeClientBackend::Electrum(client) => {
                debug!("starting progressive electrum full scan");
                client
                    .progressive_full_scan(
                        full_scan_request,
                        tx_graph,
                        last_revealed_indices,
                        stop_gap,
                        events,
                        cancel_token,
                    )
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
        let scan_result = match &self.backend {
            NodeClientBackend::Esplora(client) => client.sync(scan_request).await?,
            NodeClientBackend::Electrum(client) => client.sync(scan_request, tx_graph).await?,
        };
        Ok(scan_result)
    }

    pub async fn get_confirmed_transaction(
        &self,
        txid: Arc<Txid>,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => client.get_confirmed_transaction(&txid).await,
            NodeClientBackend::Electrum(client) => client.get_confirmed_transaction(txid).await,
        }
    }

    /// Fetches a transaction from the mempool or chain; returns `None` if not found.
    pub async fn get_transaction(&self, txid: Txid) -> Result<Option<bitcoin::Transaction>, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => client.get_transaction(txid).await,
            NodeClientBackend::Electrum(client) => client.get_transaction(txid).await,
        }
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => client.get_block_id().await,
            NodeClientBackend::Electrum(client) => client.get_block_id().await,
        }
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }

            NodeClientBackend::Electrum(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }
        }
    }

    pub async fn broadcast_transaction(&self, txn: Transaction) -> Result<Txid, Error> {
        match &self.backend {
            NodeClientBackend::Esplora(client) => client.broadcast_transaction(txn).await,
            NodeClientBackend::Electrum(client) => client.broadcast_transaction(txn).await,
        }
    }
}
