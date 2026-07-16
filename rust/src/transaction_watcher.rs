use std::{sync::Arc, time::Duration};

use act_zero::*;
use bitcoin::Txid;
use cove_types::Network;
use tracing::{debug, error, info, trace};

use crate::{
    database::Database,
    manager::wallet_manager::actor::WalletActor,
    node::{
        Node,
        client::{Error as NodeError, NodeClient, NodeClientOptions},
        client_builder::NodeClientBuilder,
    },
};

pub const TRANSACTION_WATCHER_TERMINAL_CONFIRMATIONS: u32 = 3;

/// Watches a transaction until it reaches the terminal confirmation count
#[derive(Debug)]
pub struct TransactionWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    tx_id: Txid,
    options: NodeClientOptions,
    network: Network,
    connection: Option<TransactionWatcherConnection>,
    keep_watching: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum TransactionWatcherEvent {
    ConfirmedObserved { tx_id: Txid },
}

#[derive(Debug, Clone)]
struct TransactionWatcherConnection {
    node: Node,
    client: NodeClient,
}

#[derive(Debug)]
enum TransactionWatcherPollResult {
    Pending(TransactionWatcherConnection),
    Confirmed(TransactionWatcherConnection),
    Failed(NodeError),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TransactionWatcherPollKind {
    Pending,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct TransactionWatcherPollPlan {
    keep_connection: bool,
    notify_confirmed: bool,
    wait_time: Duration,
}

#[async_trait::async_trait]
impl Actor for TransactionWatcher {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        send!(self.addr.poll());

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
        options: NodeClientOptions,
        network: Network,
    ) -> Self {
        debug!("creating transaction watcher for {tx_id}");
        Self {
            wallet_actor,
            addr: Default::default(),
            tx_id,
            options,
            network,
            connection: None,
            keep_watching: true,
        }
    }

    pub async fn poll(&mut self) -> ActorResult<()> {
        if !self.keep_watching {
            return Produces::ok(());
        }

        let selected_node = Database::global().global_config.selected_node();
        let connection =
            self.connection.take().filter(|connection| connection.node == selected_node);
        let builder = NodeClientBuilder { node: selected_node, options: self.options };
        let tx_id = self.tx_id;

        trace!("checking txn: {tx_id}");
        self.addr.send_fut_with(|addr| async move {
            let result = poll_transaction(connection, builder, tx_id).await;
            send!(addr.handle_poll_result(result));
        });

        Produces::ok(())
    }

    async fn handle_poll_result(
        &mut self,
        result: TransactionWatcherPollResult,
    ) -> ActorResult<()> {
        if !self.keep_watching {
            return Produces::ok(());
        }

        let kind = match result {
            TransactionWatcherPollResult::Pending(connection) => {
                self.connection = Some(connection);
                TransactionWatcherPollKind::Pending
            }

            TransactionWatcherPollResult::Confirmed(connection) => {
                self.connection = Some(connection);
                TransactionWatcherPollKind::Confirmed
            }

            TransactionWatcherPollResult::Failed(error) => {
                error!("failed to check txn {}: {error:?}", self.tx_id);
                self.connection = None;
                TransactionWatcherPollKind::Failed
            }
        };

        let plan = poll_plan(kind, self.normal_wait_time());
        debug_assert_eq!(self.connection.is_some(), plan.keep_connection);

        if plan.notify_confirmed {
            info!("found txn: {}", self.tx_id);
            send!(self.wallet_actor.handle_transaction_watcher_event(
                TransactionWatcherEvent::ConfirmedObserved { tx_id: self.tx_id }
            ));
        }

        self.schedule_poll(plan.wait_time);

        Produces::ok(())
    }

    pub async fn stop_watching(&mut self) -> ActorResult<()> {
        debug!("stop_watching for txn {}", self.tx_id);
        self.keep_watching = false;
        self.connection = None;
        Produces::ok(())
    }

    fn schedule_poll(&self, wait_time: Duration) {
        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep(wait_time).await;
            send!(addr.poll());
        });
    }

    fn normal_wait_time(&self) -> Duration {
        match self.network {
            Network::Bitcoin | Network::Testnet | Network::Testnet4 => Duration::from_secs(20),
            Network::Signet => Duration::from_secs(10),
        }
    }
}

#[cfg(test)]
mod test_support {
    use super::*;

    impl TransactionWatcher {
        pub(crate) async fn probe(&mut self) -> ActorResult<()> {
            Produces::ok(())
        }
    }
}

fn poll_plan(
    kind: TransactionWatcherPollKind,
    normal_wait_time: Duration,
) -> TransactionWatcherPollPlan {
    match kind {
        TransactionWatcherPollKind::Pending => TransactionWatcherPollPlan {
            keep_connection: true,
            notify_confirmed: false,
            wait_time: normal_wait_time,
        },
        TransactionWatcherPollKind::Confirmed => TransactionWatcherPollPlan {
            keep_connection: true,
            notify_confirmed: true,
            wait_time: normal_wait_time,
        },
        TransactionWatcherPollKind::Failed => TransactionWatcherPollPlan {
            keep_connection: false,
            notify_confirmed: false,
            wait_time: Duration::from_secs(30),
        },
    }
}

async fn poll_transaction(
    connection: Option<TransactionWatcherConnection>,
    builder: NodeClientBuilder,
    tx_id: Txid,
) -> TransactionWatcherPollResult {
    let connection = match connection {
        Some(connection) => connection,
        None => match builder.build().await {
            Ok(client) => TransactionWatcherConnection { node: builder.node, client },
            Err(error) => return TransactionWatcherPollResult::Failed(error),
        },
    };

    match connection.client.get_confirmed_transaction(Arc::new(tx_id)).await {
        Ok(Some(_)) => TransactionWatcherPollResult::Confirmed(connection),
        Ok(None) => TransactionWatcherPollResult::Pending(connection),
        Err(error) => TransactionWatcherPollResult::Failed(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_poll_discards_connection_and_retries() {
        let plan = poll_plan(TransactionWatcherPollKind::Failed, Duration::from_secs(20));

        assert!(!plan.keep_connection);
        assert!(!plan.notify_confirmed);
        assert_eq!(plan.wait_time, Duration::from_secs(30));
    }

    #[test]
    fn pending_poll_keeps_connection_without_notifying() {
        let normal_wait_time = Duration::from_secs(20);
        let plan = poll_plan(TransactionWatcherPollKind::Pending, normal_wait_time);

        assert!(plan.keep_connection);
        assert!(!plan.notify_confirmed);
        assert_eq!(plan.wait_time, normal_wait_time);
    }

    #[test]
    fn confirmed_poll_notifies_and_keeps_watching() {
        let normal_wait_time = Duration::from_secs(10);
        let plan = poll_plan(TransactionWatcherPollKind::Confirmed, normal_wait_time);

        assert!(plan.keep_connection);
        assert!(plan.notify_confirmed);
        assert_eq!(plan.wait_time, normal_wait_time);
    }
}
