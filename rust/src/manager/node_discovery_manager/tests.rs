use std::{net::SocketAddr, time::Duration};

use act_zero::call;

use super::*;
use crate::{
    manager::deferred_sender::SingleOrMany,
    node_discovery::{
        ui_test_search::loopback_search,
        verify::tests::{Behavior, serve},
    },
};

/// An actor and the state and messages it writes, driven the way the manager drives it
struct Harness {
    actor: Addr<NodeDiscoveryActor>,
    state: Arc<RwLock<NodeDiscoveryState>>,
    messages: Arc<flume::Receiver<SingleOrMany<Message>>>,
}

impl Harness {
    fn new() -> Self {
        crate::test_support::ensure_tokio_runtime();

        let state = Arc::new(RwLock::new(NodeDiscoveryState::Idle));
        let reconciler = ReconcileChannel::new(100);
        let messages = reconciler.receiver();
        let actor =
            cove_tokio::task::spawn_actor(NodeDiscoveryActor::new(state.clone(), reconciler));

        Self { actor, state, messages }
    }

    async fn start(&self, plan: SearchPlan) {
        call!(self.actor.start(Network::Signet, plan)).await.unwrap();
    }

    async fn stop(&self) {
        call!(self.actor.stop()).await.unwrap();
    }

    /// The next message, failing the test when none comes
    async fn next(&self) -> Message {
        let received = tokio::time::timeout(Duration::from_secs(10), self.messages.recv_async())
            .await
            .expect("a message")
            .unwrap();

        match received {
            SingleOrMany::Single(message) => message,
            SingleOrMany::Many(_) => panic!("the actor sends one message at a time"),
        }
    }

    fn state(&self) -> NodeDiscoveryState {
        self.state.read().clone()
    }
}

fn checks(servers: &[(&str, SocketAddr)], keeps_searching: bool) -> SearchPlan {
    let servers = servers.iter().map(|(host, address)| ((*host).to_string(), address.port()));

    SearchPlan::Check { search: loopback_search(Network::Signet, servers), keeps_searching }
}

fn hosts(found: &[DiscoveredNode]) -> Vec<&str> {
    found.iter().map(|server| server.host.as_str()).collect()
}

#[tokio::test]
async fn a_search_lists_only_servers_for_the_network_in_search_order() {
    let harness = Harness::new();
    let first = serve(Behavior::Electrum { network: Network::Signet }).await;
    let mainnet = serve(Behavior::Electrum { network: Network::Bitcoin }).await;
    let second = serve(Behavior::Electrum { network: Network::Signet }).await;

    harness
        .start(checks(&[("first", first), ("mainnet", mainnet), ("second", second)], false))
        .await;

    assert_eq!(harness.next().await, Message::Started);

    let mut found = Vec::new();
    loop {
        match harness.next().await {
            Message::Found { index, server } => found.insert(index as usize, server),
            Message::Finished => break,
            other => panic!("unexpected {other:?}"),
        }
    }

    assert_eq!(hosts(&found), vec!["first", "second"]);
    assert_eq!(harness.state(), NodeDiscoveryState::Finished { found });
}

// stopping keeps what the search found, and nothing the stopped search does afterwards can
// change the state, while starting again clears it
#[tokio::test]
async fn stop_keeps_the_servers_found_and_a_new_search_starts_clean() {
    let harness = Harness::new();
    let server = serve(Behavior::Electrum { network: Network::Signet }).await;

    harness.start(checks(&[("node", server)], true)).await;
    assert_eq!(harness.next().await, Message::Started);
    assert!(matches!(harness.next().await, Message::Found { index: 0, .. }));

    // a second start while one runs is ignored
    harness.start(checks(&[], false)).await;
    assert!(matches!(harness.state(), NodeDiscoveryState::Searching { .. }));

    harness.stop().await;
    assert_eq!(harness.next().await, Message::Stopped);

    let NodeDiscoveryState::Stopped { found } = harness.state() else {
        panic!("not stopped: {:?}", harness.state())
    };
    assert_eq!(hosts(&found), vec!["node"]);

    harness.start(checks(&[], false)).await;
    assert_eq!(harness.next().await, Message::Started);
    assert_eq!(harness.next().await, Message::Finished);
    assert_eq!(harness.state(), NodeDiscoveryState::Finished { found: Vec::new() });
}

#[tokio::test]
async fn a_search_that_cannot_run_fails_with_its_reason() {
    let harness = Harness::new();

    harness.start(SearchPlan::Sweep(Vec::new())).await;

    assert_eq!(harness.next().await, Message::Started);
    assert_eq!(harness.next().await, Message::Failed { error: NodeDiscoveryError::NoLocalNetwork });
    assert_eq!(
        harness.state(),
        NodeDiscoveryState::Failed { error: NodeDiscoveryError::NoLocalNetwork }
    );
}
