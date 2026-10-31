use std::{sync::Arc, time::Duration};

use bitcoin::{Amount, Network, ScriptBuf};

use super::{FakeNode, unreachable_url, wait_until};
use crate::{
    network::Network as CoveNetwork,
    node::{ApiType, Node, client::NodeClient},
    test_support::ensure_tokio_runtime,
};

const SHORT: Duration = Duration::from_millis(500);

fn test_script(tag: u8) -> ScriptBuf {
    ScriptBuf::from_bytes(vec![
        0x00, 0x14, tag, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    ])
}

async fn clients(node: &FakeNode) -> Vec<(ApiType, NodeClient)> {
    ensure_tokio_runtime();

    vec![
        (ApiType::Esplora, NodeClient::new(&node.esplora_node("esplora")).await.unwrap()),
        (ApiType::Electrum, NodeClient::new(&node.electrum_node("electrum")).await.unwrap()),
    ]
}

#[tokio::test]
async fn both_protocols_report_the_shared_tip() {
    let node = FakeNode::start(Network::Bitcoin);
    node.chain().mine(7);

    for (api, client) in clients(&node).await {
        client.check_url().await.unwrap();
        assert_eq!(client.get_height().await.unwrap(), 7, "{api:?} height");
    }

    // a mutation after connecting is visible to both protocols at once
    node.chain().mine(3);
    for (api, client) in clients(&node).await {
        assert_eq!(client.get_height().await.unwrap(), 10, "{api:?} height after mining");
    }
}

#[tokio::test]
async fn offline_node_refuses_connections() {
    ensure_tokio_runtime();

    let mut node = FakeNode::start(Network::Bitcoin);
    let esplora = node.esplora_node("esplora");
    let electrum = node.electrum_node("electrum");
    node.go_offline();

    let esplora_client = NodeClient::new(&esplora).await.unwrap();
    assert!(esplora_client.check_url().await.is_err(), "esplora must be unreachable");
    assert!(NodeClient::new(&electrum).await.is_err(), "electrum must refuse the connection");

    let dead = Node::new_esplora("dead".into(), unreachable_url(), CoveNetwork::Bitcoin);
    assert!(NodeClient::new(&dead).await.unwrap().check_url().await.is_err());
}

#[tokio::test]
async fn failing_rule_surfaces_as_a_client_error_and_counts_hits() {
    let node = FakeNode::start(Network::Bitcoin);
    let esplora_failure = node.on("/blocks/tip/height").fail(400, "boom");
    let electrum_failure = node.on("blockchain.headers.subscribe").fail(500, "boom");

    for (api, client) in clients(&node).await {
        assert!(client.check_url().await.is_err(), "{api:?} must surface the error");
    }

    assert_eq!(esplora_failure.hits(), 1);
    assert_eq!(electrum_failure.hits(), 1);

    node.clear_rules();
    for (api, client) in clients(&node).await {
        client.check_url().await.unwrap_or_else(|error| panic!("{api:?} recovers: {error}"));
    }
}

#[tokio::test]
async fn stalled_requests_never_answer() {
    let node = FakeNode::start(Network::Bitcoin);
    let stall_esplora = node.on("/blocks/tip/height").stall();
    let stall_electrum = node.on("blockchain.headers.subscribe").stall();

    for (api, client) in clients(&node).await {
        let outcome = tokio::time::timeout(SHORT, client.check_url()).await;
        assert!(outcome.is_err(), "{api:?} must still be waiting");
    }

    assert_eq!(stall_esplora.hits(), 1);
    assert_eq!(stall_electrum.hits(), 1);
}

#[tokio::test]
async fn gate_holds_requests_until_opened() {
    let node = FakeNode::start(Network::Bitcoin);
    let (gate, rule) = node.on("/blocks/tip/height").hold();
    let client = clients(&node).await.remove(0).1;

    let check = tokio::spawn(async move { client.check_url().await });
    wait_until("the request to reach the gate", || gate.waiting() == 1).await;

    assert!(!check.is_finished(), "the held request must not be answered");
    assert_eq!(rule.hits(), 1);

    gate.open();
    check.await.unwrap().unwrap();
    assert_eq!(gate.waiting(), 0);
}

#[tokio::test]
async fn going_offline_releases_held_requests_from_the_gate_count() {
    let mut node = FakeNode::start(Network::Bitcoin);
    let (gate, _) = node.on("/blocks/tip/height").hold();
    let client = clients(&node).await.remove(0).1;

    let check = tokio::spawn(async move { client.check_url().await });
    wait_until("the request to reach the gate", || gate.waiting() == 1).await;

    node.go_offline();
    assert_eq!(gate.waiting(), 0);
    assert!(check.await.unwrap().is_err(), "the held request loses its connection");
}

#[tokio::test]
async fn delayed_requests_answer_after_the_delay() {
    let node = FakeNode::start(Network::Bitcoin);
    node.on("/blocks/tip/height").delay(Duration::from_millis(300));
    let client = clients(&node).await.remove(0).1;

    let started = std::time::Instant::now();
    client.check_url().await.unwrap();

    assert!(started.elapsed() >= Duration::from_millis(300));
}

/// Selects a network for one test and restores the previous selection on drop
///
/// `selected_node` follows the selected network, which other tests change
struct SelectedNetwork(CoveNetwork);

impl SelectedNetwork {
    fn select(network: CoveNetwork) -> Self {
        let global_config = &crate::database::Database::global().global_config;
        let previous = global_config.selected_network();
        global_config.set_selected_network(network).expect("selected network is set");

        Self(previous)
    }
}

impl Drop for SelectedNetwork {
    fn drop(&mut self) {
        crate::database::Database::global()
            .global_config
            .set_selected_network(self.0)
            .expect("selected network is restored");
    }
}

#[tokio::test]
async fn selecting_a_node_points_the_wallet_config_at_it() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::database::test_support::init_test_database();
    let _network = SelectedNetwork::select(CoveNetwork::Bitcoin);

    let node = FakeNode::start(Network::Bitcoin);
    let global_config = &crate::database::Database::global().global_config;

    let esplora = node.select_esplora();
    assert_eq!(global_config.selected_node(), esplora);
    assert_eq!(esplora.api_type, ApiType::Esplora);

    let electrum = node.select_electrum();
    assert_eq!(global_config.selected_node(), electrum);
    assert_eq!(electrum.api_type, ApiType::Electrum);

    global_config.set_selected_node(&Node::default(CoveNetwork::Bitcoin)).unwrap();
}

#[tokio::test]
async fn pending_transactions_are_not_reported_as_confirmed() {
    let node = FakeNode::start(Network::Bitcoin);
    let txid = node.chain().pay(&test_script(9), Amount::from_sat(5_000));
    let clients = clients(&node).await;

    // bitcoind leaves `confirmations` out of a mempool transaction's verbose reply
    for (api, client) in &clients {
        let pending = client.get_confirmed_transaction(Arc::new(txid)).await;
        assert!(matches!(pending, Ok(None)), "{api:?} pending: {pending:?}");
    }

    node.chain().confirm(txid);
    for (api, client) in &clients {
        let confirmed = client.get_confirmed_transaction(Arc::new(txid)).await.unwrap();
        assert_eq!(confirmed.map(|tx| tx.compute_txid()), Some(txid), "{api:?} confirmed");
    }
}
