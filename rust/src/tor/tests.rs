//! The leak invariant: with Tor on, node traffic goes through the proxy or nowhere
//!
//! These drive the app's real node clients against the fake node, with a recording SOCKS5
//! proxy standing in for Tor

use bitcoin::Network;
use cove_http::NetworkRoute;

use super::{RouteError, SetTorError, TorService, TorSettings, TorStatus, current_route};
use crate::{
    database::{Database, global_config::GlobalConfigKey},
    network::Network as CoveNetwork,
    node::{Node, client::NodeClient},
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
    let proxy = RecordingSocksProxy::start();
    tor.use_proxy(proxy.proxy_address());

    assert_node_traffic_goes_through(&proxy).await;
}

// built-in Tor's bridge only serves clients that send its credentials, so a client that drops
// them fails here instead of in the field
#[tokio::test]
async fn node_traffic_authenticates_to_the_built_in_bridge() {
    let _tor = TorTest::start().await;
    let proxy = RecordingSocksProxy::start_with_credentials();
    TorService::global().publish(Ok(NetworkRoute::Tor(proxy.socks_proxy())), TorStatus::Ready);

    assert_node_traffic_goes_through(&proxy).await;
}

async fn assert_node_traffic_goes_through(proxy: &RecordingSocksProxy) {
    let node = FakeNode::start(Network::Bitcoin);
    node.chain().mine(3);

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
    assert!(matches!(TorService::global().status(), TorStatus::Failed { .. }));

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
