pub mod electrum;
pub mod esplora;

use std::sync::Arc;
use std::{collections::BTreeMap, time::Duration};

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
use cove_http::{Socks5Proxy, TorTrafficCategory};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{
    database::{Database, global_config::GlobalConfigTable},
    node::{Node, NodeConnectionIdentity},
    tor::TorConfig,
    tor_runtime::EndpointWait,
};

use super::{ApiType, client_builder::NodeClientBuilder};

const ELECTRUM_BATCH_SIZE: usize = 10;
const ESPLORA_BATCH_SIZE: usize = 1;

/// Default bound on waiting for built-in Tor readiness during client construction
pub const TOR_READY_BOUND: Duration = Duration::from_secs(60);

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
    #[error("Tor configuration is unreadable: {0}")]
    TorConfigUnreadable(#[source] crate::database::Error),

    #[error("the selected node is an onion service and requires Tor")]
    OnionNodeRequiresTor,

    #[error("the configured SOCKS5 proxy host or port is not usable")]
    InvalidTorProxy,

    #[error("failed to create node client: {0}")]
    CreateEsploraClient(esplora_client::Error),

    #[error("failed to create node client: {0}")]
    CreateElectrumClient(electrum_client::Error),

    #[error("built-in Tor runtime is unavailable")]
    BuiltInTorRuntime(#[from] crate::tor_runtime::Error),

    #[error("built-in Tor could not be started: {0}")]
    BuiltInTorManager(#[from] crate::manager::tor_manager::TorError),

    #[error("failed to connect to node: {0}")]
    EsploraConnect(esplora_client::Error),

    #[error("failed to connect to node: {0}")]
    ElectrumConnect(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    ElectrumScan(electrum_client::Error),

    #[error("failed to complete wallet scan: {0}")]
    EsploraScan(Box<esplora_client::Error>),

    #[error("failed to complete progressive wallet scan: {0}")]
    ProgressiveScan(#[from] cove_bdk_progressive_scan::Error),

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

/// Tor route a node client uses
///
/// The absence of a route is represented by `Option::None`, so this type has no off variant
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TorProxy {
    /// Resolve the built-in Tor runtime's SOCKS5 endpoint during client construction
    BuiltIn,
    /// Route through an external SOCKS5 proxy
    ///
    /// Holds a parsed endpoint so a malformed persisted host cannot reach a client:
    /// `socks5h://host/path:9050` is read by the HTTP stack as an entirely different
    /// authority than a socket-address parser reads
    Socks5(Socks5Proxy),
}

/// Options controlling node client construction and scanning
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeClientOptions {
    pub batch_size: usize,
    pub tor: Option<TorProxy>,
    /// How long construction may wait for built-in Tor to become ready
    pub tor_ready_bound: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedNodeClientOptions {
    batch_size: usize,
    tor: Option<Socks5Proxy>,
}

impl NodeClientOptions {
    /// Loads the persisted Tor route from the database
    pub fn from_db(batch_size: usize) -> Result<Self, Error> {
        let database = Database::global();

        Self::from_global_config(batch_size, &database.global_config)
    }

    fn from_global_config(batch_size: usize, config: &GlobalConfigTable) -> Result<Self, Error> {
        let tor = match config.tor_config().map_err(Error::TorConfigUnreadable)? {
            TorConfig::Off => None,
            TorConfig::BuiltIn => Some(TorProxy::BuiltIn),
            TorConfig::External { host, port } => Some(TorProxy::Socks5(
                Socks5Proxy::validated(&host, port).map_err(|_| Error::InvalidTorProxy)?,
            )),
        };

        Ok(Self { batch_size, tor, tor_ready_bound: TOR_READY_BOUND })
    }

    /// Overrides how long construction may wait for built-in Tor to become ready
    pub(crate) fn with_tor_ready_bound(self, tor_ready_bound: Duration) -> Self {
        Self { tor_ready_bound, ..self }
    }

    /// Loads persisted options with the backend's standard batch size
    pub(crate) fn from_db_for_node(node: &Node) -> Result<Self, Error> {
        let batch_size = match node.api_type {
            ApiType::Electrum => ELECTRUM_BATCH_SIZE,
            ApiType::Esplora | ApiType::Rpc => ESPLORA_BATCH_SIZE,
        };

        Self::from_db(batch_size)
    }

    async fn resolve_tor(self, node: &Node) -> Result<ResolvedNodeClientOptions, Error> {
        // enforced here because this runs before any backend exists, so an onion
        // address can never reach the platform resolver on a clearnet route
        if node.requires_tor() && self.tor.is_none() {
            return Err(Error::OnionNodeRequiresTor);
        }

        let tor = match self.tor {
            Some(TorProxy::BuiltIn) => {
                crate::manager::tor_manager::ensure_built_in_started().await?;
                let endpoint = crate::tor_runtime::built_in_socks_endpoint(EndpointWait::Within(
                    self.tor_ready_bound,
                ))
                .await?;

                // node traffic gets its own circuits, away from Cove's HTTP requests
                Some(Socks5Proxy::isolated(
                    endpoint.ip().to_string(),
                    endpoint.port(),
                    TorTrafficCategory::Node,
                ))
            }

            // a user-supplied proxy is never told what Cove is using it for
            Some(TorProxy::Socks5(proxy)) => Some(proxy),
            None => None,
        };

        Ok(ResolvedNodeClientOptions { batch_size: self.batch_size, tor })
    }
}

impl NodeClient {
    pub async fn new(node: &Node) -> Result<Self, Error> {
        let options = NodeClientOptions::from_db_for_node(node)?;

        Self::new_with_options(node, options).await
    }

    pub async fn try_from_builder(builder: &NodeClientBuilder) -> Result<Self, Error> {
        let options = NodeClientOptions::from_db(builder.batch_size)?;
        let node_client = Self::new_with_options(&builder.node, options).await?;
        Ok(node_client)
    }

    pub async fn new_with_options(node: &Node, options: NodeClientOptions) -> Result<Self, Error> {
        // identity keeps the unresolved route so it compares equal to identities
        // computed fresh from the persisted config; only backend construction
        // sees the resolved built-in endpoint, which changes on runtime restart
        let route = options.tor.clone();
        let options = options.resolve_tor(node).await?;

        // stamped after resolution because resolving is what starts the runtime: an
        // identity taken first would carry the generation this client replaced, and
        // every later lookup would discard a perfectly good client
        let connection_identity = node.connection_identity_with_tor(route);
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

            ApiType::Rpc => {
                // TODO: implement rpc check, with auth
                todo!()
            }
        };

        Ok(Self { connection_identity, backend })
    }

    pub(crate) fn connection_identity(&self) -> &NodeConnectionIdentity {
        &self.connection_identity
    }

    /// Returns whether this client's requests are routed through Tor
    pub(crate) fn uses_tor(&self) -> bool {
        self.connection_identity.uses_tor()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_tor_config_maps_to_node_client_options() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_temp_dir, config) = test_global_config();

        for (persisted, expected) in [
            (TorConfig::Off, None),
            (TorConfig::BuiltIn, Some(TorProxy::BuiltIn)),
            (
                TorConfig::External { host: "127.0.0.1".to_string(), port: 9050 },
                Some(TorProxy::Socks5(Socks5Proxy::validated("127.0.0.1", 9050).unwrap())),
            ),
        ] {
            config.set_tor_config(persisted).unwrap();

            assert_eq!(
                NodeClientOptions::from_global_config(42, &config).unwrap(),
                NodeClientOptions {
                    batch_size: 42,
                    tor: expected,
                    tor_ready_bound: TOR_READY_BOUND,
                }
            );
        }
    }

    #[tokio::test]
    async fn onion_node_without_a_tor_route_fails_before_any_connection() {
        let onion = Node::new_electrum(
            "onion".to_string(),
            "ssl://example.onion:50002".to_string(),
            cove_types::Network::Bitcoin,
        );
        let direct =
            NodeClientOptions { batch_size: 10, tor: None, tor_ready_bound: TOR_READY_BOUND };

        let error = NodeClient::new_with_options(&onion, direct).await.unwrap_err();

        assert!(matches!(error, Error::OnionNodeRequiresTor));
    }

    #[tokio::test]
    async fn an_external_node_proxy_is_never_given_isolation_credentials() {
        let node = Node::new_electrum(
            "node".to_string(),
            "ssl://node.example:50002".to_string(),
            cove_types::Network::Bitcoin,
        );
        let options = NodeClientOptions {
            batch_size: 10,
            tor: Some(TorProxy::Socks5(Socks5Proxy::validated("proxy.example", 9050).unwrap())),
            tor_ready_bound: TOR_READY_BOUND,
        };

        let resolved = options.resolve_tor(&node).await.unwrap();
        let proxy = resolved.tor.unwrap();

        assert_eq!(proxy.credentials(), None);
        assert_eq!(proxy.authority(), "proxy.example:9050");
    }

    #[test]
    fn a_malformed_persisted_proxy_never_reaches_a_node_client() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_temp_dir, config) = test_global_config();

        // an HTTP client reads this as the authority "proxy.example" on the default
        // SOCKS port, so node traffic would silently go somewhere else entirely
        config
            .set_tor_config(TorConfig::External {
                host: "proxy.example/path".to_string(),
                port: 9050,
            })
            .unwrap();

        assert!(matches!(
            NodeClientOptions::from_global_config(1, &config),
            Err(Error::InvalidTorProxy)
        ));
    }

    fn test_global_config() -> (tempfile::TempDir, GlobalConfigTable) {
        let temp_dir = tempfile::tempdir().unwrap();
        let database = Arc::new(redb::Database::create(temp_dir.path().join("test.redb")).unwrap());
        let write_transaction = database.begin_write().unwrap();
        let config = GlobalConfigTable::new(database, &write_transaction);
        write_transaction.commit().unwrap();

        (temp_dir, config)
    }
}
