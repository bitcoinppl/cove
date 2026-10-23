use std::time::Duration;

use bdk_electrum::electrum_client::{self, Client as ElectrumClient, ElectrumApi as _};
use bdk_esplora::esplora_client;
use bitcoin::{Amount, Network, ScriptBuf};

use super::{FakeNode, Protocol, TxLocation, unreachable_url, wait_until};
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
async fn delayed_requests_answer_after_the_delay() {
    let node = FakeNode::start(Network::Bitcoin);
    node.on("/blocks/tip/height").delay(Duration::from_millis(300));
    let client = clients(&node).await.remove(0).1;

    let started = std::time::Instant::now();
    client.check_url().await.unwrap();

    assert!(started.elapsed() >= Duration::from_millis(300));
}

#[tokio::test]
async fn request_log_records_protocol_and_scripthash() {
    ensure_tokio_runtime();

    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(1);
    node.chain().pay(&script, Amount::from_sat(1_000));

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    esplora.scripthash_txs(&script, None).await.unwrap();

    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();
    electrum.script_get_history(&script).unwrap();

    let requests = node.history_requests_for(&script);
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests.iter().any(|request| request.protocol == Protocol::Esplora));
    assert!(requests.iter().any(|request| request.protocol == Protocol::Electrum));
    assert!(node.history_requests_for(&test_script(2)).is_empty());

    node.clear_requests();
    assert!(node.requests().is_empty());
}

#[tokio::test]
async fn both_protocols_agree_on_history_and_confirmation() {
    ensure_tokio_runtime();

    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(3);
    let txid = node.chain().pay(&script, Amount::from_sat(5_000));

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    // pending: in the mempool on both protocols
    let pending = esplora.scripthash_txs(&script, None).await.unwrap();
    assert_eq!((pending.len(), pending[0].txid, pending[0].status.confirmed), (1, txid, false));
    assert_eq!(pending[0].vout[0].value, 5_000);
    assert_eq!(pending[0].fee, 1_000);

    let pending = electrum.script_get_history(&script).unwrap();
    assert_eq!((pending.len(), pending[0].tx_hash, pending[0].height), (1, txid, 0));

    // confirmed: both protocols move to the same block, with a valid merkle proof
    node.chain().mine(2);
    let height = node.chain().confirm(txid);
    assert_eq!(node.chain().location(txid), Some(TxLocation::Confirmed { height }));

    let status = esplora.get_tx_status(&txid).await.unwrap();
    assert_eq!((status.confirmed, status.block_height), (true, Some(height)));
    assert_eq!(
        esplora.get_header_by_hash(&status.block_hash.unwrap()).await.unwrap(),
        node.chain().header(height).unwrap()
    );

    let history = electrum.script_get_history(&script).unwrap();
    assert_eq!(history[0].height, height as i32);

    let header = electrum.block_header(height as usize).unwrap();
    let proof = electrum.transaction_get_merkle(&txid, height as usize).unwrap();
    assert!(electrum_client::utils::validate_merkle_proof(&txid, &header.merkle_root, &proof));

    let esplora_proof = esplora.get_merkle_proof(&txid).await.unwrap().unwrap();
    assert_eq!((esplora_proof.block_height, esplora_proof.pos), (height, proof.pos));

    // evicting a pending transaction removes it from both
    let evicted = node.chain().pay(&test_script(4), Amount::from_sat(1));
    node.chain().evict(evicted);
    assert!(esplora.get_tx_status(&evicted).await.is_err());
    assert!(electrum.transaction_get(&evicted).is_err());
}

#[tokio::test]
async fn selecting_a_node_points_the_wallet_config_at_it() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::database::test_support::init_test_database();

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
