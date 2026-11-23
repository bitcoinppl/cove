//! The leak invariant: with Tor on, node traffic goes through the proxy or nowhere
//!
//! These drive the app's real node clients against the fake node, with a recording SOCKS5
//! proxy standing in for Tor

use bdk_wallet::KeychainKind;
use bdk_wallet::chain::{TxGraph, spk_client::SyncRequest};
use bitcoin::Network;

use super::{RouteError, SetTorError, TorService, TorSettings, TorStatus, current_route};
use crate::{
    database::{Database, global_config::GlobalConfigKey},
    network::Network as CoveNetwork,
    node::{
        Node,
        client::{Error as NodeClientError, NodeClient},
    },
    test_support::{
        fake_node::FakeNode,
        socks_proxy::{
            ConnectionCounter, PROXY_ONLY_HOST, ProxiedTarget, RecordingSocksProxy, TorTest,
            closed_proxy_address,
        },
    },
};

/// `node` with its host swapped for one only the test proxy can resolve
fn behind_proxy(node: Node) -> Node {
    Node { url: node.url.replace("127.0.0.1", PROXY_ONLY_HOST), ..node }
}

fn port_of(node: &Node) -> u16 {
    url::Url::parse(&node.url).unwrap().port().unwrap()
}

#[tokio::test]
async fn node_traffic_goes_through_the_proxy_with_names_resolved_there() {
    let tor = TorTest::start().await;
    let node = FakeNode::start(Network::Bitcoin);
    node.chain().mine(3);

    let proxy = RecordingSocksProxy::start();
    tor.use_proxy(proxy.proxy_address());

    let esplora = behind_proxy(node.esplora_node("esplora"));
    let electrum = behind_proxy(node.electrum_node("electrum"));

    for node in [&esplora, &electrum] {
        node.check_url().await.unwrap();

        let client = NodeClient::new(node).await.unwrap();
        assert_eq!(client.get_height().await.unwrap(), 3);
    }

    // every stream carried the name, so nothing was looked up outside the proxy
    let targets = proxy.targets();
    for node in [&esplora, &electrum] {
        let expected = ProxiedTarget::Name(PROXY_ONLY_HOST.to_string(), port_of(node));
        assert!(targets.contains(&expected), "{expected:?} missing from {targets:?}");
    }

    assert!(
        targets.iter().all(|target| matches!(target, ProxiedTarget::Name(..))),
        "a client resolved a name itself: {targets:?}"
    );
}

#[tokio::test]
async fn a_down_proxy_fails_connections_instead_of_going_direct() {
    let tor = TorTest::start().await;
    let server = ConnectionCounter::start();
    tor.use_proxy(closed_proxy_address());

    let address = server.address();
    let nodes = [
        Node::new_esplora("esplora".into(), format!("http://{address}/api"), CoveNetwork::Bitcoin),
        Node::new_electrum("electrum".into(), format!("tcp://{address}"), CoveNetwork::Bitcoin),
    ];

    for node in &nodes {
        assert!(node.check_url().await.is_err(), "{} connected without the proxy", node.name);
    }

    assert_eq!(server.count(), 0, "a client reached the server outside Tor");
}

#[tokio::test]
async fn onion_nodes_need_tor() {
    let _tor = TorTest::start().await;
    let onion = Node::new_electrum(
        "onion".into(),
        "tcp://abcdefghijklmnopqrstuvwxyz234567abcdefghijklmnopqrstuvwx.onion:50001".into(),
        CoveNetwork::Bitcoin,
    );

    let error = NodeClient::new(&onion).await.unwrap_err();
    assert!(
        matches!(error, crate::node::client::Error::Route(RouteError::OnionNeedsTor)),
        "{error}"
    );
}

#[tokio::test]
async fn tor_stays_on_while_an_onion_node_is_selected() {
    let tor = TorTest::start().await;
    tor.use_proxy(closed_proxy_address());

    let onion = Node::new_electrum(
        "My onion node".into(),
        "tcp://abcdefghijklmnopqrstuvwxyz234567abcdefghijklmnopqrstuvwx.onion:50001".into(),
        CoveNetwork::Bitcoin,
    );
    Database::global().global_config.set_selected_node(&onion).unwrap();

    let error = TorService::global().set_settings(TorSettings::Off).unwrap_err();
    assert_eq!(error, SetTorError::OnionNodeSelected { node_name: "My onion node".into() });
    assert!(current_route().unwrap().is_tor(), "a refused change must leave Tor on");
}

#[tokio::test]
async fn unreadable_settings_block_every_connection() {
    let _tor = TorTest::start().await;
    let server = ConnectionCounter::start();

    Database::global().global_config.set(GlobalConfigKey::TorSettings, "{not json".into()).unwrap();
    TorService::global().restart();

    assert!(matches!(current_route(), Err(RouteError::TorUnavailable(_))));
    assert!(matches!(TorService::global().subscribe().borrow().status, TorStatus::Failed { .. }));

    let node = Node::new_esplora(
        "esplora".into(),
        format!("http://{}/api", server.address()),
        CoveNetwork::Bitcoin,
    );
    assert!(node.check_url().await.is_err());
    assert_eq!(server.count(), 0, "a client connected while the Tor setting was unreadable");
}

#[tokio::test]
async fn cached_clients_stop_matching_when_tor_turns_on() {
    let tor = TorTest::start().await;
    let node = FakeNode::start(Network::Bitcoin);
    let esplora = node.esplora_node("esplora");

    let direct_client = NodeClient::new(&esplora).await.unwrap();
    assert_eq!(direct_client.connection_identity(), &esplora.connection_identity());

    let proxy = RecordingSocksProxy::start();
    tor.use_proxy(proxy.proxy_address());

    // the wallet and the transaction watcher drop a client whose identity no longer matches
    assert_ne!(direct_client.connection_identity(), &esplora.connection_identity());
}

#[tokio::test]
async fn a_client_built_before_tor_turns_on_stops_instead_of_going_direct() {
    let tor = TorTest::start().await;
    let server = ConnectionCounter::start();
    let node = Node::new_esplora(
        "esplora".into(),
        format!("http://{}/api", server.address()),
        CoveNetwork::Bitcoin,
    );

    // built on the direct route; an Esplora client connects only when it sends a request
    let client = NodeClient::new(&node).await.unwrap();
    tor.use_proxy(closed_proxy_address());

    let error = client.get_height().await.unwrap_err();
    assert!(matches!(error, crate::node::client::Error::Route(RouteError::Changed)), "{error}");
    assert_eq!(server.count(), 0, "a client built before Tor turned on reached the server");
}

// a sync sends one request per address, so a sync that started on the direct route used to
// send the rest of them outside Tor after Tor turned on
#[tokio::test]
async fn a_running_sync_stops_when_tor_turns_on() {
    let tor = TorTest::start().await;

    // takes connections and never answers, so the sync is still running when Tor turns on
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api", silent.local_addr().unwrap());
    let node = Node::new_esplora("esplora".into(), url, CoveNetwork::Bitcoin);
    let client = NodeClient::new(&node).await.unwrap();

    let script = bitcoin::ScriptBuf::new_op_return([1; 20]);
    let request =
        SyncRequest::builder().spks_with_indexes([((KeychainKind::External, 0), script)]).build();

    let sync = tokio::spawn(async move { client.sync(&TxGraph::default(), request).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    tor.use_proxy(closed_proxy_address());

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), sync)
        .await
        .expect("the sync kept running on the direct route after Tor turned on")
        .unwrap();

    assert!(
        matches!(result, Err(NodeClientError::Route(RouteError::Changed))),
        "expected the sync to stop because the route changed, got {result:?}"
    );
    drop(silent);
}

#[tokio::test]
async fn choosing_the_running_mode_again_changes_nothing() {
    let tor = TorTest::start().await;
    let proxy = closed_proxy_address();
    tor.use_proxy(proxy.clone());

    let mut state = TorService::global().subscribe();
    state.borrow_and_update();

    // the screens send every pick, and leave it to the service to skip one that changes nothing
    TorService::global().set_settings(TorSettings::Proxy(proxy)).unwrap();
    assert!(!state.has_changed().unwrap(), "saving the running mode republished it");
}
