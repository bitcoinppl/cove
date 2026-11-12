//! Finds Electrum servers on the local network
//!
//! Discovery runs only when the person asks for it, and only on the Wi-Fi or Ethernet networks
//! the app reports. It looks up well-known node hostnames over mDNS and probes the device's own
//! subnet for the network's Electrum ports with bare TCP connections, so no server receives a
//! request until the person picks it and the node screen checks it. Every address it probes or
//! returns is a private or link-local one, and every socket is bound to the network being
//! searched, so a VPN can't carry the search off the local network

mod catalog;
mod lan_binding;
mod lan_subnet;
mod mdns;
mod probe;

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use futures::future::join_all;
use tracing::{debug, info, warn};

use crate::{network::Network, node::Node};

use catalog::{Catalog, ServicePort};
use lan_subnet::LanSubnet;
use mdns::{MdnsAnswers, MdnsClient};

pub use lan_subnet::LanInterface;

/// A server found on the local network, which the person can pick as their node
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct DiscoveredNode {
    /// The node to check and save when the person picks it, addressed by IP so it connects
    /// without the platform resolving `.local` names
    pub node: Node,
    /// The server's `.local` hostname, or its IP address when it has none
    pub host: String,
    /// The server's IP address and port, such as `192.168.1.10:50001`
    pub address: String,
    /// The node software known to run there, such as `Umbrel Fulcrum`
    pub software: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Error, thiserror::Error)]
pub enum NodeDiscoveryError {
    /// None of the interfaces the app reported is on a local network, such as when the phone is
    /// only on cellular data
    #[error("not connected to a local network")]
    NoLocalNetwork,

    /// The local network can't be searched directly, such as when a VPN that doesn't allow local
    /// network access covers the app; searching anyway would send the probes through the VPN
    #[error("the local network can't be reached directly, a VPN may be blocking it")]
    LocalNetworkBlocked,
}

/// Finds the servers for `network` reachable from the given Wi-Fi or Ethernet interfaces
pub async fn discover(
    network: Network,
    interfaces: &[LanInterface],
) -> Result<Vec<DiscoveredNode>, NodeDiscoveryError> {
    let subnets = lan_subnets(interfaces);
    if subnets.is_empty() {
        return Err(NodeDiscoveryError::NoLocalNetwork);
    }

    let subnets: Vec<LanSubnet> = subnets.into_iter().filter(can_bind_to).collect();
    if subnets.is_empty() {
        return Err(NodeDiscoveryError::LocalNetworkBlocked);
    }

    let catalog = Catalog::for_network(network);
    let found = join_all(subnets.iter().map(|subnet| discover_on(subnet, &catalog))).await;
    let nodes = merge(network, &catalog, found);

    info!("Found {} node servers on the local network", nodes.len());
    Ok(nodes)
}

/// What one subnet's lookups and probes found
#[derive(Debug, Default)]
struct SubnetFindings {
    open: BTreeSet<SocketAddr>,
    mdns: MdnsAnswers,
}

/// Whether sockets can be bound to the subnet's network, checked once before searching it so a
/// subnet the platform won't let the app reach directly is reported instead of silently empty
fn can_bind_to(subnet: &LanSubnet) -> bool {
    subnet
        .binding()
        .tcp_socket()
        .inspect_err(|error| warn!("Can't search {subnet:?} directly: {error}"))
        .is_ok()
}

async fn discover_on(subnet: &LanSubnet, catalog: &Catalog) -> SubnetFindings {
    let mdns = MdnsClient::on_subnet(subnet);
    let hostnames = catalog.known_hostnames();

    // the subnet sweep runs while mDNS waits for answers about the well-known hostnames
    let sweep_hosts: Vec<IpAddr> = subnet.hosts().map(IpAddr::V4).collect();
    let sweep = probe::open_ports(
        subnet.binding(),
        probe::targets(&sweep_hosts, &catalog.generic_port_numbers()),
        probe::CONNECT_TIMEOUT,
    );

    let (known, swept) = tokio::join!(mdns.lookup(&hostnames, &[]), sweep);
    debug!("mDNS resolved {} well-known node hostnames", known.addresses.len());

    // known hosts may sit outside the swept /24, and their own ports are probed either way
    let known_targets: Vec<SocketAddr> = known
        .addresses
        .iter()
        .flat_map(|(hostname, addresses)| {
            let hosts: Vec<IpAddr> = addresses.iter().copied().map(IpAddr::V4).collect();
            probe::targets(&hosts, &catalog.port_numbers_for(hostname))
        })
        .filter(|target| !swept.contains(target))
        .collect();

    let known_open =
        probe::open_ports(subnet.binding(), known_targets, probe::CONNECT_TIMEOUT).await;
    let open: BTreeSet<SocketAddr> = swept.into_iter().chain(known_open).collect();

    // servers no well-known hostname covers are named by a reverse lookup of their address
    let known_addresses: BTreeSet<Ipv4Addr> = known.addresses.values().flatten().copied().collect();
    let unnamed: Vec<Ipv4Addr> = open
        .iter()
        .filter_map(|address| match address.ip() {
            IpAddr::V4(ip) if !known_addresses.contains(&ip) => Some(ip),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let reverse = mdns.lookup(&[], &unnamed).await;
    let mdns = MdnsAnswers { addresses: known.addresses, names: reverse.names };

    SubnetFindings { open, mdns }
}

fn lan_subnets(interfaces: &[LanInterface]) -> Vec<LanSubnet> {
    let subnets: BTreeSet<LanSubnet> = interfaces
        .iter()
        .filter_map(|interface| {
            LanSubnet::try_from(interface)
                .inspect_err(|error| warn!("Skipping interface for node discovery: {error}"))
                .ok()
        })
        .collect();

    subnets.into_iter().collect()
}

/// Turns what every subnet found into one list of nodes, well-known hosts first
fn merge(network: Network, catalog: &Catalog, found: Vec<SubnetFindings>) -> Vec<DiscoveredNode> {
    let mut hostname_of: BTreeMap<Ipv4Addr, String> = BTreeMap::new();
    let mut open = BTreeSet::new();

    for findings in found {
        // a well-known hostname wins over a reverse lookup's name for the same address
        for (address, name) in findings.mdns.names {
            hostname_of.entry(address).or_insert(name);
        }

        for (hostname, addresses) in findings.mdns.addresses {
            for address in addresses {
                hostname_of.insert(address, hostname.clone());
            }
        }

        open.extend(findings.open);
    }

    let mut nodes: Vec<(bool, DiscoveredNode)> = open
        .into_iter()
        .filter_map(|address| {
            let IpAddr::V4(ip) = address.ip() else { return None };
            let hostname = hostname_of.get(&ip).map(String::as_str);
            let service = catalog.service_for(hostname, address.port())?;

            Some((
                service.is_known_distribution(),
                discovered_node(network, address, hostname, service),
            ))
        })
        .collect();

    nodes.sort_by(|(a_known, a), (b_known, b)| {
        b_known
            .cmp(a_known)
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.address.cmp(&b.address))
    });

    nodes.into_iter().map(|(_, node)| node).collect()
}

fn discovered_node(
    network: Network,
    address: SocketAddr,
    hostname: Option<&str>,
    service: &ServicePort,
) -> DiscoveredNode {
    let host = hostname.map_or_else(|| address.ip().to_string(), str::to_string);
    let software = service.software();

    let name = match &software {
        Some(software) => format!("{software} ({host})"),
        None => host.clone(),
    };

    // only plain TCP ports are probed, see the catalog for why TLS servers are left out
    let node = Node::new_electrum(name, format!("tcp://{address}"), network);

    DiscoveredNode { node, host, address: address.to_string(), software }
}

#[cfg(test)]
mod tests;
