//! The ports and hostnames discovery looks for
//!
//! Only plain TCP Electrum ports are listed. The app trusts only certificates from public
//! authorities, and servers on a home network use self-signed certificates or their own
//! authority, so a TLS server found here would fail the check when picked. Until the app can
//! trust a server's own certificate, TLS-only servers such as StartOS are added by hand
//!
//! Esplora servers are left out too: none of the node distributions serves a full Esplora API
//! on a fixed port (Umbrel's Mempool app lacks the paging routes BDK needs for busy addresses),
//! and an Esplora server on any other port can't be told from another web server without a
//! request

use crate::network::Network;

/// A plain TCP Electrum port, and what is known about the software behind it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServicePort {
    pub port: u16,
    distribution: Option<&'static str>,
    server: Option<&'static str>,
}

impl ServicePort {
    const fn generic(port: u16) -> Self {
        Self { port, distribution: None, server: None }
    }

    const fn known(distribution: &'static str, server: &'static str, port: u16) -> Self {
        Self { port, distribution: Some(distribution), server: Some(server) }
    }

    /// A name for the software behind the port, such as `Umbrel Fulcrum`
    pub fn software(&self) -> Option<String> {
        match (self.distribution, self.server) {
            (Some(distribution), Some(server)) => Some(format!("{distribution} {server}")),
            (Some(name), None) | (None, Some(name)) => Some(name.to_string()),
            (None, None) => None,
        }
    }

    pub const fn is_known_distribution(&self) -> bool {
        self.distribution.is_some()
    }
}

/// A node distribution's fixed `.local` hostname and the servers its apps run
#[derive(Debug, Clone, Copy)]
struct KnownHost {
    hostname: &'static str,
    services: &'static [ServicePort],
}

/// The plain TCP Electrum port servers use by default on each network
///
/// From Blockstream electrs' `default_electrum_port`, which Fulcrum and romanz electrs follow.
/// The TLS port beside it (50002 on mainnet) is left out, and so is Umbrel's Fulcrum, which
/// serves plain TCP on 50002 but is only told apart from a TLS server by its hostname
const MAINNET_PORTS: &[ServicePort] = &[ServicePort::generic(50001)];
const TESTNET_PORTS: &[ServicePort] = &[ServicePort::generic(60001)];
const TESTNET4_PORTS: &[ServicePort] = &[ServicePort::generic(40001)];
const SIGNET_PORTS: &[ServicePort] = &[ServicePort::generic(60601)];

/// Node distributions with a fixed default hostname, and the plain TCP mainnet Electrum servers
/// their apps expose on the local network
///
/// StartOS is missing because each server picks its own hostname and serves Electrum only over
/// TLS with its own certificate authority
const KNOWN_HOSTS: &[KnownHost] = &[
    // umbrel-apps electrs/exports.sh and fulcrum/exports.sh: both plain TCP, Fulcrum on 50002
    KnownHost {
        hostname: "umbrel.local",
        services: &[
            ServicePort::known("Umbrel", "Electrs", 50001),
            ServicePort::known("Umbrel", "Fulcrum", 50002),
        ],
    },
    // mynode electrs.toml; its TLS port 50002 is nginx with a self-signed certificate
    KnownHost {
        hostname: "mynode.local",
        services: &[ServicePort::known("myNode", "Electrs", 50001)],
    },
    // raspiblitz bonus.electrs.sh and bonus.fulcrum.sh, which offsets Fulcrum so both can run;
    // their TLS ports 50002 and 50022 use self-signed certificates
    KnownHost {
        hostname: "raspiblitz.local",
        services: &[
            ServicePort::known("RaspiBlitz", "Electrs", 50001),
            ServicePort::known("RaspiBlitz", "Fulcrum", 50021),
        ],
    },
];

/// What discovery looks for on one network
#[derive(Debug, Clone)]
pub struct Catalog {
    generic: &'static [ServicePort],
    known_hosts: &'static [KnownHost],
}

impl Catalog {
    pub const fn for_network(network: Network) -> Self {
        // the distributions run mainnet unless reconfigured, so their app ports only apply there
        match network {
            Network::Bitcoin => Self { generic: MAINNET_PORTS, known_hosts: KNOWN_HOSTS },
            Network::Testnet => Self { generic: TESTNET_PORTS, known_hosts: &[] },
            Network::Testnet4 => Self { generic: TESTNET4_PORTS, known_hosts: &[] },
            Network::Signet => Self { generic: SIGNET_PORTS, known_hosts: &[] },
        }
    }

    /// The hostnames to look up over mDNS
    pub fn known_hostnames(&self) -> Vec<String> {
        self.known_hosts.iter().map(|host| host.hostname.to_string()).collect()
    }

    /// The ports swept on every host in the subnet
    pub fn generic_port_numbers(&self) -> Vec<u16> {
        self.generic.iter().map(|service| service.port).collect()
    }

    /// The ports probed on a host found by a well-known hostname
    pub fn port_numbers_for(&self, hostname: &str) -> Vec<u16> {
        let known = self.known_host(hostname).map_or(&[][..], |host| host.services);
        let mut ports: Vec<u16> =
            known.iter().chain(self.generic).map(|service| service.port).collect();

        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// What a port on a host serves, preferring what its distribution is known to run there
    pub fn service_for(&self, hostname: Option<&str>, port: u16) -> Option<&ServicePort> {
        let known = hostname.and_then(|hostname| self.known_host(hostname));
        let known_service =
            known.and_then(|host| host.services.iter().find(|service| service.port == port));

        known_service.or_else(|| self.generic.iter().find(|service| service.port == port))
    }

    fn known_host(&self, hostname: &str) -> Option<&KnownHost> {
        self.known_hosts.iter().find(|host| host.hostname.eq_ignore_ascii_case(hostname))
    }
}
