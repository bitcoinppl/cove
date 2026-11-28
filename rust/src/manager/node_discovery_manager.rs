//! Runs the node settings screen's searches for Electrum servers on the local network
//!
//! The screen owns one manager while it is shown. Each search the person starts sweeps the
//! interfaces the platform reports, checks every open port, and lists each server for the
//! selected network as its check passes. Stop ends the search and keeps what it found, a search
//! ends on its own after
//! [`SEARCH_TIME_LIMIT`](crate::node_discovery::SEARCH_TIME_LIMIT), and dropping the manager cancels it

mod actor;

use std::sync::Arc;

use act_zero::{Addr, send};
use parking_lot::RwLock;

use crate::{
    database::Database,
    manager::reconcile_channel::{ReconcileChannel, impl_reconcile_sink},
    network::Network,
    node_discovery::{DiscoveredNode, LanInterface, NodeDiscoveryError},
};

use actor::{NodeDiscoveryActor, SearchPlan};

type Message = NodeDiscoveryReconcileMessage;
type Reconciler = dyn NodeDiscoveryManagerReconciler;

/// Where a search stands, with the servers it has found
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NodeDiscoveryState {
    /// No search has started
    Idle,
    /// A search is running, and has found these servers so far
    Searching { found: Vec<DiscoveredNode> },
    /// The person stopped the search, which keeps the servers it had found
    Stopped { found: Vec<DiscoveredNode> },
    /// The search checked everything it found, or reached its time limit
    Finished { found: Vec<DiscoveredNode> },
    /// The search couldn't run
    Failed { error: NodeDiscoveryError },
}

impl NodeDiscoveryState {
    /// The servers found so far, empty when no search ran or it failed
    pub fn found(&self) -> &[DiscoveredNode] {
        match self {
            Self::Idle | Self::Failed { .. } => &[],
            Self::Searching { found } | Self::Stopped { found } | Self::Finished { found } => found,
        }
    }
}

/// A change to the search, applied in order to the state the screen started from
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NodeDiscoveryReconcileMessage {
    /// A new search started, clearing the previous one's servers
    Started,
    /// A server passed its check; insert it at `index` in the found servers
    Found { index: u32, server: DiscoveredNode },
    /// The person stopped the search, which keeps its servers
    Stopped,
    /// The search ended, keeping its servers
    Finished,
    /// The search couldn't run
    Failed { error: NodeDiscoveryError },
}

#[uniffi::export(callback_interface)]
pub trait NodeDiscoveryManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

impl_reconcile_sink!(dyn NodeDiscoveryManagerReconciler, Message, many);

/// Searches the local network for Electrum servers, one search at a time
#[derive(Debug, uniffi::Object)]
pub struct RustNodeDiscoveryManager {
    actor: Addr<NodeDiscoveryActor>,
    state: Arc<RwLock<NodeDiscoveryState>>,
    reconciler: ReconcileChannel<Message>,
}

#[uniffi::export]
impl RustNodeDiscoveryManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        let state = Arc::new(RwLock::new(NodeDiscoveryState::Idle));
        let reconciler = ReconcileChannel::new(100);
        let actor = NodeDiscoveryActor::new(state.clone(), reconciler.clone());

        Arc::new(Self { actor: cove_tokio::task::spawn_actor(actor), state, reconciler })
    }

    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen_sink_async(reconciler);
    }

    /// The search as it stands, read once when the screen starts listening
    pub fn state(&self) -> NodeDiscoveryState {
        self.state.read().clone()
    }

    /// Searches the given interfaces for servers on the selected network, unless a search is
    /// already running
    ///
    /// Pass the device's Wi-Fi and Ethernet interfaces, never cellular or VPN ones. The search
    /// writes only Electrum's `server.version`, `server.features` and block header 0 requests,
    /// so no server learns anything about the wallet until the person picks one and it goes
    /// through `check_and_save_node`
    pub fn start(&self, interfaces: Vec<LanInterface>) {
        let network = Database::global().global_config.selected_network();
        send!(self.actor.start(network, search_plan(network, interfaces)));
    }

    /// Stops the running search, keeping the servers it found
    pub fn stop(&self) {
        send!(self.actor.stop());
    }
}

/// The sweep of the given interfaces, or the UI test's scripted search while it sets one
fn search_plan(network: Network, interfaces: Vec<LanInterface>) -> SearchPlan {
    #[cfg(debug_assertions)]
    if let Some(scripted) = crate::node_discovery::ui_test_search::next_search(network) {
        return match scripted {
            Ok(scripted) => SearchPlan::Check {
                search: scripted.search,
                keeps_searching: scripted.keeps_searching,
            },
            Err(error) => SearchPlan::Fail(error),
        };
    }

    #[cfg(not(debug_assertions))]
    let _ = network;

    SearchPlan::Sweep(interfaces)
}

impl Drop for RustNodeDiscoveryManager {
    fn drop(&mut self) {
        send!(self.actor.shutdown());
    }
}

#[cfg(test)]
mod tests;
