use std::net::{IpAddr, Ipv4Addr};

use ipnet::{Ipv4AddrRange, Ipv4Net};

use super::lan_binding::LanBinding;

/// The widest prefix discovery sweeps, so a /16 home network still costs at most 254 probes
/// per port
const WIDEST_SWEPT_PREFIX: u8 = 24;

/// A Wi-Fi or Ethernet interface address the app reports, the only network discovery sweeps
///
/// The platform picks the interfaces, since only it knows which one is Wi-Fi or Ethernet rather
/// than cellular or a VPN, which can also carry private addresses
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct LanInterface {
    /// The interface's IPv4 address, such as `192.168.1.18`
    pub ipv4_address: String,
    /// The interface's network prefix length, such as 24 for a `255.255.255.0` netmask
    pub prefix_length: u8,
    /// The network every socket searching this interface's subnet is bound to
    pub binding: LanBinding,
}

/// Why an interface the platform reported can't be swept
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LanSubnetError {
    #[error("{0} is not an IPv4 address")]
    NotIpv4(String),

    #[error("prefix length {0} is longer than 32")]
    InvalidPrefix(u8),

    #[error("{0} is not a local network address")]
    NotLocal(Ipv4Addr),
}

/// The local IPv4 subnet discovery sweeps, at most a /24 around the device's own address
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanSubnet {
    binding: LanBinding,
    /// The block swept, the interface's network narrowed to at most a /24
    swept: Ipv4Net,
    /// The interface's whole network, whose own network and broadcast addresses aren't hosts
    network: Ipv4Net,
    own_address: Ipv4Addr,
}

impl LanSubnet {
    /// The addresses to probe, every host in the swept block except the device itself
    ///
    /// Only the interface network's own edges are skipped: when a wider network is narrowed to
    /// a /24, that block's first and last addresses are ordinary hosts
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> + '_ {
        // a /31 or /32 has no network or broadcast address to skip
        let edges = (self.network.prefix_len() < 31)
            .then(|| [self.network.network(), self.network.broadcast()]);

        Ipv4AddrRange::new(self.swept.network(), self.swept.broadcast()).filter(move |host| {
            *host != self.own_address && !edges.is_some_and(|edges| edges.contains(host))
        })
    }

    /// Whether both subnets would sweep the same hosts over the same network, such as two
    /// addresses the device holds on one Wi-Fi network
    pub fn sweeps_same_hosts_as(&self, other: &Self) -> bool {
        self.binding == other.binding && self.swept == other.swept && self.network == other.network
    }

    /// The device's own address on this subnet, which mDNS queries are sent from
    pub const fn own_address(&self) -> Ipv4Addr {
        self.own_address
    }

    /// The network the subnet's sockets are bound to
    pub const fn binding(&self) -> LanBinding {
        self.binding
    }
}

impl TryFrom<&LanInterface> for LanSubnet {
    type Error = LanSubnetError;

    fn try_from(interface: &LanInterface) -> Result<Self, Self::Error> {
        let own_address: Ipv4Addr = interface
            .ipv4_address
            .parse()
            .map_err(|_| LanSubnetError::NotIpv4(interface.ipv4_address.clone()))?;

        if !is_lan_ipv4(own_address) {
            return Err(LanSubnetError::NotLocal(own_address));
        }

        let network = Ipv4Net::new(own_address, interface.prefix_length)
            .map_err(|_| LanSubnetError::InvalidPrefix(interface.prefix_length))?
            .trunc();

        // a wider network is narrowed to the /24 holding the device, the block most nodes on a
        // home network share with the phone
        let swept_prefix = interface.prefix_length.max(WIDEST_SWEPT_PREFIX);
        let swept = Ipv4Net::new(own_address, swept_prefix)
            .expect("a prefix no shorter than a valid one is valid")
            .trunc();

        Ok(Self { binding: interface.binding, swept, network, own_address })
    }
}

/// Whether an address belongs to a local network: RFC 1918 private or link-local
///
/// Carrier-grade NAT space (100.64.0.0/10) is left out, since it is the carrier's network rather
/// than the user's, and so are loopback and every public address
pub fn is_lan_ipv4(address: Ipv4Addr) -> bool {
    address.is_private() || address.is_link_local()
}

/// Whether an address, of either family, belongs to a local network
///
/// IPv6 counts unique local (fc00::/7) and link-local (fe80::/10) addresses
pub fn is_lan_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_lan_ipv4(address),
        IpAddr::V6(address) => address.is_unique_local() || address.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(address: &str, prefix_length: u8) -> LanInterface {
        LanInterface {
            ipv4_address: address.to_string(),
            prefix_length,
            binding: LanBinding::InterfaceIndex { index: 1 },
        }
    }

    #[test]
    fn a_home_subnet_sweeps_every_other_host() {
        let subnet = LanSubnet::try_from(&interface("192.168.1.18", 24)).unwrap();
        let hosts: Vec<_> = subnet.hosts().collect();

        assert_eq!(hosts.len(), 253);
        assert_eq!(hosts.first(), Some(&Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(hosts.last(), Some(&Ipv4Addr::new(192, 168, 1, 254)));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 18)));
    }

    #[test]
    fn a_wide_subnet_is_narrowed_to_the_devices_slash_24() {
        let subnet = LanSubnet::try_from(&interface("192.168.1.18", 16)).unwrap();
        let hosts: Vec<_> = subnet.hosts().collect();

        // the block's first and last addresses are ordinary hosts on the wider network
        assert_eq!(hosts.len(), 255);
        assert_eq!(hosts.first(), Some(&Ipv4Addr::new(192, 168, 1, 0)));
        assert_eq!(hosts.last(), Some(&Ipv4Addr::new(192, 168, 1, 255)));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 18)));
    }

    #[test]
    fn the_wider_networks_own_edges_are_still_skipped() {
        let first_block = LanSubnet::try_from(&interface("10.0.0.5", 8)).unwrap();
        let last_block = LanSubnet::try_from(&interface("10.255.255.5", 8)).unwrap();

        assert_eq!(first_block.hosts().next(), Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(last_block.hosts().last(), Some(Ipv4Addr::new(10, 255, 255, 254)));
    }

    #[test]
    fn a_narrow_subnet_keeps_its_own_size() {
        let subnet = LanSubnet::try_from(&interface("192.168.1.18", 30)).unwrap();

        assert_eq!(subnet.hosts().collect::<Vec<_>>(), vec![Ipv4Addr::new(192, 168, 1, 17)]);
    }

    #[test]
    fn only_local_interfaces_can_be_swept() {
        for (address, expected) in [
            ("8.8.8.8", LanSubnetError::NotLocal(Ipv4Addr::new(8, 8, 8, 8))),
            ("100.72.1.2", LanSubnetError::NotLocal(Ipv4Addr::new(100, 72, 1, 2))),
            ("127.0.0.1", LanSubnetError::NotLocal(Ipv4Addr::LOCALHOST)),
            ("fe80::1", LanSubnetError::NotIpv4("fe80::1".to_string())),
        ] {
            assert_eq!(LanSubnet::try_from(&interface(address, 24)), Err(expected));
        }

        assert_eq!(
            LanSubnet::try_from(&interface("192.168.1.18", 33)),
            Err(LanSubnetError::InvalidPrefix(33))
        );
    }

    #[test]
    fn lan_addresses_cover_private_and_link_local_ranges_of_both_families() {
        for local in ["10.0.0.1", "172.16.5.4", "192.168.1.10", "169.254.1.1", "fd00::1", "fe80::1"]
        {
            assert!(is_lan_address(local.parse().unwrap()), "{local} is local");
        }

        for remote in ["1.1.1.1", "100.64.0.1", "127.0.0.1", "2606:4700::1111", "::1"] {
            assert!(!is_lan_address(remote.parse().unwrap()), "{remote} is not local");
        }
    }
}
