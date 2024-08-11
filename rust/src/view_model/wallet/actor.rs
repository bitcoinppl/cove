use crate::{
    database::Database,
    node::client::NodeClient,
    transaction::Transaction,
    view_model::wallet::Error,
    wallet::{balance::Balance, Wallet},
};
use act_zero::*;
use crossbeam::channel::Sender;
use tokio::time::Instant;
use tracing::{debug, error, info};

use super::WalletViewModelReconcileMessage;

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,
    pub last_scan_finished: Option<Instant>,
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

        if let Some(error) = error.downcast::<Error>().ok().map(|e| *e) {
            self.send(WalletViewModelReconcileMessage::WalletError(error));
        } else {
            self.send(WalletViewModelReconcileMessage::UnknownError(error_string));
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
            last_scan_finished: None,
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

        transactions.sort_unstable();

        Produces::ok(transactions)
    }

    pub async fn wallet_scan_and_notify(&mut self) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;
        debug!("wallet_scan_and_notify");

        // notify the frontend that the wallet is starting to scan
        self.send(Msg::StartedWalletScan);

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

        // start the wallet scan and send balnce and transactions after scan is complete
        {
            // start the wallet scan
            self.start_wallet_scan()
                .await?
                .await
                .map_err(|error| Error::WalletScanError(error.to_string()))?;

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
        }

        Produces::ok(())
    }

    pub async fn start_wallet_scan(&mut self) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;
        debug!("start_wallet_scan");

        if let Some(last_scan) = self.last_scan_finished {
            if last_scan.elapsed().as_secs() < 10 {
                info!("skipping wallet scan, last scan was less than 10 seconds ago");
                return Produces::ok(());
            }
        }

        self.reconciler.send(Msg::StartedWalletScan).unwrap();

        let node = Database::global().global_config.selected_node();
        let reconciler = self.reconciler.clone();

        // save the node client
        match NodeClient::new_from_node(&node).await {
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

        if self.wallet.metadata.performed_full_scan {
            self.perform_incremental_scan().await?;
        } else {
            self.perform_full_scan().await?;
        }

        Produces::ok(())
    }

    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        debug!("starting full scan");
        let start = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();

        let full_scan_request = self.wallet.start_full_scan();
        let graph = self.wallet.tx_graph();
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?;

        let mut full_scan_result = node_client
            .start_wallet_scan(graph, full_scan_request)
            .await?;

        debug!("applying full scan result");
        let now = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        let _ = full_scan_result
            .graph_update
            .update_last_seen_unconfirmed(now);

        self.wallet.apply_update(full_scan_result)?;
        self.wallet.persist()?;
        self.last_scan_finished = Some(Instant::now());

        self.wallet.metadata.performed_full_scan = true;
        Database::global()
            .wallets
            .update_wallet_metadata(self.wallet.metadata.clone())?;

        let end = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done full scan in {}s", end - start);

        Produces::ok(())
    }

    async fn perform_incremental_scan(&mut self) -> ActorResult<()> {
        debug!("starting incremental scan");
        let start = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();

        let scan_request = self.wallet.start_sync_with_revealed_spks();
        let graph = self.wallet.tx_graph();
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?;

        let now = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        let mut sync_result = node_client.sync(graph, scan_request).await?;
        let _ = sync_result.graph_update.update_last_seen_unconfirmed(now);

        self.wallet.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.last_scan_finished = Some(Instant::now());

        let end = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done incremental scan in {}s", end - start);

        Produces::ok(())
    }
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
