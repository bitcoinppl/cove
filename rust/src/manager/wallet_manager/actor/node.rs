use std::{fmt, time::Duration};

use act_zero::*;
use act_zero_ext::into_actor_result;
use bdk_wallet::chain::BlockId;
use cove_tokio::FutureTimeoutExt as _;
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use tracing::{debug, warn};

use crate::{
    database::{
        Database,
        wallet::{WalletInternalMetadataPatch, WalletMetadataPatch},
    },
    manager::wallet_manager::{
        Error, WalletManagerReconcileMessage,
        actor::{WalletActor, WalletScanGeneration},
    },
    node::{Node, NodeConnectionIdentity, client::NodeClient},
    tor::RouteError,
    wallet::metadata::BlockSizeLast,
};

/// How long the wallet's connection check waits for a node on a direct route
const DIRECT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

const HEIGHT_CACHE_FRESH_SECS: u64 = 25;
const HEIGHT_CACHE_BACKGROUND_REFRESH_SECS: u64 = 120;

type HeightRefreshResult = NodeRefreshResult<NodeHeightRefresh>;
type BlockIdRefreshResult = NodeRefreshResult<NodeBlockIdRefresh>;
pub(crate) type HeightReply = futures::channel::oneshot::Sender<Produces<Result<usize, Error>>>;
type BlockIdReply = futures::channel::oneshot::Sender<Produces<Result<BlockId, Error>>>;
type NodeConnectionReply = futures::channel::oneshot::Sender<Produces<Result<(), Error>>>;

struct NodeHeightRefresh {
    node_client: NodeClient,
    block_height: usize,
}

struct NodeBlockIdRefresh {
    node_client: NodeClient,
    block_id: BlockId,
}

struct NodeRefreshResult<T> {
    key: NodeRefreshKey,
    result: Result<T, Error>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub(crate) struct NodeRefreshKey {
    connection_identity: NodeConnectionIdentity,
    generation: Option<WalletScanGeneration>,
}

pub(crate) struct HeightRefreshInFlight {
    replies: Vec<HeightReply>,
}

impl HeightRefreshInFlight {
    fn new(reply: Option<HeightReply>) -> Self {
        let replies = reply.into_iter().collect();

        Self { replies }
    }

    fn attach(&mut self, reply: Option<HeightReply>) {
        if let Some(reply) = reply {
            self.replies.push(reply);
        }
    }

    fn finish(self, result: Result<usize, Error>) {
        for reply in self.replies {
            let _ = reply.send(Produces::Value(result.clone()));
        }
    }
}

impl fmt::Debug for HeightRefreshInFlight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeightRefreshInFlight").field("reply_count", &self.replies.len()).finish()
    }
}

impl WalletActor {
    #[into_actor_result]
    pub async fn check_node_connection(&mut self) {
        let node = Database::global().global_config.selected_node();

        self.addr.send_fut_with(|addr| async move {
            let result = check_node_connection_inner(&node).await;
            send!(addr.handle_node_connection_check_result(node, result));
        });
    }

    pub async fn get_height(&mut self, force: bool) -> ActorResult<Result<usize, Error>> {
        if let Some((last_height_fetched, block_height)) = self.last_height_fetched() {
            let elapsed = super::elapsed_secs_since(last_height_fetched);
            if !force && elapsed < HEIGHT_CACHE_BACKGROUND_REFRESH_SECS {
                if elapsed < HEIGHT_CACHE_FRESH_SECS {
                    return Produces::ok(Ok(block_height));
                }

                self.start_height_refresh(None, None);
                return Produces::ok(Ok(block_height));
            }
        }

        Ok(self.deferred_height_refresh(None))
    }

    pub(crate) async fn update_height(
        &mut self,
        generation: WalletScanGeneration,
    ) -> ActorResult<Result<usize, Error>> {
        debug!("actor update_height");
        Ok(self.deferred_height_refresh(Some(generation)))
    }

    pub(crate) async fn update_block_id(
        &mut self,
        generation: WalletScanGeneration,
    ) -> ActorResult<Result<BlockId, Error>> {
        debug!("actor update_block_id");
        Ok(self.deferred_block_id_refresh(generation))
    }

    fn start_height_refresh(
        &mut self,
        reply: Option<HeightReply>,
        generation: Option<WalletScanGeneration>,
    ) {
        let node = Database::global().global_config.selected_node();
        let key = NodeRefreshKey { connection_identity: node.connection_identity(), generation };
        if let Some(in_flight) = self.height_refreshes_in_flight.get_mut(&key) {
            in_flight.attach(reply);
            return;
        }

        self.height_refreshes_in_flight.insert(key.clone(), HeightRefreshInFlight::new(reply));

        let node_client = self.node_client.clone();

        self.addr.send_fut_with(|addr| async move {
            let result = fetch_node_height(node, key, node_client).await;
            let _ = call!(addr.handle_height_refresh_result(result)).await;
        });
    }

    fn start_block_id_refresh(
        &mut self,
        reply: Option<BlockIdReply>,
        generation: WalletScanGeneration,
    ) {
        let node = Database::global().global_config.selected_node();
        let key = NodeRefreshKey {
            connection_identity: node.connection_identity(),
            generation: Some(generation),
        };
        let node_client = self.node_client.clone();

        self.addr.send_fut_with(|addr| async move {
            let result = fetch_node_block_id(node, key, node_client).await;
            let applied = call!(addr.handle_block_id_refresh_result(result)).await;

            if let Some(reply) = reply {
                let _ = reply.send(Produces::Value(applied.unwrap_or(Err(Error::GetHeightError))));
            }
        });
    }

    fn start_node_connection(&mut self, reply: Option<NodeConnectionReply>) {
        let node = Database::global().global_config.selected_node();

        self.addr.send_fut_with(|addr| async move {
            let result = checked_node_client(&node).await;
            let applied = call!(addr.handle_node_connection_result(node, result)).await;

            if let Some(reply) = reply {
                let _ = reply.send(Produces::Value(applied.unwrap_or(Err(Error::GetHeightError))));
            }
        });
    }

    pub(crate) fn deferred_node_connection(&mut self) -> Produces<Result<(), Error>> {
        let (reply, receiver) = futures::channel::oneshot::channel();
        self.start_node_connection(Some(reply));

        Produces::Deferred(receiver)
    }

    fn deferred_height_refresh(
        &mut self,
        generation: Option<WalletScanGeneration>,
    ) -> Produces<Result<usize, Error>> {
        let (reply, receiver) = futures::channel::oneshot::channel();
        self.start_height_refresh(Some(reply), generation);

        Produces::Deferred(receiver)
    }

    fn deferred_block_id_refresh(
        &mut self,
        generation: WalletScanGeneration,
    ) -> Produces<Result<BlockId, Error>> {
        let (reply, receiver) = futures::channel::oneshot::channel();
        self.start_block_id_refresh(Some(reply), generation);

        Produces::Deferred(receiver)
    }

    async fn handle_node_connection_result(
        &mut self,
        node: Node,
        result: Result<NodeClient, Error>,
    ) -> ActorResult<Result<(), Error>> {
        if !self.is_selected_node(&node) {
            return Produces::ok(Err(Error::GetHeightError));
        }

        match result {
            Ok(node_client) => {
                self.node_client = Some(node_client);
                Produces::ok(Ok(()))
            }

            Err(error) => {
                self.report_node_refresh_error(&error);
                Produces::ok(Err(error))
            }
        }
    }

    async fn handle_height_refresh_result(
        &mut self,
        result: HeightRefreshResult,
    ) -> ActorResult<Result<usize, Error>> {
        let NodeRefreshResult { key, result } = result;
        let result = if !self.should_apply_node_refresh(&key) {
            Err(Error::GetHeightError)
        } else {
            match result {
                Ok(refresh) => {
                    let block_height = refresh.block_height;
                    self.node_client = Some(refresh.node_client);
                    self.apply_last_height_fetched(block_height)
                }

                Err(error) => {
                    self.report_node_refresh_error(&error);
                    Err(error)
                }
            }
        };

        if let Some(in_flight) = self.height_refreshes_in_flight.remove(&key) {
            in_flight.finish(result.clone());
        }

        Produces::ok(result)
    }

    async fn handle_block_id_refresh_result(
        &mut self,
        result: BlockIdRefreshResult,
    ) -> ActorResult<Result<BlockId, Error>> {
        let NodeRefreshResult { key, result } = result;
        if !self.should_apply_node_refresh(&key) {
            return Produces::ok(Err(Error::GetHeightError));
        }

        match result {
            Ok(refresh) => {
                let block_id = refresh.block_id;
                self.node_client = Some(refresh.node_client);
                match self.apply_last_height_fetched(block_id.height as usize) {
                    Ok(_) => Produces::ok(Ok(block_id)),
                    Err(error) => Produces::ok(Err(error)),
                }
            }

            Err(error) => {
                self.report_node_refresh_error(&error);
                Produces::ok(Err(error))
            }
        }
    }

    fn report_node_refresh_error(&self, error: &Error) {
        match error {
            Error::NodeConnectionFailed(error) => {
                self.send(WalletManagerReconcileMessage::NodeConnectionFailed(error.clone()));
            }

            error => {
                self.send(WalletManagerReconcileMessage::WalletError(error.clone()));
            }
        }
    }

    async fn handle_node_connection_check_result(
        &mut self,
        node: Node,
        result: Result<(), String>,
    ) -> ActorResult<()> {
        if self.is_selected_node(&node)
            && let Err(error) = result
        {
            self.send(WalletManagerReconcileMessage::NodeConnectionFailed(error));
        }

        Produces::ok(())
    }

    fn is_selected_node(&self, node: &Node) -> bool {
        self.is_selected_connection(&node.connection_identity())
    }

    fn is_selected_connection(&self, identity: &NodeConnectionIdentity) -> bool {
        Database::global().global_config.selected_node().connection_identity() == *identity
    }

    fn should_apply_node_refresh(&self, key: &NodeRefreshKey) -> bool {
        self.is_selected_connection(&key.connection_identity)
            && key.generation.is_none_or(|generation| generation == self.scan_generation)
    }

    pub(crate) fn last_height_fetched(&mut self) -> Option<(Duration, usize)> {
        if let Some(last_height_fetched) = self.last_height_fetched {
            return Some(last_height_fetched);
        }

        let BlockSizeLast { block_height, last_seen } =
            &self.wallet.metadata.internal.last_height_fetched?;

        let last_height_fetched = Some((*last_seen, *(block_height) as usize));
        self.last_height_fetched = last_height_fetched;

        last_height_fetched
    }

    fn save_last_height_fetched(&mut self, block_height: usize) -> Result<(), Error> {
        let now = std::time::UNIX_EPOCH.elapsed().unwrap_or_default();
        let last_height_fetched =
            BlockSizeLast { block_height: block_height as u64, last_seen: now };

        self.apply_metadata_patch(WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
            last_height_fetched: Some(last_height_fetched),
            ..Default::default()
        }))?;

        self.last_height_fetched = Some((now, block_height));

        if !self.wallet.uses_persistent_storage() {
            return Ok(());
        }

        Database::global()
            .global_cache
            .set_block_height(self.wallet.network, last_height_fetched)
            .ok();

        Ok(())
    }

    fn apply_last_height_fetched(&mut self, block_height: usize) -> Result<usize, Error> {
        let Some((_, current_height)) = self.last_height_fetched() else {
            self.save_last_height_fetched(block_height)?;
            return Ok(block_height);
        };

        if block_height < current_height {
            return Ok(current_height);
        }

        self.save_last_height_fetched(block_height)?;
        Ok(block_height)
    }

    pub(crate) fn node_client(&mut self) -> Result<&NodeClient, Error> {
        let selected_node = Database::global().global_config.selected_node();
        let selected_identity = selected_node.connection_identity();
        if self
            .node_client
            .as_ref()
            .is_some_and(|client| client.connection_identity() != &selected_identity)
        {
            self.node_client = None;
        }

        self.node_client.as_ref().ok_or_else(|| {
            Error::NodeConnectionFailed("node client is not connected yet".to_string())
        })
    }
}

pub(crate) async fn checked_node_client(node: &Node) -> Result<NodeClient, Error> {
    check_node_connection_inner(node).await.map_err(Error::NodeConnectionFailed)?;

    NodeClient::new(node)
        .await
        .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)
}

async fn fetch_node_height(
    node: Node,
    key: NodeRefreshKey,
    node_client: Option<NodeClient>,
) -> HeightRefreshResult {
    let result = async {
        let node_client = node_client_or_new(&node, node_client).await?;
        let block_height = node_client.get_height().await.map_err(|_| Error::GetHeightError)?;

        Ok(NodeHeightRefresh { node_client, block_height })
    }
    .await;

    NodeRefreshResult { key, result }
}

async fn fetch_node_block_id(
    node: Node,
    key: NodeRefreshKey,
    node_client: Option<NodeClient>,
) -> BlockIdRefreshResult {
    let result = async {
        let node_client = node_client_or_new(&node, node_client).await?;
        let block_id = node_client.get_block_id().await.map_err(|_| Error::GetHeightError)?;

        Ok(NodeBlockIdRefresh { node_client, block_id })
    }
    .await;

    NodeRefreshResult { key, result }
}

async fn node_client_or_new(
    node: &Node,
    node_client: Option<NodeClient>,
) -> Result<NodeClient, Error> {
    let connection_identity = node.connection_identity();
    if let Some(node_client) =
        node_client.filter(|client| client.connection_identity() == &connection_identity)
    {
        return Ok(node_client);
    }

    NodeClient::new(node)
        .await
        .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)
}

/// Checks that `node` answers, checking again on the new route when a route change stops it
///
/// Turning Tor on while a wallet opens would otherwise fail the check that starts its scan
async fn check_node_connection_inner(node: &Node) -> Result<(), String> {
    match check_node_connection_once(node).await {
        Err(NodeCheckFailure::RouteChanged) => {
            crate::tor::usable_route()
                .with_timeout(crate::node::TOR_CHECK_URL_TIMEOUT)
                .await
                .map_err(|_| "unable to connect to node, Tor is not ready".to_string())?;

            check_node_connection_once(node).await.map_err(String::from)
        }

        result => result.map_err(String::from),
    }
}

/// Why one node check failed
enum NodeCheckFailure {
    /// The route changed while the check ran
    RouteChanged,
    /// A message the user can read
    Failed(String),
}

impl From<NodeCheckFailure> for String {
    fn from(failure: NodeCheckFailure) -> Self {
        match failure {
            NodeCheckFailure::RouteChanged => RouteError::Changed.to_string(),
            NodeCheckFailure::Failed(message) => message,
        }
    }
}

async fn check_node_connection_once(node: &Node) -> Result<(), NodeCheckFailure> {
    // create a fresh client with its own TCP connection for connection probes
    // because the actor may continue processing messages with its cached client
    // while a background check is running. the underlying rust-electrum-client
    // is not designed for concurrent access, so a fresh connection ensures no
    // shared state or concurrent access
    //
    // todo: consider reusing the cached client when using esplora, since esplora
    // uses HTTP and does not have electrum's persistent TCP concurrency limits
    // the user-facing message stays generic, so the log keeps the failure kind. the cause's
    // text can quote the node URL and any credentials in it
    let node_client = NodeClient::new(node).await.map_err(|error| {
        let kind = error.kind();
        warn!("failed to create node client for connection check: {kind}");
        NodeCheckFailure::Failed("unable to create a connection to the node".to_string())
    })?;

    // Tor's first request can take far longer than a direct one while Tor joins the network
    let timeout = node_client.check_timeout(DIRECT_CHECK_TIMEOUT);
    let checked =
        node_client.check_url().with_timeout(timeout).await.map_err(|_| {
            NodeCheckFailure::Failed("unable to connect to node, timeout".to_string())
        })?;

    match checked {
        Ok(()) => Ok(()),
        Err(error) if error.is_route_changed() => Err(NodeCheckFailure::RouteChanged),
        Err(error) => Err(NodeCheckFailure::Failed(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cove_http::ProxyAddress;

    use crate::{
        network::Network,
        node::Node,
        test_support::{
            fake_node::{FakeNode, wait_until},
            socks_proxy::{RecordingSocksProxy, TorTest},
        },
    };

    // the check that starts a wallet's scan failed when Tor turned on while it ran, so a wallet
    // opened just then never scanned
    #[tokio::test]
    async fn a_node_check_that_tor_interrupts_checks_again_through_tor() {
        let tor = TorTest::start().await;
        let node = FakeNode::start(bitcoin::Network::Bitcoin);
        let esplora = node.esplora_node("esplora");

        let (gate, rule) = node.on("/blocks/tip/height").hold();
        let check = tokio::spawn(async move { super::check_node_connection_inner(&esplora).await });
        wait_until("the check to reach the node directly", || gate.waiting() > 0).await;

        let proxy = RecordingSocksProxy::start();
        tor.use_proxy(proxy.proxy_address());
        wait_until("the check to reach the node through Tor", || !proxy.targets().is_empty()).await;
        drop(rule);
        gate.open();

        let result = check.await.unwrap();
        assert_eq!(result, Ok(()), "the check gave up when Tor turned on");
    }

    // the check gave up after five seconds even over Tor, so a wallet opened while Tor was still
    // joining the network never started its scan; paused time lets the wait run without delay
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn the_wallet_check_waits_longer_over_tor() {
        let tor = TorTest::start().await;

        // takes connections and never answers, like a Tor that's still joining the network
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = silent.local_addr().unwrap();
        tor.use_proxy(ProxyAddress::new_ip(address.ip(), address.port()));

        let node =
            Node::new_esplora("esplora".into(), "http://node.invalid/api".into(), Network::Bitcoin);

        let started = tokio::time::Instant::now();
        let result = super::check_node_connection_inner(&node).await;
        let waited = started.elapsed();

        assert!(result.is_err(), "a proxy that never answers can't pass the check");
        assert!(waited >= Duration::from_secs(30), "the check gave up over Tor after {waited:?}");
        drop(silent);
    }
}
