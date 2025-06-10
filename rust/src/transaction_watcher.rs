use std::{sync::Arc, time::Duration};

use act_zero::*;
use bitcoin::{Transaction, Txid};
use cove_types::Network;
use tracing::{debug, error, info};

use crate::{
    manager::wallet_manager::actor::WalletActor,
    node::{client::NodeClient, client_builder::NodeClientBuilder},
};

/// Watches for a transaction to see if it is confirmed or to waits for it to be fully confirmed (3 confirmations)

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TransactionWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    tx_id: Arc<Txid>,
    client_builder: NodeClientBuilder,
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
            network,
        }
    }

    pub async fn start_watching(&mut self) -> ActorResult<()> {
        debug!("start_watching for txn {}", self.tx_id);
        let client = Arc::new(self.client_builder.build().await?);
        let addr = self.addr.clone();
        let manager = self.wallet_actor.clone();
        let tx_id = self.tx_id.clone();

        let normal_wait_time = match self.network {
            Network::Bitcoin => Duration::from_secs(20),
            Network::Testnet | Network::Testnet4 => Duration::from_secs(20),
            Network::Signet => Duration::from_secs(10),
        };

        self.addr.send_fut(async move {
            let client = client;
            loop {
                debug!("checking txn: {tx_id}");
                let result = call!(addr.check_txn(client.clone())).await;

                match result {
                    Ok(WatchResult::Found(txn)) => {
                        let tx_id = txn.compute_txid();
                        info!("found txn: {}", tx_id);
                        send!(manager.mark_transaction_found(tx_id));
                        break;
                    }

                    // sleep for 10 seconds before checking again
                    Ok(WatchResult::Continue) => {
                        debug!("continue watching, waiting for {}", normal_wait_time.as_secs());
                        tokio::time::sleep(normal_wait_time).await;
                    }

                    // sleep for 30 seconds before checking again, if we get an error
                    Err(error) => {
                        error!("failed to check txn: {error:?}");
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
        });

        Produces::ok(())
    }

    async fn check_txn(&mut self, client: Arc<NodeClient>) -> ActorResult<WatchResult> {
        let txn = client.get_confirmed_transaction(self.tx_id.clone()).await?;
        match txn {
            Some(txn) => Produces::ok(WatchResult::Found(txn)),
            None => Produces::ok(WatchResult::Continue),
        }
    }
}
