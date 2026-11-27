use std::{collections::BTreeMap, sync::Arc};

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
use cove_bdk_progressive_scan::{ProgressiveScanner, ScanEvent};
use tap::TapFallible as _;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use cove_http::NetworkRoute;

use crate::node::Node;

use super::{ESPLORA_BATCH_SIZE, Error, NodeClientOptions};

#[derive(Debug, Clone)]
pub struct EsploraClient {
    client: Arc<AsyncClient>,
    options: NodeClientOptions,
}

impl EsploraClient {
    pub(crate) const fn default_options() -> NodeClientOptions {
        NodeClientOptions { batch_size: ESPLORA_BATCH_SIZE }
    }

    /// A client for `node` on `route`, through the SOCKS proxy when the route is Tor
    #[allow(clippy::disallowed_methods, reason = "the one place that builds Esplora clients")]
    pub fn new_from_node(
        node: &Node,
        options: NodeClientOptions,
        route: &NetworkRoute,
    ) -> Result<Self, Error> {
        let builder = esplora_client::Builder::new(&node.url);

        // `socks5h` has the proxy resolve the node's name, see `SocksProxy::url`
        let builder = match route {
            NetworkRoute::Direct => builder,
            NetworkRoute::Tor(proxy) => {
                builder.proxy(&proxy.url()).timeout(route.timeouts().request.as_secs())
            }
        };

        let client =
            builder.build_async().map_err(|error| Error::CreateEsploraClient(error.into()))?;

        Ok(Self::new_with_options(client.into(), options))
    }

    pub const fn new_with_options(client: Arc<AsyncClient>, options: NodeClientOptions) -> Self {
        Self { client, options }
    }

    pub async fn get_height(&self) -> Result<u32, Error> {
        self.client
            .get_height()
            .await
            .map_err(|error| Error::EsploraConnect(error.into()))
            .tap_err(|error| tracing::error!("Failed to get height: {error}"))
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        let height = self.get_height().await?;
        let hash = self
            .client
            .get_block_hash(height)
            .await
            .map_err(|error| Error::EsploraConnect(error.into()))
            .tap_err(|error| tracing::error!("Failed to get block hash: {error}"))?;

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
            .map_err(|error| Error::EsploraScan(error.into()))
    }

    pub async fn progressive_full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        last_revealed_indices: BTreeMap<KeychainKind, u32>,
        stop_gap: usize,
        events: flume::Sender<ScanEvent<KeychainKind>>,
        cancel_token: CancellationToken,
    ) -> cove_bdk_progressive_scan::Result<FullScanResponse<KeychainKind>> {
        ProgressiveScanner::builder()
            .request(request)
            .last_revealed_indices(last_revealed_indices)
            .stop_gap(stop_gap)
            .events(events)
            .cancel_token(cancel_token)
            .esplora(self.client.clone())?
            .parallel_requests(self.options.batch_size)
            .run()
            .await
    }

    pub async fn sync(
        &self,
        request: SyncRequest<(KeychainKind, u32)>,
    ) -> Result<SyncResponse, Error> {
        debug!("starting esplora sync, batch size: {}", self.options.batch_size);

        self.client
            .sync(request, self.options.batch_size)
            .await
            .map_err(|error| Error::EsploraScan(error.into()))
    }

    pub async fn get_confirmed_transaction(
        &self,
        txid: &Txid,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        let status = self
            .client
            .get_tx_status(txid)
            .await
            .map_err(|error| Error::EsploraGetTransaction(error.into()))?;
        if !status.confirmed {
            return Ok(None);
        }

        let tx = self
            .client
            .get_tx(txid)
            .await
            .map_err(|error| Error::EsploraGetTransaction(error.into()))?;
        Ok(tx)
    }

    /// Fetches a transaction from the mempool or chain; returns `None` if not found
    pub async fn get_transaction(&self, txid: Txid) -> Result<Option<bitcoin::Transaction>, Error> {
        self.client.get_tx(&txid).await.map_err(|error| Error::EsploraGetTransaction(error.into()))
    }

    pub async fn broadcast_transaction(&self, txn: bitcoin::Transaction) -> Result<Txid, Error> {
        self.client.broadcast(&txn).await.map_err(|error| Error::EsploraBroadcast(error.into()))?;

        Ok(txn.compute_txid())
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        let stats = self
            .client
            .get_address_stats(&address)
            .await
            .map_err(|error| Error::EsploraAddress(error.into()))?;
        Ok(stats.chain_stats.tx_count > 0 || stats.mempool_stats.tx_count > 0)
    }
}

#[cfg(test)]
mod tests {

    use cove_http::NetworkRoute;
    use cove_types::network::Network;

    use super::EsploraClient;
    use crate::node::Node;

    // a node URL can carry an access token, and node errors reach logs
    #[tokio::test]
    async fn request_errors_leave_out_the_node_url() {
        let token = "private-node-access-token";
        let node = Node::new_esplora(
            "Private".to_string(),
            format!("http://127.0.0.1:1/{token}/api"),
            Network::Signet,
        );

        let client = EsploraClient::new_from_node(
            &node,
            EsploraClient::default_options(),
            &NetworkRoute::Direct,
        )
        .unwrap();

        let error = client.get_height().await.unwrap_err();

        assert!(!error.to_string().contains(token), "node error text quotes the node URL");
        assert!(!format!("{error:?}").contains(token), "node error debug quotes the node URL");
        assert_eq!(error.kind(), "esplora_connect");
    }
}
