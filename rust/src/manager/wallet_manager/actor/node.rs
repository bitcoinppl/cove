use std::{fmt, time::Duration};

use act_zero::*;
use act_zero_ext::into_actor_result;
use bdk_wallet::chain::BlockId;
use cove_tokio::FutureTimeoutExt as _;
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use tracing::debug;

use crate::{
    database::Database,
    manager::wallet_manager::{
        Error, WalletManagerReconcileMessage,
        actor::{WalletActor, WalletScanGeneration},
    },
    node::{Node, client::NodeClient},
    wallet::metadata::BlockSizeLast,
};

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
    node: Node,
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
        let key = NodeRefreshKey { node, generation };
        if let Some(in_flight) = self.height_refreshes_in_flight.get_mut(&key) {
            in_flight.attach(reply);
            return;
        }

        self.height_refreshes_in_flight.insert(key.clone(), HeightRefreshInFlight::new(reply));

        let node_client = self.node_client.clone();

        self.addr.send_fut_with(|addr| async move {
            let result = fetch_node_height(key, node_client).await;
            let _ = call!(addr.handle_height_refresh_result(result)).await;
        });
    }

    fn start_block_id_refresh(
        &mut self,
        reply: Option<BlockIdReply>,
        generation: WalletScanGeneration,
    ) {
        let node = Database::global().global_config.selected_node();
        let key = NodeRefreshKey { node, generation: Some(generation) };
        let node_client = self.node_client.clone();

        self.addr.send_fut_with(|addr| async move {
            let result = fetch_node_block_id(key, node_client).await;
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
                    let applied_height = self.apply_last_height_fetched(block_height);

                    Ok(applied_height)
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
                self.apply_last_height_fetched(block_id.height as usize);

                Produces::ok(Ok(block_id))
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
        Database::global().global_config.selected_node() == *node
    }

    fn should_apply_node_refresh(&self, key: &NodeRefreshKey) -> bool {
        self.is_selected_node(&key.node)
            && key.generation.is_none_or(|generation| generation == self.scan_generation)
    }

    pub(crate) fn last_height_fetched(&mut self) -> Option<(Duration, usize)> {
        if let Some(last_height_fetched) = self.last_height_fetched {
            return Some(last_height_fetched);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let BlockSizeLast { block_height, last_seen } = &metadata.internal.last_height_fetched?;

        let last_height_fetched = Some((*last_seen, *(block_height) as usize));
        self.last_height_fetched = last_height_fetched;

        last_height_fetched
    }

    fn save_last_height_fetched(&mut self, block_height: usize) -> Option<()> {
        let now = std::time::UNIX_EPOCH.elapsed().unwrap_or_default();
        self.last_height_fetched = Some((now, block_height));

        let wallets = Database::global().wallets();

        let mut metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let last_height_fetched =
            BlockSizeLast { block_height: block_height as u64, last_seen: now };

        metadata.internal.last_height_fetched = Some(last_height_fetched);
        wallets.update_internal_metadata(&metadata).ok();

        Database::global()
            .global_cache
            .set_block_height(self.wallet.network, last_height_fetched)
            .ok();

        self.wallet.metadata = metadata.clone();

        Some(())
    }

    fn apply_last_height_fetched(&mut self, block_height: usize) -> usize {
        let Some((_, current_height)) = self.last_height_fetched() else {
            self.save_last_height_fetched(block_height);
            return block_height;
        };

        if block_height < current_height {
            return current_height;
        }

        self.save_last_height_fetched(block_height);
        block_height
    }

    pub(crate) fn node_client(&mut self) -> Result<&NodeClient, Error> {
        let selected_node = Database::global().global_config.selected_node();
        if self.node_client.as_ref().is_some_and(|client| client.node() != &selected_node) {
            self.node_client = None;
        }

        self.node_client.as_ref().ok_or_else(|| {
            Error::NodeConnectionFailed("node client is not connected yet".to_string())
        })
    }
}

async fn checked_node_client(node: &Node) -> Result<NodeClient, Error> {
    check_node_connection_inner(node).await.map_err(Error::NodeConnectionFailed)?;

    NodeClient::new(node)
        .await
        .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)
}

async fn fetch_node_height(
    key: NodeRefreshKey,
    node_client: Option<NodeClient>,
) -> HeightRefreshResult {
    let result = async {
        let node_client = node_client_or_new(&key.node, node_client).await?;
        let block_height = node_client.get_height().await.map_err(|_| Error::GetHeightError)?;

        Ok(NodeHeightRefresh { node_client, block_height })
    }
    .await;

    NodeRefreshResult { key, result }
}

async fn fetch_node_block_id(
    key: NodeRefreshKey,
    node_client: Option<NodeClient>,
) -> BlockIdRefreshResult {
    let result = async {
        let node_client = node_client_or_new(&key.node, node_client).await?;
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
    if let Some(node_client) = node_client.filter(|client| client.node() == node) {
        return Ok(node_client);
    }

    NodeClient::new(node)
        .await
        .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)
}

async fn check_node_connection_inner(node: &Node) -> Result<(), String> {
    // create a fresh client with its own TCP connection for connection probes
    // because the actor may continue processing messages with its cached client
    // while a background check is running. the underlying rust-electrum-client
    // is not designed for concurrent access, so a fresh connection ensures no
    // shared state or concurrent access
    //
    // todo: consider reusing the cached client when using esplora, since esplora
    // uses HTTP and does not have electrum's persistent TCP concurrency limits
    let node_client = NodeClient::new(node)
        .await
        .map_err(|_| "unable to create a connection to the node".to_string())?;

    node_client
        .check_url()
        .with_timeout(Duration::from_secs(5))
        .await
        .map_err(|_| "unable to connect to node, timeout".to_string())?
        .map_err(|err| err.to_string())?;

    Ok(())
}
