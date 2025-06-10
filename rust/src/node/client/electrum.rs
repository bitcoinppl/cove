use std::sync::Arc;

use bdk_electrum::{
    BdkElectrumClient,
    electrum_client::{self, Client, ElectrumApi as _, Param},
};
use bdk_wallet::chain::{
    BlockId, ConfirmationBlockTime, TxGraph,
    bitcoin::Address,
    spk_client::{SyncRequest, SyncResponse},
};
use bdk_wallet::{
    KeychainKind,
    chain::spk_client::{FullScanRequest, FullScanResponse},
};
use bitcoin::{Transaction, Txid, consensus::Decodable};
use serde::Deserialize;
use serde_json::Value;
use tap::TapFallible as _;
use tracing::{debug, error, warn};

use super::{ELECTRUM_BATCH_SIZE, Error, NodeClientOptions};
use crate::node::Node;

type ElectrumClientInner = BdkElectrumClient<Client>;

#[derive(Debug, Deserialize)]
struct ElectrumTransactionResponse {
    hex: String,
    confirmations: i64,
}

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
                .tap_err(|error| error!("Failed to get height: {error:?}"))
        })
        .await
        .map_err(Error::ElectrumConnect)?;

        Ok(header.height)
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        let client = self.client.clone();
        let header_notification = crate::unblock::run_blocking(move || {
            client
                .inner
                .block_headers_subscribe()
                .tap_err(|error| error!("Failed to get height: {error:?}"))
        })
        .await
        .map_err(Error::ElectrumConnect)?;

        let height = header_notification.height as u32;
        let block_hash = header_notification.header.block_hash();

        Ok(BlockId { height, hash: block_hash })
    }

    pub async fn get_confirmed_transaction(
        &self,
        txid: Arc<Txid>,
    ) -> Result<Option<bitcoin::Transaction>, Error> {
        let client = self.client.clone();
        let txid_string = txid.to_string();
        let result = crate::unblock::run_blocking(move || {
            client
                .inner
                .raw_call(
                    "blockchain.transaction.get",
                    [Param::String(txid_string), Param::Bool(true)],
                )
                .tap_err(|error| error!("electrum failed to get transaction: {error:?}"))
        })
        .await;

        fn err(e: impl Into<String>) -> Error {
            Error::ElectrumGetTransaction(electrum_client::Error::InvalidResponse(Value::String(
                e.into(),
            )))
        }

        let response = match result {
            Ok(response) => response,
            Err(electrum_client::Error::InvalidResponse(Value::Null)) => return Ok(None),
            Err(electrum_client::Error::Protocol(error_value)) => {
                // Check if this is an error related to verbose flag not being supported
                let error_str = format!("{:?}", error_value);
                if error_str.contains("verbose")
                    || error_str.contains("invalid params")
                    || error_str.contains("not supported")
                {
                    warn!(
                        "Server doesn't support verbose transactions, falling back to script-hash method"
                    );

                    let txn = self.get_confirmed_transaction_fallback(*txid).await?;
                    return Ok(txn);
                } else {
                    return Err(Error::ElectrumGetTransaction(electrum_client::Error::Protocol(
                        error_value,
                    )));
                }
            }
            Err(error) => return Err(Error::ElectrumGetTransaction(error)),
        };

        let tx_response: ElectrumTransactionResponse = serde_json::from_value(response.clone())
            .map_err(|e| err(format!("failed to deserialize electrum response: {e:?}")))?;

        if tx_response.confirmations < 1 {
            return Ok(None);
        }

        let bytes = hex::decode(&tx_response.hex)
            .map_err(|error| err(format!("failed to decode hex: {error:?}")))?;

        let txn = Transaction::consensus_decode(&mut bytes.as_slice())
            .map_err(|error| err(format!("failed to decode transaction: {error:?}")))?;

        Ok(Some(txn))
    }

    async fn get_confirmed_transaction_fallback(
        &self,
        txid: Txid,
    ) -> Result<Option<Transaction>, Error> {
        let client = self.client.clone();
        let txid_clone = txid;

        let tx: Transaction =
            crate::unblock::run_blocking(move || client.inner.transaction_get(&txid_clone))
                .await
                .tap_err(|error| error!("electrum failed to get transaction: {error:?}"))
                .map_err(Error::ElectrumGetTransaction)?;

        let tip_height = self.get_height().await? as u32;

        // Check every output script for its history
        for output in &tx.output {
            let client = self.client.clone();
            let script = output.script_pubkey.clone();

            let history =
                crate::unblock::run_blocking(move || client.inner.script_get_history(&script))
                    .await
                    .map_err(Error::ElectrumGetTransaction)?;

            // Find our transaction in the history
            if let Some(hist_entry) = history.iter().find(|h| h.tx_hash == txid && h.height > 0) {
                let block_height = hist_entry.height as u32;
                let confirmations = tip_height - block_height + 1;

                if confirmations >= 1 {
                    return Ok(Some(tx));
                }
            }
        }

        Ok(None)
    }

    pub async fn full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        stop_gap: usize,
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
        let batch_size = self.options.batch_size;

        let result = crate::unblock::run_blocking(move || {
            client.full_scan(request, stop_gap, batch_size, false).map_err(Error::ElectrumScan)
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
            client.inner.transaction_broadcast(&txn).map_err(Error::ElectrumBroadcast)
        })
        .await?;

        Ok(tx_id)
    }

    fn default_options() -> NodeClientOptions {
        NodeClientOptions { batch_size: ELECTRUM_BATCH_SIZE }
    }
}

impl std::fmt::Debug for ElectrumClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElectrumClient").field("options", &self.options).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_get_confirmed_transaction_main() {
        let client = ElectrumClient::new_from_node(&crate::node::Node {
            url: "ssl://electrum.diynodes.com:50022".to_string(),
            name: "mempool".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        })
        .unwrap();

        let id = "79fd7b17741a33006bbbaeccc30f5f8eeb07745fd2e70e88ec3c392c264500a4";
        let txid = Arc::new(Txid::from_str(id).unwrap());
        let result = client.get_confirmed_transaction(txid).await;

        match result {
            Ok(Some(tx)) => {
                assert_eq!(tx.compute_txid().to_string(), id);
            }
            Ok(None) => panic!(
                "Transaction should be confirmed but got None - this indicates an error in confirmation logic"
            ),
            Err(e) => panic!("Failed to get transaction: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_get_confirmed_transaction_fallback() {
        // blockstream.info does not support verbose transactions
        let client = ElectrumClient::new_from_node(&crate::node::Node {
            url: "ssl://electrum.blockstream.info:50002".to_string(),
            name: "blockstream".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        })
        .unwrap();

        // Test with a known confirmed transaction
        let id = "79fd7b17741a33006bbbaeccc30f5f8eeb07745fd2e70e88ec3c392c264500a4";
        let txid = Txid::from_str(id).unwrap();
        let result = client.get_confirmed_transaction_fallback(txid).await;

        match result {
            Ok(Some(txn)) => assert_eq!(txn.compute_txid().to_string(), txid.to_string()),
            Ok(None) => panic!("Expected confirmed transaction but got None"),
            Err(e) => panic!("Fallback method failed: {:?}", e),
        }
    }
}
