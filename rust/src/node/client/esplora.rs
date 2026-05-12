use std::sync::Arc;

use bdk_esplora::{
    EsploraAsyncExt as _,
    esplora_client::{self, r#async::AsyncClient},
};
use bdk_wallet::chain::{
    bitcoin::Address,
    spk_client::{FullScanRequest, FullScanResponse, SyncRequest, SyncResponse},
};
use bdk_wallet::{KeychainKind, chain::BlockId};
use bitcoin::Txid;
use tap::TapFallible as _;
use tracing::debug;

use crate::node::Node;

use super::{ESPLORA_BATCH_SIZE, Error, NodeClientOptions};

#[derive(Debug, Clone)]
pub struct EsploraClient {
    client: Arc<AsyncClient>,
    options: NodeClientOptions,
}

impl EsploraClient {
    pub fn new(client: Arc<AsyncClient>) -> Self {
        Self::new_with_options(
            client,
            NodeClientOptions { batch_size: ESPLORA_BATCH_SIZE, ..NodeClientOptions::default() },
        )
    }

    pub fn new_from_node(node: &Node) -> Result<Self, Error> {
        let options = NodeClientOptions::default();
        Self::new_from_node_and_options(node, &options)
    }

    pub fn new_from_node_and_options(
        node: &Node,
        options: &NodeClientOptions,
    ) -> Result<Self, Error> {
        debug!(
            api_type = "esplora",
            tor_enabled = options.use_tor,
            tor_mode = ?options.tor_mode,
            batch_size = options.batch_size,
            "creating esplora client from node and options"
        );
        let mut builder = esplora_client::Builder::new(&node.url);

        if options.use_tor {
            debug!(
                api_type = "esplora",
                tor_enabled = true,
                tor_mode = ?options.tor_mode,
                "esplora using socks proxy"
            );
            let proxy =
                format!("socks5h://{}:{}", options.tor_external_host, options.tor_external_port);
            builder = builder.proxy(&proxy);
        } else {
            debug!("esplora connecting without tor proxy");
        }

        let client = builder.build_async().map_err(Error::CreateEsploraClient)?.into();

        Ok(Self::new_with_options(client, options.clone()))
    }

    pub const fn new_with_options(client: Arc<AsyncClient>, options: NodeClientOptions) -> Self {
        Self { client, options }
    }

    pub async fn get_height(&self) -> Result<u32, Error> {
        self.client
            .get_height()
            .await
            .tap_err(|error| tracing::error!("Failed to get height: {error:?}"))
            .map_err(Error::EsploraConnect)
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        let height = self.get_height().await?;
        let hash = self
            .client
            .get_block_hash(height)
            .await
            .tap_err(|e| tracing::error!("Failed to get block hash: {e:?}"))
            .map_err(Error::EsploraConnect)?;

        Ok(BlockId { height, hash })
    }

    pub async fn full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        stop_gap: usize,
    ) -> Result<FullScanResponse<KeychainKind>, Error> {
        self.client
            .full_scan(request, stop_gap, self.options.batch_size)
            .await
            .map_err(Error::EsploraScan)
    }

    pub async fn sync(
        &self,
        request: SyncRequest<(KeychainKind, u32)>,
    ) -> Result<SyncResponse, Error> {
        debug!("starting esplora sync, batch size: {}", self.options.batch_size);

        self.client.sync(request, self.options.batch_size).await.map_err(Error::EsploraScan)
    }

    pub async fn get_confirmed_transaction(
        &self,
        txid: &Txid,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        let status = self.client.get_tx_status(txid).await.map_err(Error::EsploraGetTransaction)?;
        if !status.confirmed {
            return Ok(None);
        }

        let tx = self.client.get_tx(txid).await.map_err(Error::EsploraGetTransaction)?;
        Ok(tx)
    }

    pub async fn broadcast_transaction(&self, txn: bitcoin::Transaction) -> Result<Txid, Error> {
        self.client.broadcast(&txn).await.map_err(Error::EsploraBroadcast)?;

        Ok(txn.compute_txid())
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        let stats = self.client.get_address_stats(&address).await.map_err(Error::EsploraAddress)?;

        Ok(stats.chain_stats.tx_count > 0)
    }
}
