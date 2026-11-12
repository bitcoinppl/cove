use std::collections::{BTreeMap, BTreeSet};

use super::{lan_binding::LanBinding, *};
use crate::node::ApiType;

fn address(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn ip(text: &str) -> Ipv4Addr {
    text.parse().unwrap()
}

fn findings(open: &[&str], names: &[(&str, &str)], known: &[(&str, &str)]) -> SubnetFindings {
    let mut addresses: BTreeMap<String, BTreeSet<Ipv4Addr>> = BTreeMap::new();
    for (hostname, address) in known {
        addresses.entry((*hostname).to_string()).or_default().insert(ip(address));
    }

    SubnetFindings {
        open: open.iter().map(|text| address(text)).collect(),
        mdns: MdnsAnswers {
            addresses,
            names: names.iter().map(|(address, name)| (ip(address), (*name).to_string())).collect(),
        },
    }
}

fn merge_mainnet(found: Vec<SubnetFindings>) -> Vec<DiscoveredNode> {
    merge(Network::Bitcoin, &Catalog::for_network(Network::Bitcoin), found)
}

#[test]
fn a_swept_electrum_port_becomes_a_node_named_by_its_reverse_lookup() {
    let nodes = merge_mainnet(vec![findings(
        &["192.168.1.10:50001"],
        &[("192.168.1.10", "bitcoin.local")],
        &[],
    )]);

    let expected = DiscoveredNode {
        node: Node::new_electrum(
            "bitcoin.local".to_string(),
            "tcp://192.168.1.10:50001".to_string(),
            Network::Bitcoin,
        ),
        host: "bitcoin.local".to_string(),
        address: "192.168.1.10:50001".to_string(),
        software: None,
    };

    assert_eq!(nodes, vec![expected]);
}

// a TLS server would fail the check when picked, since its certificate isn't from a public
// authority, so an open TLS port on an unknown host is not offered
#[test]
fn an_unnamed_server_is_shown_by_address_and_tls_ports_are_not_offered() {
    let nodes =
        merge_mainnet(vec![findings(&["192.168.1.30:50001", "192.168.1.31:50002"], &[], &[])]);

    assert_eq!(nodes.len(), 1, "{nodes:?}");
    assert_eq!(nodes[0].host, "192.168.1.30");
    assert_eq!(nodes[0].node.name, "192.168.1.30");
    assert_eq!(nodes[0].node.url, "tcp://192.168.1.30:50001");
}

#[test]
fn a_known_distribution_uses_its_own_port_meanings_and_lists_first() {
    let nodes = merge_mainnet(vec![findings(
        &["192.168.1.5:50001", "192.168.1.20:50002", "192.168.1.20:50001"],
        &[],
        &[("umbrel.local", "192.168.1.20")],
    )]);

    let summary: Vec<_> =
        nodes.iter().map(|found| (found.node.name.as_str(), found.node.url.as_str())).collect();

    assert_eq!(
        summary,
        vec![
            ("Umbrel Electrs (umbrel.local)", "tcp://192.168.1.20:50001"),
            // Umbrel's Fulcrum serves plain TCP on 50002, unlike the usual TLS port
            ("Umbrel Fulcrum (umbrel.local)", "tcp://192.168.1.20:50002"),
            ("192.168.1.5", "tcp://192.168.1.5:50001"),
        ]
    );

    assert!(nodes.iter().all(|found| found.node.api_type == ApiType::Electrum));
}

#[test]
fn a_well_known_hostname_wins_over_a_reverse_lookup_from_another_subnet() {
    let nodes = merge_mainnet(vec![
        findings(&["192.168.1.20:50001"], &[("192.168.1.20", "node.local")], &[]),
        findings(&[], &[], &[("umbrel.local", "192.168.1.20")]),
    ]);

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].host, "umbrel.local");
    assert_eq!(nodes[0].software.as_deref(), Some("Umbrel Electrs"));
}

#[test]
fn other_networks_use_their_own_electrum_ports_and_no_distribution_ports() {
    let catalog = Catalog::for_network(Network::Signet);

    assert_eq!(catalog.generic_port_numbers(), vec![60601]);
    assert!(catalog.known_hostnames().is_empty());

    let nodes = merge(
        Network::Signet,
        &catalog,
        vec![findings(&["192.168.1.10:60601", "192.168.1.10:50001"], &[], &[])],
    );

    assert_eq!(nodes.len(), 1, "a mainnet port is not a signet server: {nodes:?}");
    assert_eq!(nodes[0].node.url, "tcp://192.168.1.10:60601");
    assert_eq!(nodes[0].node.network, Network::Signet);
}

// the node screen re-saves the selected custom node by parsing its own values again, so a
// picked discovered node must parse back to itself
#[test]
fn a_discovered_node_parses_back_to_itself_as_a_custom_node() {
    let nodes = merge_mainnet(vec![findings(
        &["192.168.1.10:50001", "192.168.1.20:50002", "192.168.1.30:50001"],
        &[("192.168.1.10", "bitcoin.local")],
        &[("umbrel.local", "192.168.1.20")],
    )]);

    let selector = crate::node_connect::tests::selector(Network::Bitcoin);
    for found in nodes {
        let node = found.node;
        let reparsed =
            selector.parse_custom_node(node.url.clone(), node.api_type, node.name.clone()).unwrap();

        assert_eq!(reparsed, node);
    }
}

#[tokio::test]
async fn no_local_interface_means_no_local_network() {
    let cellular = LanInterface {
        ipv4_address: "100.72.1.2".to_string(),
        prefix_length: 24,
        binding: LanBinding::InterfaceIndex { index: 1 },
    };

    assert_eq!(
        discover(Network::Bitcoin, &[cellular]).await,
        Err(NodeDiscoveryError::NoLocalNetwork)
    );
}

// when the platform won't let sockets onto the network, as with a VPN that doesn't allow local
// network access, discovery reports it rather than searching through the default route
#[tokio::test]
async fn a_network_sockets_cannot_be_bound_to_is_reported_as_blocked() {
    let behind_vpn = LanInterface {
        ipv4_address: "192.168.1.18".to_string(),
        prefix_length: 24,
        binding: LanBinding::InterfaceIndex { index: u32::MAX },
    };

    assert_eq!(
        discover(Network::Bitcoin, &[behind_vpn]).await,
        Err(NodeDiscoveryError::LocalNetworkBlocked)
    );
}

// two addresses on one network share a block that is swept once, while the same block on
// another network, such as Ethernet beside Wi-Fi, is still swept over that network
#[test]
fn a_block_is_swept_once_per_network() {
    let on = |address: &str, prefix_length: u8, index: u32| LanInterface {
        ipv4_address: address.to_string(),
        prefix_length,
        binding: LanBinding::InterfaceIndex { index },
    };

    let subnets = lan_subnets(&[
        on("192.168.1.18", 24, 1),
        on("192.168.1.19", 24, 1),
        on("192.168.1.20", 16, 1),
        on("192.168.1.30", 24, 2),
    ]);

    // the /16 address narrows to the same /24 but also sweeps that block's edges, so it stays
    assert_eq!(subnets.len(), 3);
}

/// Runs discovery on a real local network, then picks what it found the way the node screen
/// does
///
/// `COVE_LIVE_LAN_INTERFACE` names this machine's interface by address, prefix and name, such
/// as `192.168.1.18/24@eth0`, and `COVE_LIVE_LAN_NODE` the node url discovery must find, such as
/// `tcp://192.168.1.10:50001`
#[tokio::test]
#[ignore = "needs a local network with an Electrum server, see the doc comment"]
async fn live_lan_discovery_finds_and_connects_to_the_node() {
    crate::test_support::ensure_tokio_runtime();

    let interface = std::env::var("COVE_LIVE_LAN_INTERFACE").expect("COVE_LIVE_LAN_INTERFACE");
    let expected_url = std::env::var("COVE_LIVE_LAN_NODE").expect("COVE_LIVE_LAN_NODE");
    let (address, rest) = interface.split_once('/').expect("address/prefix@name");
    let (prefix, name) = rest.split_once('@').expect("address/prefix@name");

    let name = std::ffi::CString::new(name).expect("interface name");
    // SAFETY: the name is a valid NUL-terminated string for the length of the call
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(index, 0, "no interface named {name:?}");

    let interface = LanInterface {
        ipv4_address: address.to_string(),
        prefix_length: prefix.parse().expect("prefix length"),
        binding: LanBinding::InterfaceIndex { index },
    };

    let started = std::time::Instant::now();
    let nodes = discover(Network::Bitcoin, &[interface]).await.expect("a local network");
    println!("discovery took {:?} and found {nodes:#?}", started.elapsed());

    let found = nodes
        .iter()
        .find(|found| found.node.url == expected_url)
        .unwrap_or_else(|| panic!("{expected_url} not found in {nodes:#?}"));

    found.node.check_url().await.expect("the discovered node answers once picked");
}
