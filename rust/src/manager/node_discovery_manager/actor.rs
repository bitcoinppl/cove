use std::sync::Arc;

use act_zero::{Actor, ActorResult, Addr, AddrLike as _, Produces, WeakAddr, send};
use cove_tokio::FutureTimeoutExt as _;
use futures::StreamExt as _;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::{Message, NodeDiscoveryState};
use crate::{
    manager::reconcile_channel::ReconcileChannel,
    network::Network,
    node_discovery::{LanInterface, NodeDiscoveryError, SEARCH_TIME_LIMIT, Search, Verified},
};

/// The only writer of the search state, so a stopped or replaced search can't change it
#[derive(Debug)]
pub struct NodeDiscoveryActor {
    addr: WeakAddr<Self>,
    state: Arc<RwLock<NodeDiscoveryState>>,
    reconciler: ReconcileChannel<Message>,
    last_generation: u64,
    running: Option<RunningSearch>,
}

/// The search that may still change the state
#[derive(Debug)]
struct RunningSearch {
    generation: u64,
    cancel: CancellationToken,
    /// The rank of each found server, in the order they are listed
    ranks: Vec<usize>,
}

/// What a search looks through
#[derive(Debug)]
pub enum SearchPlan {
    /// Sweep these Wi-Fi or Ethernet interfaces
    Sweep(Vec<LanInterface>),
    /// Check these servers instead of sweeping, for the UI tests' scripted searches and the
    /// manager's tests, staying open afterwards when `keeps_searching`
    #[cfg(debug_assertions)]
    Check { search: Search, keeps_searching: bool },
    /// Fail at once, for a scripted search that can't run
    #[cfg(debug_assertions)]
    Fail(NodeDiscoveryError),
}

/// How a search's work ended
#[derive(Debug)]
pub enum SearchEnd {
    Finished,
    TimedOut,
    Failed(NodeDiscoveryError),
}

#[async_trait::async_trait]
impl Actor for NodeDiscoveryActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl NodeDiscoveryActor {
    pub fn new(
        state: Arc<RwLock<NodeDiscoveryState>>,
        reconciler: ReconcileChannel<Message>,
    ) -> Self {
        Self { addr: WeakAddr::default(), state, reconciler, last_generation: 0, running: None }
    }

    pub async fn start(&mut self, network: Network, plan: SearchPlan) -> ActorResult<()> {
        if self.running.is_some() {
            debug!("A node search is already running");
            return Produces::ok(());
        }

        self.last_generation += 1;
        let generation = self.last_generation;
        let cancel = CancellationToken::new();
        self.running =
            Some(RunningSearch { generation, cancel: cancel.clone(), ranks: Vec::new() });

        *self.state.write() = NodeDiscoveryState::Searching { found: Vec::new() };
        self.reconciler.send(Message::Started);

        self.addr.send_fut_with(move |addr| async move {
            let work = search(network, plan, generation, addr.clone());
            let end = tokio::select! {
                () = cancel.cancelled() => return,
                outcome = work.with_timeout(SEARCH_TIME_LIMIT) => match outcome {
                    Ok(Ok(())) => SearchEnd::Finished,
                    Ok(Err(error)) => SearchEnd::Failed(error),
                    Err(_) => SearchEnd::TimedOut,
                },
            };

            send!(addr.ended(generation, end));
        });

        Produces::ok(())
    }

    pub async fn stop(&mut self) -> ActorResult<()> {
        let Some(running) = self.running.take() else { return Produces::ok(()) };
        running.cancel.cancel();

        self.update_state(|state| match state {
            NodeDiscoveryState::Searching { found } => NodeDiscoveryState::Stopped { found },
            other => other,
        });

        self.reconciler.send(Message::Stopped);
        Produces::ok(())
    }

    pub async fn shutdown(&mut self) -> ActorResult<()> {
        if let Some(running) = self.running.take() {
            running.cancel.cancel();
        }

        Produces::ok(())
    }

    pub async fn found(&mut self, generation: u64, verified: Verified) -> ActorResult<()> {
        let Some(running) =
            self.running.as_mut().filter(|running| running.generation == generation)
        else {
            return Produces::ok(());
        };

        // servers stay in the search's order, wherever each check finished
        let index = running.ranks.partition_point(|rank| *rank < verified.rank);
        running.ranks.insert(index, verified.rank);

        let server = verified.found;
        {
            let mut state = self.state.write();
            if let NodeDiscoveryState::Searching { found } = &mut *state {
                found.insert(index, server.clone());
            }
        }

        let index = u32::try_from(index).unwrap_or(u32::MAX);
        self.reconciler.send(Message::Found { index, server });
        Produces::ok(())
    }

    pub async fn ended(&mut self, generation: u64, end: SearchEnd) -> ActorResult<()> {
        if self.running.as_ref().is_none_or(|running| running.generation != generation) {
            return Produces::ok(());
        }

        self.running = None;

        match end {
            SearchEnd::Finished | SearchEnd::TimedOut => {
                if matches!(end, SearchEnd::TimedOut) {
                    info!("Node search reached its {SEARCH_TIME_LIMIT:?} limit");
                }

                self.update_state(|state| NodeDiscoveryState::Finished {
                    found: state.found().to_vec(),
                });

                self.reconciler.send(Message::Finished);
            }

            SearchEnd::Failed(error) => {
                *self.state.write() = NodeDiscoveryState::Failed { error: error.clone() };
                self.reconciler.send(Message::Failed { error });
            }
        }

        Produces::ok(())
    }

    fn update_state(&self, update: impl FnOnce(NodeDiscoveryState) -> NodeDiscoveryState) {
        let mut state = self.state.write();
        let current = std::mem::replace(&mut *state, NodeDiscoveryState::Idle);
        *state = update(current);
    }
}

/// Sweeps, then reports each server to the actor as its check passes
async fn search(
    network: Network,
    plan: SearchPlan,
    generation: u64,
    addr: WeakAddr<NodeDiscoveryActor>,
) -> Result<(), NodeDiscoveryError> {
    match plan {
        SearchPlan::Sweep(interfaces) => {
            let search = Search::sweep(network, &interfaces).await?;
            report_verified(search, generation, &addr).await;
        }

        #[cfg(debug_assertions)]
        SearchPlan::Check { search, keeps_searching } => {
            report_verified(search, generation, &addr).await;

            if keeps_searching {
                std::future::pending::<()>().await;
            }
        }

        #[cfg(debug_assertions)]
        SearchPlan::Fail(error) => return Err(error),
    }

    Ok(())
}

async fn report_verified(search: Search, generation: u64, addr: &WeakAddr<NodeDiscoveryActor>) {
    let verified = search.verified();
    futures::pin_mut!(verified);

    while let Some(server) = verified.next().await {
        send!(addr.found(generation, server));
    }
}
