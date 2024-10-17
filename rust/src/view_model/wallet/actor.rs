use crate::{
    database::Database,
    node::client::NodeClient,
    transaction::{Transaction, TransactionDetails, TxId},
    view_model::wallet::Error,
    wallet::{balance::Balance, metadata::BlockSizeLast, AddressInfo, Wallet},
};
use act_zero::*;
use bdk_chain::spk_client::{FullScanResult, SyncResult};
use bdk_wallet::KeychainKind;
use crossbeam::channel::Sender;
use std::time::{Duration, UNIX_EPOCH};
use tracing::{debug, error, info};

use super::WalletViewModelReconcileMessage;

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,

    last_scan_finished_: Option<Duration>,
    last_height_fetched_: Option<(Duration, usize)>,

    pub state: ActorState,
}

#[derive(Debug)]
pub enum ActorState {
    Initial,
    PerformingFullScan,
    PerformingIncrementalScan,
    ScanComplete,
}

#[async_trait::async_trait]
impl Actor for WalletActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletActor Error: {error:?}");
        let error_string = error.to_string();

        // an error occurred, that wasn't a wallet error, send unknown error
        let Some(error) = error.downcast::<Error>().ok().map(|e| *e) else {
            self.send(WalletViewModelReconcileMessage::UnknownError(error_string));
            return false;
        };

        match error {
            Error::NodeConnectionFailed(error_string) => {
                self.send(WalletViewModelReconcileMessage::NodeConnectionFailed(
                    error_string,
                ));
            }

            _ => {
                self.send(WalletViewModelReconcileMessage::WalletError(error));
            }
        };

        false
    }
}

impl WalletActor {
    pub fn new(wallet: Wallet, reconciler: Sender<WalletViewModelReconcileMessage>) -> Self {
        Self {
            addr: Default::default(),
            reconciler,
            wallet,
            node_client: None,
            last_scan_finished_: None,
            last_height_fetched_: None,
            state: ActorState::Initial,
        }
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    pub async fn transactions(&mut self) -> ActorResult<Vec<Transaction>> {
        let mut transactions = self
            .wallet
            .transactions()
            .map(|tx| Transaction::new(&self.wallet, tx))
            .collect::<Vec<Transaction>>();

        transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());

        Produces::ok(transactions)
    }

    pub async fn next_address(&mut self) -> ActorResult<AddressInfo> {
        let address = self.wallet.get_next_address()?;
        Produces::ok(address)
    }

    pub async fn check_node_connection(&mut self) -> ActorResult<()> {
        let node_client = match &self.node_client {
            Some(node_client) => node_client,
            None => {
                let node = Database::global().global_config.selected_node();
                let node_client = NodeClient::new(&node).await?;
                self.node_client = Some(node_client);

                self.node_client.as_ref().expect("just checked")
            }
        };

        node_client
            .check_url()
            .await
            .map_err(|error| Error::NodeConnectionFailed(error.to_string()))?;

        Produces::ok(())
    }

    pub async fn wallet_scan_and_notify(&mut self, force_scan: bool) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;
        debug!("wallet_scan_and_notify");

        // get the initial balance and transactions
        {
            let initial_balance = self
                .balance()
                .await?
                .await
                .map_err(|error| Error::WalletBalanceError(error.to_string()))?;

            self.send(Msg::WalletBalanceChanged(initial_balance));

            let initial_transactions = self
                .transactions()
                .await?
                .await
                .map_err(|error| Error::TransactionsRetrievalError(error.to_string()))?;

            self.send(Msg::AvailableTransactions(initial_transactions))
        }

        // start the wallet scan in a background task
        self.start_wallet_scan_in_task(force_scan)
            .await?
            .await
            .map_err(|error| Error::WalletScanError(error.to_string()))?;

        Produces::ok(())
    }

    pub async fn start_wallet_scan_in_task(&mut self, force_scan: bool) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;
        debug!("start_wallet_scan");

        if !force_scan {
            if let Some(last_scan) = self.last_scan_finished() {
                if elapsed_secs_since(last_scan) < 60 {
                    info!("skipping wallet scan, last scan was less than 60 seconds ago");
                    return Produces::ok(());
                }
            }
        }

        self.reconciler.send(Msg::StartedWalletScan).unwrap();

        let node = Database::global().global_config.selected_node();
        let reconciler = self.reconciler.clone();

        // save the node client
        match NodeClient::new(&node).await {
            Ok(client) => {
                self.node_client = Some(client);
            }
            Err(error) => {
                reconciler
                    .send(Msg::NodeConnectionFailed(error.to_string()))
                    .unwrap();

                return Err(error.into());
            }
        }

        assert!(self.node_client.is_some());

        // check the node connection, and send frontend the error if it fails
        send!(self.addr.check_node_connection());

        // perform that scanning in a background task
        let addr = self.addr.clone();
        if self.wallet.metadata.performed_full_scan {
            send!(addr.perform_incremental_scan());
        } else {
            send!(addr.perform_full_scan());
        }

        Produces::ok(())
    }

    pub async fn get_height(&mut self, force: bool) -> ActorResult<usize> {
        if !force {
            if let Some((last_height_fetched, block_height)) = self.last_height_fetched() {
                let elapsed = elapsed_secs_since(last_height_fetched);
                if elapsed < 60 * 5 {
                    if elapsed < 60 {
                        return Produces::ok(block_height);
                    }

                    send!(self.addr.update_height());
                    return Produces::ok(block_height);
                }
            }
        }

        let block_height = self.update_height().await?.await?;
        Produces::ok(block_height)
    }

    async fn update_height(&mut self) -> ActorResult<usize> {
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?;

        let block_height = node_client
            .get_height()
            .await
            .map_err(|_| Error::GetHeightError)?;

        self.set_last_height_fetched(block_height);
        Produces::ok(block_height)
    }

    pub async fn transaction_details(&mut self, tx_id: TxId) -> ActorResult<TransactionDetails> {
        let tx = self
            .wallet
            .get_tx(tx_id.0)
            .ok_or(Error::TransactionDetailsError(
                "transaction not found".to_string(),
            ))?;

        let details = TransactionDetails::try_new(&self.wallet, tx)
            .map_err(|error| Error::TransactionDetailsError(error.to_string()))?;

        Produces::ok(details)
    }

    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        debug!("starting full scan");

        self.state = ActorState::PerformingFullScan;
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

        let full_scan_request = self.wallet.start_full_scan().build();

        let graph = self.wallet.tx_graph().clone();
        let node_client = self
            .node_client
            .clone()
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?
            .clone();

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let full_scan_result = node_client
                .start_wallet_scan(&graph, full_scan_request)
                .await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done full scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_full_scan_complete(full_scan_result));
        });

        Produces::ok(())
    }

    async fn perform_incremental_scan(&mut self) -> ActorResult<()> {
        debug!("starting incremental scan");
        self.state = ActorState::PerformingIncrementalScan;

        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

        let scan_request = self.wallet.start_sync_with_revealed_spks().build();
        let graph = self.wallet.tx_graph().clone();
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?
            .clone();

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let sync_result = node_client.sync(&graph, scan_request).await;
            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done incremental scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_incremental_scan_complete(sync_result));
        });

        Produces::ok(())
    }

    async fn handle_full_scan_complete(
        &mut self,
        full_scan_result: Result<FullScanResult<KeychainKind>, crate::node::client::Error>,
    ) -> ActorResult<()> {
        debug!("applying full scan result");

        let full_scan_result = full_scan_result?;

        self.wallet.apply_update(full_scan_result)?;
        self.wallet.persist()?;
        self.set_last_scan_finished();

        self.wallet.metadata.performed_full_scan = true;
        Database::global()
            .wallets
            .update_wallet_metadata(self.wallet.metadata.clone())?;

        self.mark_and_notify_scan_complete().await?;

        Produces::ok(())
    }

    async fn handle_incremental_scan_complete(
        &mut self,
        sync_result: Result<SyncResult, crate::node::client::Error>,
    ) -> ActorResult<()> {
        let sync_result = sync_result?;
        self.wallet.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.set_last_scan_finished();

        self.mark_and_notify_scan_complete().await?;

        Produces::ok(())
    }

    /// Mark the wallet as scanned
    /// Notify the frontend that the wallet scan is complete
    /// Ssend the wallet balance and transactions
    async fn mark_and_notify_scan_complete(&mut self) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;

        // set the scan state to complete
        self.state = ActorState::ScanComplete;

        // get and send wallet balance
        let balance = self
            .balance()
            .await?
            .await
            .map_err(|error| Error::WalletBalanceError(error.to_string()))?;

        self.send(Msg::WalletBalanceChanged(balance));

        // get and send transactions
        let transactions: Vec<Transaction> = self
            .transactions()
            .await?
            .await
            .map_err(|error| Error::TransactionsRetrievalError(error.to_string()))?;

        self.send(Msg::ScanComplete(transactions));

        Produces::ok(())
    }

    fn last_scan_finished(&mut self) -> Option<Duration> {
        if let Some(last_scan_finished) = self.last_scan_finished_ {
            return Some(last_scan_finished);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network)
            .ok()??;

        let last_scan_finished = metadata.internal().last_scan_finished;
        self.last_scan_finished_ = last_scan_finished;

        last_scan_finished
    }

    fn set_last_scan_finished(&mut self) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_scan_finished_ = Some(now);

        let wallets = Database::global().wallets();

        let mut metadata = wallets.get(&self.wallet.id, self.wallet.network).ok()??;
        metadata.internal_mut().last_scan_finished = Some(now);

        wallets.save_wallet(metadata).ok()
    }

    fn last_height_fetched(&mut self) -> Option<(Duration, usize)> {
        if let Some(last_height_fetched) = self.last_height_fetched_ {
            return Some(last_height_fetched);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network)
            .ok()??;

        let BlockSizeLast {
            block_height,
            last_seen,
        } = &metadata.internal().last_height_fetched?;

        let last_height_fetched = Some((*last_seen, *(block_height) as usize));
        self.last_height_fetched_ = last_height_fetched;

        last_height_fetched
    }

    fn set_last_height_fetched(&mut self, block_height: usize) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_height_fetched_ = Some((now, block_height));

        let wallets = Database::global().wallets();
        let mut metadata = wallets.get(&self.wallet.id, self.wallet.network).ok()??;

        metadata.internal_mut().last_height_fetched = Some(BlockSizeLast {
            block_height: block_height as u64,
            last_seen: now,
        });

        wallets.save_wallet(metadata).ok()
    }
}

fn elapsed_secs_since(earlier: Duration) -> u64 {
    let now = UNIX_EPOCH.elapsed().expect("time went backwards");
    (now - earlier).as_secs()
}

impl WalletActor {
    fn send(&self, msg: WalletViewModelReconcileMessage) {
        self.reconciler.send(msg).unwrap();
    }
}

impl Drop for WalletActor {
    fn drop(&mut self) {
        debug!("[DROP] Wallet Actor");
    }
}
