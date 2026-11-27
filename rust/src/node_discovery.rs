//! Finds Electrum servers on the local network
//!
//! Discovery runs only when the person asks for it, and only on the Wi-Fi or Ethernet networks
//! the app reports. It looks up well-known node hostnames over mDNS and probes the device's own
//! subnet for the network's Electrum ports with bare TCP connections, then asks each open port
//! which chain it serves, so only Electrum servers for the selected network are listed. Every
//! address it probes or returns is a private or link-local one, and every socket is bound to the
//! network being searched, so a VPN can't carry the search off the local network

mod catalog;
mod lan_binding;
mod lan_subnet;
mod mdns;
mod probe;
#[cfg(debug_assertions)]
pub(crate) mod ui_test_search;
pub(crate) mod verify;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use futures::{Stream, StreamExt as _, future::join_all, stream};
use tokio::net::TcpSocket;
use tracing::{debug, info, warn};

use crate::{network::Network, node::Node};

use catalog::{Catalog, ServicePort};
use lan_binding::LanBinding;
use lan_subnet::LanSubnet;
use mdns::{MdnsAnswers, MdnsClient};
use probe::Prober;
use verify::ServerIdentity;

pub use lan_subnet::LanInterface;

/// The longest a whole search runs, after which it ends with the servers found so far
pub const SEARCH_TIME_LIMIT: Duration = Duration::from_secs(30);

/// The most subnets one search covers, the first ones the platform reports, so a phone on
/// several networks at once doesn't multiply the sweep
pub const MAX_SEARCHED_SUBNETS: usize = 2;

/// How many open ports are checked at once, few since a search rarely finds more than a handful
const MAX_CONCURRENT_CHECKS: usize = 4;

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
    /// The node software known or reported to run there, such as `Umbrel Fulcrum`
    pub software: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
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

/// The network a search's sockets go over
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Route {
    /// A Wi-Fi or Ethernet network the platform reported
    Lan(LanBinding),
    /// This device's loopback, where UI tests run their servers
    #[cfg(debug_assertions)]
    UiTestLoopback,
}

impl Route {
    fn tcp_socket(self) -> io::Result<TcpSocket> {
        match self {
            Self::Lan(binding) => binding.tcp_socket(),
            #[cfg(debug_assertions)]
            Self::UiTestLoopback => TcpSocket::new_v4(),
        }
    }
}

/// An open port that may be a server, waiting for its check
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    found: DiscoveredNode,
    address: SocketAddr,
    route: Route,
}

impl Candidate {
    /// The listed server once its check passed, named by its own software when nothing better
    /// is known about it
    fn verified(self, identity: ServerIdentity) -> DiscoveredNode {
        let mut found = self.found;
        if found.software.is_some() {
            return found;
        }

        let Some(software) = identity.software else { return found };
        found.node.name = format!("{software} ({})", found.host);
        found.software = Some(software);
        found
    }
}

/// A server that passed its check, with its place among the search's servers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The server's place in the search's order, well-known hosts first, which stays the same
    /// whatever order the checks finish in
    pub rank: usize,
    pub found: DiscoveredNode,
}

/// The open ports one search found, before each is checked
#[derive(Debug)]
pub struct Search {
    network: Network,
    candidates: Vec<Candidate>,
}

impl Search {
    /// Sweeps the given Wi-Fi or Ethernet interfaces for the network's Electrum ports
    pub async fn sweep(
        network: Network,
        interfaces: &[LanInterface],
    ) -> Result<Self, NodeDiscoveryError> {
        let subnets = lan_subnets(interfaces);
        if subnets.is_empty() {
            return Err(NodeDiscoveryError::NoLocalNetwork);
        }

        let subnets: Vec<LanSubnet> = subnets.into_iter().filter(can_bind_to).collect();
        if subnets.is_empty() {
            return Err(NodeDiscoveryError::LocalNetworkBlocked);
        }

        // every subnet is searched at once, drawing on one allowance of open sockets
        let catalog = Catalog::for_network(network);
        let prober = Prober::default();
        let found =
            join_all(subnets.iter().map(|subnet| discover_on(subnet, &catalog, &prober))).await;
        let candidates = merge(network, &catalog, found);

        info!("Found {} open Electrum ports on the local network", candidates.len());
        Ok(Self { network, candidates })
    }

    /// Checks every open port, yielding each server for the selected network as its check
    /// passes, in the order the checks finish
    pub fn verified(self) -> impl Stream<Item = Verified> {
        let network = self.network;

        stream::iter(self.candidates.into_iter().enumerate())
            .map(move |(rank, candidate)| async move {
                let socket = candidate
                    .route
                    .tcp_socket()
                    .inspect_err(|error| debug!("Can't check {}: {error}", candidate.address))
                    .ok()?;

                match verify::verify(socket, candidate.address, network).await {
                    Ok(identity) => Some(Verified { rank, found: candidate.verified(identity) }),
                    Err(error) => {
                        info!("Not listing {}: {error}", candidate.address);
                        None
                    }
                }
            })
            .buffer_unordered(MAX_CONCURRENT_CHECKS)
            .filter_map(std::future::ready)
    }
}

/// What one subnet's lookups and probes found
#[derive(Debug)]
struct SubnetFindings {
    binding: LanBinding,
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

async fn discover_on(subnet: &LanSubnet, catalog: &Catalog, prober: &Prober) -> SubnetFindings {
    let mdns = MdnsClient::on_subnet(subnet);
    let hostnames = catalog.known_hostnames();

    // the subnet sweep runs while mDNS waits for answers about the well-known hostnames
    let sweep_hosts: Vec<IpAddr> = subnet.hosts().map(IpAddr::V4).collect();
    let sweep = prober.open_ports(
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
        prober.open_ports(subnet.binding(), known_targets, probe::CONNECT_TIMEOUT).await;
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

    SubnetFindings { binding: subnet.binding(), open, mdns }
}

/// The subnets to sweep, in the order the platform reported their interfaces
fn lan_subnets(interfaces: &[LanInterface]) -> Vec<LanSubnet> {
    let mut subnets: Vec<LanSubnet> = Vec::new();

    for interface in interfaces {
        let subnet = match LanSubnet::try_from(interface) {
            Ok(subnet) => subnet,
            Err(error) => {
                warn!("Skipping interface for node discovery: {error}");
                continue;
            }
        };

        // hosts reached through two addresses on one network are swept once
        if subnets.iter().any(|earlier| subnet.sweeps_same_hosts_as(earlier)) {
            continue;
        }

        if subnets.len() == MAX_SEARCHED_SUBNETS {
            info!("Not searching {subnet:?}, a search covers {MAX_SEARCHED_SUBNETS} subnets");
            continue;
        }

        subnets.push(subnet);
    }

    subnets
}

/// Turns what every subnet found into one list of servers to check, well-known hosts first
fn merge(network: Network, catalog: &Catalog, found: Vec<SubnetFindings>) -> Vec<Candidate> {
    let mut hostname_of: BTreeMap<Ipv4Addr, String> = BTreeMap::new();
    let mut open: BTreeMap<SocketAddr, LanBinding> = BTreeMap::new();

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

        // an address open on two networks is checked over the first that found it
        for address in findings.open {
            open.entry(address).or_insert(findings.binding);
        }
    }

    let mut candidates: Vec<(bool, Candidate)> = open
        .into_iter()
        .filter_map(|(address, binding)| {
            let IpAddr::V4(ip) = address.ip() else { return None };
            let hostname = hostname_of.get(&ip).map(String::as_str);
            let service = catalog.service_for(hostname, address.port())?;
            let found = discovered_node(network, address, hostname, service);

            let candidate = Candidate { found, address, route: Route::Lan(binding) };
            Some((service.is_known_distribution(), candidate))
        })
        .collect();

    candidates.sort_by(|(a_known, a), (b_known, b)| {
        b_known
            .cmp(a_known)
            .then_with(|| a.found.host.cmp(&b.found.host))
            .then_with(|| a.found.address.cmp(&b.found.address))
    });

    candidates.into_iter().map(|(_, candidate)| candidate).collect()
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
