use crate::{
    database::Database,
    node::client::NodeClient,
    transaction::{Transaction, Transactions},
    wallet::{balance::Balance, Wallet},
};
use act_zero::*;
use crossbeam::channel::Sender;
use tracing::error;

use super::WalletViewModelReconcileMessage;

#[derive(Debug)]
pub struct WalletActor {
    pub addr: Addr<Self>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,
}

#[async_trait::async_trait]
impl Actor for WalletActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr;
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletActor Error: {error:?}");
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
        }
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    pub async fn start_wallet_scan(&mut self) -> ActorResult<()> {
        use WalletViewModelReconcileMessage as Msg;

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

        let node_client = self.node_client.as_ref().expect("just set it");
        let graph = self.wallet.tx_graph();

        let full_scan_request = self.wallet.start_full_scan();

        let mut full_scan_result = node_client
            .start_wallet_scan(graph, full_scan_request)
            .await?;

        let now = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        let _ = full_scan_result
            .graph_update
            .update_last_seen_unconfirmed(now);

        self.wallet.apply_update(full_scan_result)?;
        self.wallet.persist()?;

        Produces::ok(())
    }

    pub async fn transactions(&mut self) -> ActorResult<Transactions> {
        let transactions: Vec<Transaction> =
            self.wallet.transactions().map(Transaction::from).collect();

        let transactions = Transactions::from(transactions);
        Produces::ok(transactions)
    }
}
