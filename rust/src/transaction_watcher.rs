use std::{sync::Arc, time::Duration};

use act_zero::{runtimes::tokio::Timer, timer::Tick, *};
use bitcoin::Txid;
use cove_types::Network;
use tracing::{debug, error, info};

use crate::{
    manager::wallet_manager::actor::WalletActor,
    node::{client::NodeClient, client_builder::NodeClientBuilder},
};

/// Watches for a transaction to see if it is confirmed
pub struct TransactionWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    tx_id: Arc<Txid>,
    client_builder: NodeClientBuilder,
    timer: Timer,
    client: Option<Arc<NodeClient>>,
    normal_wait_time: Duration,
}

#[async_trait::async_trait]
impl Actor for TransactionWatcher {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();

        // build client asynchronously, don't block actor startup
        let builder = self.client_builder.clone();
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            match builder.build().await {
                Ok(client) => send!(addr.set_client(Arc::new(client))),
                Err(e) => error!("Failed to build node client: {e}"),
            }
        });

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("TransactionWatcher Error: {error:?}");
        // reschedule with error backoff
        self.timer.set_timeout_for_weak(self.addr.clone(), Duration::from_secs(30));
        false
    }
}

#[async_trait::async_trait]
impl Tick for TransactionWatcher {
    async fn tick(&mut self) -> ActorResult<()> {
        if !self.timer.tick() {
            return Produces::ok(());
        }

        let Some(client) = &self.client else {
            // client not ready yet, will be started by init task
            return Produces::ok(());
        };

        debug!("checking txn: {}", self.tx_id);
        let txn = client.get_confirmed_transaction(self.tx_id.clone()).await?;

        match txn {
            Some(txn) => {
                let tx_id = txn.compute_txid();
                info!("found txn: {tx_id}");
                send!(self.wallet_actor.mark_transaction_found(tx_id));
                self.timer.clear();
            }
            None => {
                debug!("continue watching, waiting for {}s", self.normal_wait_time.as_secs());
                self.timer.set_timeout_for_weak(self.addr.clone(), self.normal_wait_time);
            }
        }

        Produces::ok(())
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

        let normal_wait_time = match network {
            Network::Bitcoin => Duration::from_secs(20),
            Network::Testnet | Network::Testnet4 => Duration::from_secs(20),
            Network::Signet => Duration::from_secs(10),
        };

        Self {
            wallet_actor,
            addr: Default::default(),
            tx_id: Arc::new(tx_id),
            client_builder,
            timer: Default::default(),
            client: None,
            normal_wait_time,
        }
    }

    async fn set_client(&mut self, client: Arc<NodeClient>) {
        self.client = Some(client);
        // start checking immediately
        self.timer.set_timeout_for_weak(self.addr.clone(), Duration::ZERO);
    }
}
