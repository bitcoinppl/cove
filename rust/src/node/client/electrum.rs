use std::{collections::BTreeMap, sync::Arc};

use bdk_electrum::{
    BdkElectrumClient,
    electrum_client::{self, Client, Config, ConfigBuilder, ElectrumApi as _, Param, Socks5Config},
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
use cove_bdk_progressive_scan::{ProgressiveScanner, ScanEvent};
use serde::Deserialize;
use serde_json::Value;
use tap::TapFallible as _;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

#[cfg(test)]
use super::{ELECTRUM_BATCH_SIZE, NodeClientOptions, TorProxy};
use super::{Error, ResolvedNodeClientOptions};
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
    options: ResolvedNodeClientOptions,
}

impl ElectrumClient {
    const fn new_with_options(
        client: Arc<ElectrumClientInner>,
        options: ResolvedNodeClientOptions,
    ) -> Self {
        Self { client, options }
    }

    pub(crate) async fn new_from_node_and_options(
        node: &Node,
        options: ResolvedNodeClientOptions,
    ) -> Result<Self, Error> {
        let url = node.url.strip_suffix('/').unwrap_or(&node.url).to_string();
        let config = Self::connection_config(&options);

        // use spawn_blocking for the synchronous TCP connection to avoid blocking the async runtime
        let inner_client =
            cove_tokio::unblock::run_blocking(move || Client::from_config(&url, config))
                .await
                .map_err(Error::CreateElectrumClient)?;

        let bdk_client = BdkElectrumClient::new(inner_client);
        let client = Arc::new(bdk_client);

        Ok(Self::new_with_options(client, options))
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        let client = self.client.clone();
        let header = cove_tokio::unblock::run_blocking(move || {
            client
                .inner
                .block_headers_subscribe()
                .tap_err(|error| debug!("Failed to get height: {error:?}"))
        })
        .await
        .map_err(Error::ElectrumConnect)?;

        Ok(header.height)
    }

    pub async fn get_block_id(&self) -> Result<BlockId, Error> {
        let client = self.client.clone();
        let header_notification = cove_tokio::unblock::run_blocking(move || {
            client
                .inner
                .block_headers_subscribe()
                .tap_err(|error| debug!("Failed to get height: {error:?}"))
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
        let result = cove_tokio::unblock::run_blocking(move || {
            client
                .inner
                .raw_call(
                    "blockchain.transaction.get",
                    [Param::String(txid_string), Param::Bool(true)],
                )
                .tap_err(|error| debug!("electrum failed to get transaction: {error:?}"))
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
                let error_str = format!("{error_value:?}");
                if error_str.contains("verbose")
                    || error_str.contains("invalid params")
                    || error_str.contains("not supported")
                {
                    warn!(
                        "Server doesn't support verbose transactions, falling back to script-hash method"
                    );

                    let txn = self.get_confirmed_transaction_fallback(*txid).await?;
                    return Ok(txn);
                }

                return Err(Error::ElectrumGetTransaction(electrum_client::Error::Protocol(
                    error_value,
                )));
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

    /// Fetches a transaction from the mempool or chain; returns `None` if not found.
    pub async fn get_transaction(&self, txid: Txid) -> Result<Option<bitcoin::Transaction>, Error> {
        let client = self.client.clone();
        cove_tokio::unblock::run_blocking(move || client.inner.transaction_get(&txid))
            .await
            .map(Some)
            .or_else(|e| match e {
                // Electrum returns a Protocol error when the transaction is not in the
                // mempool or chain; map to Ok(None) for consistent "not found" semantics
                // across node backends.  Some server implementations may also return a
                // null response, so that is handled here too.
                electrum_client::Error::Protocol(_)
                | electrum_client::Error::InvalidResponse(Value::Null) => Ok(None),
                other => Err(Error::ElectrumGetTransaction(other)),
            })
    }

    async fn get_confirmed_transaction_fallback(
        &self,
        txid: Txid,
    ) -> Result<Option<Transaction>, Error> {
        let client = self.client.clone();
        let txid_clone = txid;

        let tx: Transaction =
            cove_tokio::unblock::run_blocking(move || client.inner.transaction_get(&txid_clone))
                .await
                .tap_err(|error| debug!("electrum failed to get transaction: {error:?}"))
                .map_err(Error::ElectrumGetTransaction)?;

        let tip_height = self.get_height().await? as u32;

        // Check every output script for its history
        for output in &tx.output {
            let client = self.client.clone();
            let script = output.script_pubkey.clone();

            let history =
                cove_tokio::unblock::run_blocking(move || client.inner.script_get_history(&script))
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
        cove_tokio::unblock::run_blocking(move || {
            client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx));
        })
        .await;
        debug!("populate_tx_cache done");

        let client = self.client.clone();
        let batch_size = self.options.batch_size;

        let result = cove_tokio::unblock::run_blocking(move || {
            client.full_scan(request, stop_gap, batch_size, false).map_err(Error::ElectrumScan)
        })
        .await?;

        Ok(result)
    }

    pub async fn progressive_full_scan(
        &self,
        request: FullScanRequest<KeychainKind>,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
        last_revealed_indices: BTreeMap<KeychainKind, u32>,
        stop_gap: usize,
        events: flume::Sender<ScanEvent<KeychainKind>>,
        cancel_token: CancellationToken,
    ) -> cove_bdk_progressive_scan::Result<FullScanResponse<KeychainKind>> {
        debug!("start populate_tx_cache for progressive scan");
        let client = self.client.clone();
        let tx_graph = tx_graph.clone();
        cove_tokio::unblock::run_blocking(move || {
            client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx));
        })
        .await;
        debug!("populate_tx_cache for progressive scan done");

        let client = self.client.clone();
        let batch_size = self.options.batch_size;
        cove_tokio::unblock::run_blocking(move || {
            ProgressiveScanner::builder()
                .request(request)
                .last_revealed_indices(last_revealed_indices)
                .stop_gap(stop_gap)
                .events(events)
                .cancel_token(cancel_token)
                .electrum(client)?
                .batch_size(batch_size)
                .fetch_prev_txouts(false)
                .run()
        })
        .await
    }

    pub async fn sync(
        &self,
        request: SyncRequest<(KeychainKind, u32)>,
        tx_graph: &TxGraph<ConfirmationBlockTime>,
    ) -> Result<SyncResponse, Error> {
        debug!("start populate_tx_cache");
        let client = self.client.clone();
        let tx_graph = tx_graph.clone();
        cove_tokio::unblock::run_blocking(move || {
            client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx));
        })
        .await;
        debug!("populate_tx_cache done");

        let client = self.client.clone();
        let batch_size = self.options.batch_size;

        let client = client.clone();

        let result =
            cove_tokio::unblock::run_blocking(move || client.sync(request, batch_size, false))
                .await
                .map_err(Error::ElectrumScan)?;

        Ok(result)
    }

    pub async fn check_address_for_txn(&self, address: Address) -> Result<bool, Error> {
        let client = self.client.clone();
        let txns = cove_tokio::unblock::run_blocking(move || {
            let script = address.script_pubkey();
            client.inner.script_get_history(&script)
        })
        .await
        .map_err(Error::ElectrumAddress)?;

        Ok(!txns.is_empty())
    }

    pub async fn broadcast_transaction(&self, txn: Transaction) -> Result<Txid, Error> {
        let client = self.client.clone();
        let tx_id = cove_tokio::unblock::run_blocking(move || {
            client.inner.transaction_broadcast(&txn).map_err(Error::ElectrumBroadcast)
        })
        .await?;

        Ok(tx_id)
    }

    fn connection_config(options: &ResolvedNodeClientOptions) -> Config {
        let socks5 = options.tor.as_ref().map(|endpoint| Socks5Config::new(endpoint.authority()));

        ConfigBuilder::new().socks5(socks5).build()
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
    use bdk_electrum::electrum_client::Param;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    #[ignore] // requires a local Tor SOCKS proxy on 127.0.0.1:9050
    async fn test_get_confirmed_transaction_fallback() {
        crate::test_support::ensure_tokio_runtime();

        // blockstream.info does not support verbose transactions
        let node = crate::node::Node {
            url: "ssl://electrum.blockstream.info:50002".to_string(),
            name: "blockstream".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        };
        let options = NodeClientOptions {
            batch_size: ELECTRUM_BATCH_SIZE,
            tor: Some(TorProxy::Socks5 { host: "127.0.0.1".to_string(), port: 9050 }),
        }
        .resolve_tor()
        .await
        .unwrap();
        let client = timeout(
            Duration::from_secs(20),
            ElectrumClient::new_from_node_and_options(&node, options),
        )
        .await
        .expect("timed out creating electrum client")
        .expect("failed to create electrum client through Tor proxy");

        // test with a known confirmed transaction
        let id = "79fd7b17741a33006bbbaeccc30f5f8eeb07745fd2e70e88ec3c392c264500a4";
        let txid = Arc::new(Txid::from_str(id).unwrap());
        let result =
            timeout(Duration::from_secs(20), client.get_confirmed_transaction(txid.clone()))
                .await
                .expect("timed out fetching confirmed transaction through Tor proxy");

        match result {
            Ok(Some(txn)) => assert_eq!(txn.compute_txid().to_string(), txid.to_string()),
            Ok(None) => panic!("Expected confirmed transaction but got None"),
            Err(e) => panic!("Fallback method failed: {e:?}"),
        }
    }

    #[tokio::test]
    #[ignore] // requires a local Tor SOCKS proxy on 127.0.0.1:9050
    async fn test_onion_electrum_via_tor_proxy() {
        crate::test_support::ensure_tokio_runtime();

        let node = crate::node::Node {
            url: "tcp://xotqmhnei2wy7fk423tekp62ilcxpawnf4aiqmnkfhuutfkimgpqk5qd.onion:50001"
                .to_string(),
            name: "onion-electrum".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        };
        let options = NodeClientOptions {
            batch_size: ELECTRUM_BATCH_SIZE,
            tor: Some(TorProxy::Socks5 { host: "127.0.0.1".to_string(), port: 9050 }),
        }
        .resolve_tor()
        .await
        .unwrap();
        let client = timeout(
            Duration::from_secs(20),
            ElectrumClient::new_from_node_and_options(&node, options),
        )
        .await
        .expect("timed out creating electrum client")
        .expect("failed to create electrum client through Tor proxy");

        let raw_version = timeout(Duration::from_secs(20), async {
            cove_tokio::unblock::run_blocking({
                let inner = client.client.clone();

                move || {
                    inner.inner.raw_call(
                        "server.version",
                        [Param::String("cove-test".to_string()), Param::String("1.4".to_string())],
                    )
                }
            })
            .await
        })
        .await
        .expect("timed out waiting for server.version response")
        .expect("server.version call failed via onion Electrum through Tor proxy");

        let response =
            raw_version.as_array().expect("server.version response should be a JSON array");
        assert!(response.len() >= 2, "server.version response array too short");
        assert!(response[0].is_string(), "server software field must be a string");
        assert!(response[1].is_string(), "protocol version field must be a string");
    }
}
