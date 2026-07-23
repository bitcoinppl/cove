use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use cove_util::ResultExt as _;
use if_addrs::{IfAddr, Interface};

use super::NodeScannerError;

const MAX_SUBNETS: usize = 2;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct InterfaceCandidate {
    name: String,
    ip: Ipv4Addr,
    netmask: Ipv4Addr,
    oper_up: bool,
    point_to_point: bool,
}

impl TryFrom<Interface> for InterfaceCandidate {
    type Error = ();

    fn try_from(interface: Interface) -> Result<Self, Self::Error> {
        let oper_up = interface.is_oper_up();
        let point_to_point = interface.is_p2p();
        let IfAddr::V4(address) = interface.addr else { return Err(()) };

        Ok(Self {
            name: interface.name,
            ip: address.ip,
            netmask: address.netmask,
            oper_up,
            point_to_point,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Subnet {
    network: Ipv4Addr,
    prefix: u8,
    device_ip: Ipv4Addr,
}

impl Subnet {
    fn from_candidate(candidate: &InterfaceCandidate) -> Option<Self> {
        if !eligible(candidate) {
            return None;
        }

        let prefix = contiguous_prefix(candidate.netmask)?;
        if prefix > 30 {
            return None;
        }

        let prefix = prefix.max(24);
        let mask = prefix_mask(prefix);
        let network = Ipv4Addr::from(u32::from(candidate.ip) & mask);

        Some(Self { network, prefix, device_ip: candidate.ip })
    }

    fn contains(self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & prefix_mask(self.prefix) == u32::from(self.network)
    }

    fn hosts(self) -> impl Iterator<Item = Ipv4Addr> {
        let network = u32::from(self.network);
        let host_count = 1u32 << (32 - self.prefix);
        let broadcast = network + host_count - 1;

        (network + 1..broadcast).map(Ipv4Addr::from).filter(move |ip| *ip != self.device_ip)
    }
}

pub(crate) fn hosts_to_scan() -> Result<Vec<Ipv4Addr>, NodeScannerError> {
    let interfaces = if_addrs::get_if_addrs().map_err_str(NodeScannerError::ScanFailed)?;
    let candidates = interfaces
        .into_iter()
        .filter_map(|interface| InterfaceCandidate::try_from(interface).ok())
        .collect::<Vec<_>>();
    let default_ip = default_route_ip();

    select_hosts(&candidates, default_ip).ok_or(NodeScannerError::NoLocalNetwork)
}

fn select_hosts(
    candidates: &[InterfaceCandidate],
    default_ip: Option<Ipv4Addr>,
) -> Option<Vec<Ipv4Addr>> {
    let self_ips = candidates
        .iter()
        .filter(|candidate| eligible(candidate))
        .map(|candidate| candidate.ip)
        .chain(default_ip)
        .collect::<BTreeSet<_>>();
    let mut subnets = candidates
        .iter()
        .filter_map(|candidate| {
            let subnet = Subnet::from_candidate(candidate)?;
            let default_route = default_ip.is_some_and(|ip| subnet.contains(ip));

            Some((default_route, candidate.name.clone(), candidate.ip, subnet))
        })
        .collect::<Vec<_>>();

    subnets.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)).then_with(|| left.2.cmp(&right.2))
    });

    let mut unique_subnets = BTreeSet::new();
    let mut selected = Vec::new();
    for (_, _, _, subnet) in subnets {
        if unique_subnets.insert((subnet.network, subnet.prefix)) {
            selected.push(subnet);
        }

        if selected.len() == MAX_SUBNETS {
            break;
        }
    }

    if selected.is_empty()
        && let Some(ip) = default_ip.filter(|ip| is_rfc1918(*ip))
    {
        selected.push(Subnet {
            network: Ipv4Addr::from(u32::from(ip) & prefix_mask(24)),
            prefix: 24,
            device_ip: ip,
        });
    }

    let hosts = selected
        .into_iter()
        .flat_map(Subnet::hosts)
        .filter(|host| !self_ips.contains(host))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    (!hosts.is_empty()).then_some(hosts)
}

fn eligible(candidate: &InterfaceCandidate) -> bool {
    candidate.oper_up
        && !candidate.point_to_point
        && !candidate.ip.is_loopback()
        && !candidate.ip.is_link_local()
        && is_rfc1918(candidate.ip)
        && !denied_interface_name(&candidate.name)
}

fn denied_interface_name(name: &str) -> bool {
    const DENIED_PREFIXES: [&str; 9] =
        ["pdp_ip", "awdl", "llw", "utun", "ipsec", "rmnet", "ccmni", "tun", "ppp"];

    DENIED_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

const fn is_rfc1918(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();

    octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
}

fn contiguous_prefix(netmask: Ipv4Addr) -> Option<u8> {
    let mask = u32::from(netmask);
    let prefix = mask.leading_ones() as u8;

    (mask == prefix_mask(prefix)).then_some(prefix)
}

const fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) }
}

fn default_route_ip() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;

    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if is_rfc1918(ip) => Some(ip),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, ip: [u8; 4], prefix: u8) -> InterfaceCandidate {
        InterfaceCandidate {
            name: name.to_string(),
            ip: Ipv4Addr::from(ip),
            netmask: Ipv4Addr::from(prefix_mask(prefix)),
            oper_up: true,
            point_to_point: false,
        }
    }

    #[test]
    fn interface_status_point_to_point_name_and_address_filters_are_applied() {
        let valid = candidate("en0", [192, 168, 1, 10], 24);
        assert!(eligible(&valid));

        let mut down = valid.clone();
        down.oper_up = false;
        assert!(!eligible(&down));

        let mut point_to_point = valid.clone();
        point_to_point.point_to_point = true;
        assert!(!eligible(&point_to_point));

        for name in
            ["pdp_ip0", "awdl0", "llw0", "utun4", "ipsec0", "rmnet0", "ccmni0", "tun0", "ppp0"]
        {
            assert!(!eligible(&candidate(name, [192, 168, 1, 10], 24)), "{name}");
        }

        for ip in [[127, 0, 0, 1], [169, 254, 1, 2], [100, 64, 0, 1], [8, 8, 8, 8]] {
            assert!(!eligible(&candidate("en0", ip, 24)), "{ip:?}");
        }
    }

    #[test]
    fn masks_must_be_contiguous() {
        assert_eq!(contiguous_prefix(Ipv4Addr::new(255, 255, 255, 0)), Some(24));
        assert_eq!(contiguous_prefix(Ipv4Addr::new(255, 255, 254, 0)), Some(23));
        assert_eq!(contiguous_prefix(Ipv4Addr::new(255, 0, 255, 0)), None);
    }

    #[test]
    fn broader_networks_are_clamped_to_the_device_24() {
        let hosts = select_hosts(&[candidate("en0", [10, 42, 7, 9], 16)], None).unwrap();

        assert_eq!(hosts.len(), 253);
        assert_eq!(hosts.first(), Some(&Ipv4Addr::new(10, 42, 7, 1)));
        assert_eq!(hosts.last(), Some(&Ipv4Addr::new(10, 42, 7, 254)));
        assert!(!hosts.contains(&Ipv4Addr::new(10, 42, 7, 9)));
    }

    #[test]
    fn host_math_excludes_network_broadcast_and_self_for_24_through_30() {
        for prefix in 24..=30 {
            let own = [192, 168, 1, 1];
            let hosts = select_hosts(&[candidate("en0", own, prefix)], None).unwrap();
            let expected = (1usize << (32 - prefix)) - 3;

            assert_eq!(hosts.len(), expected, "/{prefix}");
            assert!(!hosts.contains(&Ipv4Addr::from(own)));
        }
    }

    #[test]
    fn prefixes_narrower_than_30_are_skipped() {
        assert!(select_hosts(&[candidate("en0", [192, 168, 1, 2], 31)], None).is_none());
        assert!(select_hosts(&[candidate("en0", [192, 168, 1, 2], 32)], None).is_none());
    }

    #[test]
    fn subnets_are_deduplicated_and_capped_with_default_route_first() {
        let candidates = [
            candidate("en1", [192, 168, 2, 10], 24),
            candidate("en0", [192, 168, 1, 10], 24),
            candidate("bridge0", [192, 168, 1, 20], 24),
            candidate("eth0", [10, 0, 0, 10], 24),
        ];
        let hosts = select_hosts(&candidates, Some(Ipv4Addr::new(192, 168, 2, 44))).unwrap();

        assert_eq!(hosts.len(), 504);
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 2, 44)));
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 100)));
        assert!(!hosts.contains(&Ipv4Addr::new(10, 0, 0, 100)));
    }

    #[test]
    fn private_default_route_falls_back_to_a_24() {
        let hosts = select_hosts(&[], Some(Ipv4Addr::new(172, 20, 8, 5))).unwrap();

        assert_eq!(hosts.len(), 253);
        assert!(!hosts.contains(&Ipv4Addr::new(172, 20, 8, 5)));
    }
}
