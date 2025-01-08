use std::sync::Arc;

use bdk_chain::{bitcoin::Address, ConfirmationBlockTime, TxGraph};
use bdk_core::spk_client::{FullScanRequest, FullScanResponse, SyncRequest, SyncResponse};
use bdk_electrum::{
    electrum_client::{Client, ElectrumApi as _},
    BdkElectrumClient,
};
use bdk_wallet::KeychainKind;
use bitcoin::{Transaction, Txid};
use tap::TapFallible as _;
use tracing::debug;

use super::{Error, NodeClientOptions, ELECTRUM_BATCH_SIZE, STOP_GAP};
use crate::node::Node;

type ElectrumClientInner = BdkElectrumClient<Client>;

#[derive(Clone)]
pub struct ElectrumClient {
    client: Arc<ElectrumClientInner>,
    options: NodeClientOptions,
}

impl ElectrumClient {
    pub fn new_with_options(client: Arc<ElectrumClientInner>, options: NodeClientOptions) -> Self {
        Self { client, options }
    }

    pub fn new(client: Arc<ElectrumClientInner>) -> Self {
        Self::new_with_options(client, Self::default_options())
    }

    pub fn new_from_node(node: &Node) -> Result<Self, Error> {
        Self::new_from_node_and_options(node, Self::default_options())
    }

    pub fn new_from_node_and_options(
        node: &Node,
        options: NodeClientOptions,
    ) -> Result<Self, Error> {
        let url = node.url.strip_suffix('/').unwrap_or(&node.url);
        let inner_client = Client::new(url).map_err(Error::CreateElectrumClient)?;
        let bdk_client = BdkElectrumClient::new(inner_client);
        let client = Arc::new(bdk_client);

        Ok(Self::new_with_options(client, options))
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        let client = self.client.clone();
        let header = crate::unblock::run_blocking(move || {
            client
                .inner
                .block_headers_subscribe()
                .tap_err(|error| tracing::error!("Failed to get height: {error:?}"))
        })
        .await
        .map_err(Error::ElectrumConnect)?;

        Ok(header.height)
    }

    pub async fn full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
    ) -> Result<FullScanResponse<KeychainKind>, Error> {
        debug!("start populate_tx_cache");
        let client = self.client.clone();
        let tx_graph = tx_graph.clone();
        crate::unblock::run_blocking(move || {
            client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx))
        })
        .await;
        debug!("populate_tx_cache done");

        let client = self.client.clone();
        let stop_gap = self.options.stop_gap;
        let batch_size = self.options.batch_size;

        let result = crate::unblock::run_blocking(move || {
            client
                .full_scan(request, stop_gap, batch_size, false)
                .map_err(Error::ElectrumScan)
        })
        .await?;

        Ok(result)
    }

    pub async fn sync(
        &self,
        request: SyncRequest<(KeychainKind, u32)>,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
    ) -> Result<SyncResponse, Error> {
        debug!("start populate_tx_cache");
        let client = self.client.clone();
        let tx_graph = tx_graph.clone();
        crate::unblock::run_blocking(move || {
            client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx))
        })
        .await;
        debug!("populate_tx_cache done");

        let client = self.client.clone();
        let batch_size = self.options.batch_size;

        let client = client.clone();

        let result = crate::unblock::run_blocking(move || client.sync(request, batch_size, false))
            .await
            .map_err(Error::ElectrumScan)?;

        Ok(result)
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        let client = self.client.clone();
        let txns = crate::unblock::run_blocking(move || {
            let script = address.script_pubkey();
            client.inner.script_get_history(&script)
        })
        .await
        .map_err(Error::ElectrumAddress)?;

        Ok(!txns.is_empty())
    }

    pub async fn broadcast_transaction(&self, txn: Transaction) -> Result<Txid, Error> {
        let client = self.client.clone();
        let tx_id = crate::unblock::run_blocking(move || {
            client
                .inner
                .transaction_broadcast(&txn)
                .map_err(Error::ElectrumBroadcast)
        })
        .await?;

        Ok(tx_id)
    }

    fn default_options() -> NodeClientOptions {
        NodeClientOptions {
            batch_size: ELECTRUM_BATCH_SIZE,
            stop_gap: STOP_GAP,
        }
    }
}

impl std::fmt::Debug for ElectrumClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElectrumClient")
            .field("options", &self.options)
            .finish()
    }
}
