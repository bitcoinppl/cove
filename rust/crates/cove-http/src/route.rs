//! The route an outbound connection takes: straight to its destination, or through Tor

use std::{
    fmt,
    net::{IpAddr, Ipv6Addr},
    time::Duration,
};

use crate::RouteTimeouts;

const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DIRECT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// a fresh Tor circuit takes a few round trips through three relays, and a circuit to an onion
// service takes more, so Tor gets longer waits than a direct connection
const TOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
const TOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

/// Where new outbound connections go
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkRoute {
    /// Straight to the destination, only while Tor is off
    Direct,

    /// Through a SOCKS5 proxy into Tor, which also resolves every host name
    Tor(SocksProxy),
}

impl NetworkRoute {
    pub const fn is_tor(&self) -> bool {
        matches!(self, Self::Tor(_))
    }

    /// How long clients wait on this route
    pub const fn timeouts(&self) -> RouteTimeouts {
        match self {
            Self::Direct => {
                RouteTimeouts { connect: DIRECT_CONNECT_TIMEOUT, request: DIRECT_REQUEST_TIMEOUT }
            }

            Self::Tor(_) => {
                RouteTimeouts { connect: TOR_CONNECT_TIMEOUT, request: TOR_REQUEST_TIMEOUT }
            }
        }
    }
}

/// A SOCKS5 proxy that leads into Tor
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SocksProxy {
    pub address: ProxyAddress,
    pub credentials: Option<SocksCredentials>,
}

impl SocksProxy {
    /// The proxy URL for reqwest
    ///
    /// `socks5h` makes the proxy resolve host names. Plain `socks5` would look them up with the
    /// system resolver first, which reveals every host to the local DNS server and can't
    /// resolve `.onion` names at all
    pub fn url(&self) -> String {
        let address = &self.address;
        match &self.credentials {
            None => format!("socks5h://{address}"),
            Some(credentials) => {
                let SocksCredentials { username, password } = credentials;
                format!("socks5h://{username}:{password}@{address}")
            }
        }
    }
}

/// A SOCKS5 username and password
///
/// The built-in Tor proxy listens on a local port, which other apps on the device can reach.
/// Credentials that only this process knows keep them from using it
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SocksCredentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for SocksCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocksCredentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// A proxy's host and port
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyAddress {
    host: ProxyHost,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ProxyHost {
    Ip(IpAddr),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ProxyAddressError {
    #[error("enter the proxy as host:port, such as 127.0.0.1:9050")]
    MissingPort,

    #[error("the port must be a number from 1 to 65535")]
    InvalidPort,

    #[error("enter the proxy's host name or IP address")]
    MissingHost,

    #[error("the host name can only contain letters, digits, dots and dashes")]
    InvalidHost,
}

impl ProxyAddress {
    pub const fn new_ip(ip: IpAddr, port: u16) -> Self {
        Self { host: ProxyHost::Ip(ip), port }
    }

    /// Parses `host:port`, with an IPv6 host in brackets, such as `[::1]:9050`
    pub fn parse(input: &str) -> Result<Self, ProxyAddressError> {
        let input = input.trim();
        let input = ["socks5h://", "socks5://"]
            .iter()
            .find_map(|scheme| input.strip_prefix(scheme))
            .unwrap_or(input);

        let (host, port) = split_host_port(input)?;
        let port = port.parse::<u16>().ok().filter(|port| *port != 0);
        let port = port.ok_or(ProxyAddressError::InvalidPort)?;

        Ok(Self { host: parse_host(host)?, port })
    }
}

fn split_host_port(input: &str) -> Result<(&str, &str), ProxyAddressError> {
    if let Some(rest) = input.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').ok_or(ProxyAddressError::InvalidHost)?;
        let port = rest.strip_prefix(':').ok_or(ProxyAddressError::MissingPort)?;
        return Ok((host, port));
    }

    // a bare IPv6 address has several colons and no way to tell its port apart
    if input.parse::<Ipv6Addr>().is_ok() {
        return Err(ProxyAddressError::MissingPort);
    }

    input.rsplit_once(':').ok_or(ProxyAddressError::MissingPort)
}

fn parse_host(host: &str) -> Result<ProxyHost, ProxyAddressError> {
    if host.is_empty() {
        return Err(ProxyAddressError::MissingHost);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ProxyHost::Ip(ip));
    }

    let valid_name = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });

    if !valid_name {
        return Err(ProxyAddressError::InvalidHost);
    }

    Ok(ProxyHost::Name(host.to_ascii_lowercase()))
}

impl fmt::Display for ProxyAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let port = self.port;
        match &self.host {
            ProxyHost::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]:{port}"),
            ProxyHost::Ip(IpAddr::V4(ip)) => write!(f, "{ip}:{port}"),
            ProxyHost::Name(name) => write!(f, "{name}:{port}"),
        }
    }
}

/// Whether `host` names an onion service, which only Tor can reach
///
/// Looking one up with the system resolver would hand the onion address to the local DNS
/// server, so callers refuse these hosts on a direct route instead of trying them
pub fn is_onion_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    host.len() > ".onion".len() && host.to_ascii_lowercase().ends_with(".onion")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_ipv6_and_names() {
        let ipv4 = ProxyAddress::parse("127.0.0.1:9050").unwrap();
        assert_eq!(ipv4.to_string(), "127.0.0.1:9050");

        let ipv6 = ProxyAddress::parse(" [::1]:9150 ").unwrap();
        assert_eq!(ipv6.to_string(), "[::1]:9150");

        let name = ProxyAddress::parse("socks5h://Proxy.Home.Lan:9050").unwrap();
        assert_eq!(name.to_string(), "proxy.home.lan:9050");
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert_eq!(ProxyAddress::parse("127.0.0.1"), Err(ProxyAddressError::MissingPort));
        assert_eq!(ProxyAddress::parse("::1"), Err(ProxyAddressError::MissingPort));
        assert_eq!(ProxyAddress::parse("[::1]"), Err(ProxyAddressError::MissingPort));
        assert_eq!(ProxyAddress::parse("127.0.0.1:0"), Err(ProxyAddressError::InvalidPort));
        assert_eq!(ProxyAddress::parse("127.0.0.1:70000"), Err(ProxyAddressError::InvalidPort));
        assert_eq!(ProxyAddress::parse(":9050"), Err(ProxyAddressError::MissingHost));
        assert_eq!(ProxyAddress::parse("bad host:9050"), Err(ProxyAddressError::InvalidHost));
        assert_eq!(ProxyAddress::parse("user@host:9050"), Err(ProxyAddressError::InvalidHost));
    }

    #[test]
    fn debug_output_hides_the_proxy_password() {
        let address = ProxyAddress::parse("127.0.0.1:9050").unwrap();
        let credentials =
            SocksCredentials { username: "cove".to_string(), password: "secret".to_string() };
        let proxy = SocksProxy { address, credentials: Some(credentials) };

        assert!(!format!("{proxy:?}").contains("secret"));
    }

    #[test]
    fn detects_onion_hosts() {
        assert!(is_onion_host("abcdefghijklmnop.onion"));
        assert!(is_onion_host("ABCDEFGHIJKLMNOP.ONION."));
        assert!(!is_onion_host("onion"));
        assert!(!is_onion_host(".onion"));
        assert!(!is_onion_host("example.com"));
        assert!(!is_onion_host("onion.example.com"));
    }
}
