use std::time::Duration;

use act_zero::*;
use act_zero_ext::into_actor_result;
use bdk_wallet::chain::BlockId;
use cove_tokio::FutureTimeoutExt as _;
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use tracing::debug;

use crate::{
    database::Database,
    manager::wallet_manager::{Error, WalletManagerReconcileMessage, actor::WalletActor},
    node::{Node, client::NodeClient},
    wallet::metadata::BlockSizeLast,
};

impl WalletActor {
    pub(crate) async fn ensure_node_connection(&mut self) -> Result<(), Error> {
        let node = Database::global().global_config.selected_node();
        check_node_connection_inner(&node).await.map_err(Error::NodeConnectionFailed)?;

        let node_client = NodeClient::new(&node)
            .await
            .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)?;
        self.node_client = Some(node_client);

        Ok(())
    }

    #[into_actor_result]
    pub async fn check_node_connection(&mut self) {
        let node = Database::global().global_config.selected_node();

        let reconciler = self.reconciler.clone();
        self.addr.send_fut(async move {
            if let Err(error) = check_node_connection_inner(&node).await {
                let _ = reconciler
                    .send(WalletManagerReconcileMessage::NodeConnectionFailed(error).into());
            }
        });
    }

    pub async fn get_height(&mut self, force: bool) -> ActorResult<usize> {
        if let Some((last_height_fetched, block_height)) = self.last_height_fetched() {
            let elapsed = super::elapsed_secs_since(last_height_fetched);
            if !force && elapsed < 120 {
                // if less than 25 seconds return the height, without updating
                if elapsed < 25 {
                    return Produces::ok(block_height);
                }

                // if more than a minute return height immediately, but update the height in the background
                send!(self.addr.update_height());
                return Produces::ok(block_height);
            }
        }

        // update the height and return the new height
        let block_height = self.update_height().await?.await?;
        Produces::ok(block_height)
    }

    pub(crate) async fn update_height(&mut self) -> ActorResult<usize> {
        debug!("actor update_height");
        self.check_node_connection().await?;
        let node_client = self.node_client().await?;
        let block_height = node_client.get_height().await.map_err(|_| Error::GetHeightError)?;
        self.save_last_height_fetched(block_height);

        Produces::ok(block_height)
    }

    pub(crate) async fn update_block_id(&mut self) -> Result<BlockId> {
        debug!("actor update_block_id");
        if self.check_node_connection().await.is_err() {
            return Err(Error::GetHeightError.into());
        };

        let node_client = self.node_client().await?;
        let block_id = node_client.get_block_id().await.map_err(|_| Error::GetHeightError)?;
        self.save_last_height_fetched(block_id.height as usize);

        Ok(block_id)
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

    pub(crate) async fn node_client(&mut self) -> Result<&NodeClient, Error> {
        let node_client = self.node_client.as_ref();
        if node_client.is_none() {
            let node = Database::global().global_config.selected_node();
            let node_client = NodeClient::new(&node)
                .await
                .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)?;

            self.node_client = Some(node_client);
        }

        Ok(self.node_client.as_ref().expect("just checked"))
    }
}

async fn check_node_connection_inner(node: &Node) -> Result<(), String> {
    // create a fresh client with its own TCP connection for connection probes
    // because the actor may continue processing messages with its cached client
    // while a background check is running. the underlying rust-electrum-client
    // is not designed for concurrent access, so a fresh connection ensures no
    // shared state or concurrent access
    //
    // TODO: We could optimize this to reuse the cached client when using esplora,
    // since esplora uses HTTP and doesn't have the concurrent access limitations
    // that electrum's persistent TCP connection has.
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
