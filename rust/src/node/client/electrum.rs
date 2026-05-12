use std::sync::Arc;

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
use serde::Deserialize;
use serde_json::Value;
use tap::TapFallible as _;
use tokio::time::sleep;
use tracing::{debug, error, warn};

use super::{ELECTRUM_BATCH_SIZE, Error, NodeClientOptions};
use crate::node::{Node, TorMode};

type ElectrumClientInner = BdkElectrumClient<Client>;

const BUILT_IN_TOR_CONNECT_RETRY_DELAYS_MS: [u64; 8] =
    [500, 1000, 1500, 2000, 3000, 5000, 8000, 12000];

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
    pub const fn new_with_options(
        client: Arc<ElectrumClientInner>,
        options: NodeClientOptions,
    ) -> Self {
        Self { client, options }
    }

    pub fn new(client: Arc<ElectrumClientInner>) -> Self {
        Self::new_with_options(client, Self::default_options())
    }

    pub async fn new_from_node(node: &Node) -> Result<Self, Error> {
        let options = Self::default_options();
        Self::new_from_node_and_options(node, &options).await
    }

    pub async fn new_from_node_and_options(
        node: &Node,
        options: &NodeClientOptions,
    ) -> Result<Self, Error> {
        let url = node.url.strip_suffix('/').unwrap_or(&node.url).to_string();
        debug!(
            api_type = "electrum",
            tor_enabled = options.use_tor,
            tor_mode = ?options.tor_mode,
            batch_size = options.batch_size,
            "creating electrum client from node and options"
        );
        let config = Self::connection_config(options);
        let retry_delays = if options.use_tor && matches!(options.tor_mode, TorMode::BuiltIn) {
            BUILT_IN_TOR_CONNECT_RETRY_DELAYS_MS.as_slice()
        } else {
            &[]
        };

        let mut attempt: usize = 0;
        let inner_client = loop {
            let conn_url = url.clone();
            let config = config.clone();

            // use spawn_blocking for the synchronous TCP connection to avoid blocking the async runtime
            match cove_tokio::unblock::run_blocking(move || Client::from_config(&conn_url, config))
                .await
            {
                Ok(client) => break client,
                Err(error) => {
                    if Self::is_built_in_tor_bootstrap_socks_failure(&error)
                        && let Some(delay_ms) = retry_delays.get(attempt).copied()
                    {
                        warn!(
                            api_type = "electrum",
                            tor_enabled = options.use_tor,
                            tor_mode = ?options.tor_mode,
                            attempt = attempt + 1,
                            max_retries = retry_delays.len(),
                            delay_ms,
                            "electrum connect failed while built-in tor may still be bootstrapping; retrying"
                        );
                        attempt += 1;
                        sleep(std::time::Duration::from_millis(delay_ms)).await;
                        continue;
                    }

                    let built_in_tor_status =
                        if options.use_tor && matches!(options.tor_mode, TorMode::BuiltIn) {
                            crate::tor_runtime::built_in_status_summary()
                        } else {
                            "n/a".to_string()
                        };

                    error!(
                        api_type = "electrum",
                        tor_enabled = options.use_tor,
                        tor_mode = ?options.tor_mode,
                        attempt = attempt + 1,
                        max_retries = retry_delays.len(),
                        built_in_tor_status = %built_in_tor_status,
                        "failed to create electrum client"
                    );

                    if options.use_tor
                        && matches!(options.tor_mode, TorMode::BuiltIn)
                        && Self::is_built_in_tor_bootstrap_socks_failure(&error)
                    {
                        let enriched = format!(
                            "{error}; built-in tor status: {built_in_tor_status}; this often means Tor bootstrap is still incomplete or bootstrap connectivity failed"
                        );
                        return Err(Error::CreateElectrumClient(electrum_client::Error::Message(
                            enriched,
                        )));
                    }

                    return Err(Error::CreateElectrumClient(error));
                }
            }
        };

        let bdk_client = BdkElectrumClient::new(inner_client);
        let client = Arc::new(bdk_client);

        Ok(Self::new_with_options(client, options.clone()))
    }

    pub async fn get_height(&self) -> Result<usize, Error> {
        let client = self.client.clone();
        let header = cove_tokio::unblock::run_blocking(move || {
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
        let header_notification = cove_tokio::unblock::run_blocking(move || {
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
        let result = cove_tokio::unblock::run_blocking(move || {
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

    async fn get_confirmed_transaction_fallback(
        &self,
        txid: Txid,
    ) -> Result<Option<Transaction>, Error> {
        let client = self.client.clone();
        let txid_clone = txid;

        let tx: Transaction =
            cove_tokio::unblock::run_blocking(move || client.inner.transaction_get(&txid_clone))
                .await
                .tap_err(|error| error!("electrum failed to get transaction: {error:?}"))
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

    fn default_options() -> NodeClientOptions {
        NodeClientOptions { batch_size: ELECTRUM_BATCH_SIZE, ..NodeClientOptions::default() }
    }

    fn connection_config(options: &NodeClientOptions) -> Config {
        let socks5_addr = if options.use_tor {
            let endpoint = format!("{}:{}", options.tor_external_host, options.tor_external_port);
            debug!(
                api_type = "electrum",
                tor_enabled = true,
                tor_mode = ?options.tor_mode,
                "electrum using socks5 endpoint"
            );
            Some(endpoint)
        } else {
            debug!("electrum connecting without tor proxy");
            None
        };

        ConfigBuilder::new().socks5(socks5_addr.map(Socks5Config::new)).build()
    }

    fn is_built_in_tor_bootstrap_socks_failure(error: &electrum_client::Error) -> bool {
        matches!(
            error,
            electrum_client::Error::IOError(io_error)
                if io_error.kind() == std::io::ErrorKind::Other
                    && io_error.to_string().contains("general SOCKS server failure")
        )
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
    use bdk_electrum::electrum_client::{ElectrumApi, Param};
    use std::{str::FromStr, time::Duration};
    use tokio::time::timeout;

    #[tokio::test]
    #[ignore] // requires external network connection to blockstream electrum server
    async fn test_get_confirmed_transaction_fallback() {
        // blockstream.info does not support verbose transactions
        let client = ElectrumClient::new_from_node(&crate::node::Node {
            url: "ssl://electrum.blockstream.info:50002".to_string(),
            name: "blockstream".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        })
        .await
        .unwrap();

        // Test with a known confirmed transaction
        let id = "79fd7b17741a33006bbbaeccc30f5f8eeb07745fd2e70e88ec3c392c264500a4";
        let txid = Arc::new(Txid::from_str(id).unwrap());
        let result = client.get_confirmed_transaction(txid.clone()).await;

        match result {
            Ok(Some(txn)) => assert_eq!(txn.compute_txid().to_string(), txid.to_string()),
            Ok(None) => panic!("Expected confirmed transaction but got None"),
            Err(e) => panic!("Fallback method failed: {e:?}"),
        }
    }

    #[tokio::test]
    #[ignore] // requires local Tor SOCKS proxy and reachable onion electrum server
    async fn test_onion_electrum_via_tor_proxy() {
        cove_tokio::init();

        let node = crate::node::Node {
            url: "tcp://xotqmhnei2wy7fk423tekp62ilcxpawnf4aiqmnkfhuutfkimgpqk5qd.onion:50001"
                .to_string(),
            name: "onion-electrum".to_string(),
            api_type: crate::node::ApiType::Electrum,
            network: cove_types::network::Network::Bitcoin,
        };

        let options = NodeClientOptions {
            batch_size: 10,
            use_tor: true,
            tor_mode: crate::node::TorMode::External,
            tor_external_host: "127.0.0.1".to_string(),
            tor_external_port: 9050,
        };

        let client = timeout(
            Duration::from_secs(20),
            ElectrumClient::new_from_node_and_options(&node, &options),
        )
        .await
        .expect("timed out creating electrum client")
        .expect("failed to create electrum client through tor proxy");

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
        .expect("server.version call failed via onion electrum through tor proxy");

        let response =
            raw_version.as_array().expect("server.version response should be a JSON array");
        assert!(response.len() >= 2, "server.version response array too short");
        assert!(response[0].is_string(), "server software field must be a string");
        assert!(response[1].is_string(), "protocol version field must be a string");
    }
}
