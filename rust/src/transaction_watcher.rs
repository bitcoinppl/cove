use std::{sync::Arc, time::Duration};

use act_zero::{runtimes::tokio::Timer, timer::Tick, *};
use bitcoin::{Transaction, Txid};
use cove_types::Network;
use tracing::{debug, error, info};

use crate::{
    manager::wallet_manager::actor::WalletActor,
    node::{client::NodeClient, client_builder::NodeClientBuilder},
};

/// Watches for a transaction to see if it is confirmed or to waits for it to be fully confirmed (3 confirmations)
pub struct TransactionWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    tx_id: Arc<Txid>,
    client_builder: NodeClientBuilder,
    timer: Timer,
    client: Option<Arc<NodeClient>>,
    network: Network,
}

/// If we should keep watching or stop
enum WatchResult {
    Found(Transaction),
    Continue,
}

#[async_trait::async_trait]
impl Actor for TransactionWatcher {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        send!(self.addr.start_watching());

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("TransactionWatcher Error: {error:?}");
        false
    }
}

impl TransactionWatcher {
    pub fn new(
        wallet_actor: WeakAddr<WalletActor>,
        tx_id: Txid,
        client_builder: NodeClientBuilder,
        network: Network,
    ) -> Self {
        debug!("creating transaction watcher for {tx_id}");
        Self {
            wallet_actor,
            addr: Default::default(),
            tx_id: Arc::new(tx_id),
            client_builder,
            timer: Timer::default(),
            client: None,
            network,
        }
    }

    fn normal_wait_time(&self) -> Duration {
        match self.network {
            Network::Bitcoin => Duration::from_secs(20),
            Network::Testnet | Network::Testnet4 => Duration::from_secs(20),
            Network::Signet => Duration::from_secs(10),
        }
    }

    pub async fn start_watching(&mut self) -> ActorResult<()> {
        debug!("start_watching for txn {}", self.tx_id);
        let client = match self.client_builder.build().await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                error!("failed to build client: {e:?}");
                self.timer.set_timeout_for_weak(self.addr.clone(), Duration::from_secs(10));
                return Produces::ok(());
            }
        };
        self.client = Some(client);

        // Trigger the first check immediately.
        self.timer.set_timeout_for_weak(self.addr.clone(), Duration::ZERO);

        Produces::ok(())
    }

    async fn check_txn(&mut self) -> Result<WatchResult, Box<dyn std::error::Error>> {
        let Some(client) = self.client.clone() else {
            error!("transaction watcher client not initialized");
            return Ok(WatchResult::Continue);
        };

        let txn = client.get_confirmed_transaction(self.tx_id.clone()).await?;
        match txn {
            Some(txn) => Ok(WatchResult::Found(txn)),
            None => Ok(WatchResult::Continue),
        }
    }
}

#[async_trait::async_trait]
impl Tick for TransactionWatcher {
    async fn tick(&mut self) -> ActorResult<()> {
        if !self.timer.tick() {
            return Produces::ok(());
        }

        debug!("checking txn: {}", self.tx_id);

        match self.check_txn().await {
            Ok(WatchResult::Found(txn)) => {
                let tx_id = txn.compute_txid();
                info!("found txn: {}", tx_id);
                send!(self.wallet_actor.mark_transaction_found(tx_id));
                send!(self.wallet_actor.remove_watcher_for_txn(tx_id));
                self.timer.clear();
            }

            Ok(WatchResult::Continue) => {
                debug!("continue watching, waiting for {}", self.normal_wait_time().as_secs());
                self.timer.set_timeout_for_weak(self.addr.clone(), self.normal_wait_time());
            }

            Err(error) => {
                error!("failed to check txn: {error:?}");
                self.timer.set_timeout_for_weak(self.addr.clone(), Duration::from_secs(30));
            }
        }

        Produces::ok(())
    }
}
