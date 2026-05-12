pub mod electrum;
pub mod esplora;

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
use cove_util::ResultExt as _;
use tracing::{debug, info, warn};

use crate::{
    database::Database,
    node::{Node, TorMode},
    tor_runtime,
};

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
            Self::Esplora(_) => write!(f, "Esplora"),
            Self::Electrum(_) => write!(f, "Electrum"),
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

    #[error("failed to resolve tor endpoint: {0}")]
    ResolveTorEndpoint(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeClientOptions {
    pub batch_size: usize,
    pub use_tor: bool,
    pub tor_mode: TorMode,
    pub tor_external_host: String,
    pub tor_external_port: u16,
}

impl Default for NodeClientOptions {
    fn default() -> Self {
        Self {
            batch_size: ESPLORA_BATCH_SIZE,
            use_tor: false,
            tor_mode: TorMode::BuiltIn,
            tor_external_host: "127.0.0.1".to_string(),
            tor_external_port: 9050,
        }
    }
}

impl NodeClientOptions {
    pub async fn resolve_tor_endpoint(mut self) -> Result<Self, Error> {
        info!(
            use_tor = self.use_tor,
            tor_mode = ?self.tor_mode,
            tor_external_host = %self.tor_external_host,
            tor_external_port = self.tor_external_port,
            "resolving tor endpoint"
        );

        if !self.use_tor {
            info!("tor disabled; skipping tor endpoint resolution");
            return Ok(self);
        }

        match self.tor_mode {
            TorMode::External | TorMode::Orbot => {
                if self.tor_external_host.is_empty() {
                    warn!("tor external host empty; defaulting to 127.0.0.1");
                    self.tor_external_host = "127.0.0.1".to_string();
                }
                if self.tor_external_port == 0 {
                    warn!("tor external port missing; defaulting to 9050");
                    self.tor_external_port = 9050;
                }
                info!(
                    tor_mode = ?self.tor_mode,
                    tor_external_host = %self.tor_external_host,
                    tor_external_port = self.tor_external_port,
                    "using configured tor endpoint"
                );
            }
            TorMode::BuiltIn => {
                info!("tor mode is built-in; requesting Arti socks endpoint");
                let endpoint = tor_runtime::built_in_socks_endpoint()
                    .await
                    .map_err_str(Error::ResolveTorEndpoint)?;
                self.tor_external_host = endpoint.ip().to_string();
                self.tor_external_port = endpoint.port();
                info!(%endpoint, "resolved built-in tor socks endpoint");
            }
        }

        Ok(self)
    }
}

impl NodeClient {
    pub async fn new(node: &Node) -> Result<Self, Error> {
        let db = Database::global();
        let config = db.global_config();
        let tor_external_host = config
            .tor_external_host()
            .ok()
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let batch_size = match node.api_type {
            ApiType::Electrum => ELECTRUM_BATCH_SIZE,
            ApiType::Esplora | ApiType::Rpc => ESPLORA_BATCH_SIZE,
        };

        let options = NodeClientOptions {
            batch_size,
            use_tor: config.use_tor(),
            tor_mode: config.tor_mode().unwrap_or_default(),
            tor_external_host,
            tor_external_port: config.tor_external_port(),
        };

        info!(node = %node.url, api_type = ?node.api_type, options = ?options, "creating node client with db-backed options");

        Self::new_with_options(node, options).await
    }

    pub async fn try_from_builder(builder: &NodeClientBuilder) -> Result<Self, Error> {
        let node_client = Self::new_with_options(&builder.node, builder.options.clone()).await?;
        Ok(node_client)
    }

    pub async fn new_with_options(node: &Node, options: NodeClientOptions) -> Result<Self, Error> {
        info!(node = %node.url, api_type = ?node.api_type, options = ?options, "creating node client with explicit options");
        let options = options.resolve_tor_endpoint().await?;

        info!(node = %node.url, api_type = ?node.api_type, resolved_options = ?options, "node client options resolved");

        match node.api_type {
            ApiType::Esplora => {
                let client = esplora::EsploraClient::new_from_node_and_options(node, &options)?;
                Ok(Self::Esplora(client))
            }

            ApiType::Electrum => {
                let client =
                    electrum::ElectrumClient::new_from_node_and_options(node, &options).await?;
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
            Self::Esplora(client) => {
                client.get_height().await?;
            }

            Self::Electrum(client) => {
                client.get_height().await?;
            }
        }

        Ok(())
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        match self {
            Self::Esplora(client) => {
                let height = client.get_height().await?;
                Ok(height as usize)
            }

            Self::Electrum(client) => {
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
            Self::Esplora(client) => {
                debug!("starting esplora full scan");
                client.full_scan(full_scan_request, stop_gap).await?
            }

            Self::Electrum(client) => {
                debug!("starting electrum full scan");
                client.full_scan(full_scan_request, tx_graph, stop_gap).await?
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
            Self::Esplora(client) => client.sync(scan_request).await?,
            Self::Electrum(client) => client.sync(scan_request, tx_graph).await?,
        };
        Ok(scan_result)
    }

    pub async fn get_confirmed_transaction(
        &self,
        txid: Arc<Txid>,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        match self {
            Self::Esplora(client) => client.get_confirmed_transaction(&txid).await,
            Self::Electrum(client) => client.get_confirmed_transaction(txid).await,
        }
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        match self {
            Self::Esplora(client) => client.get_block_id().await,
            Self::Electrum(client) => client.get_block_id().await,
        }
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        match self {
            Self::Esplora(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }

            Self::Electrum(client) => {
                let address = client.check_address_for_txn(address).await?;
                Ok(address)
            }
        }
    }

    pub async fn broadcast_transaction(&self, txn: Transaction) -> Result<Txid, Error> {
        match self {
            Self::Esplora(client) => client.broadcast_transaction(txn).await,
            Self::Electrum(client) => client.broadcast_transaction(txn).await,
        }
    }
}
