use std::{net::Ipv6Addr, time::Duration};

/// HTTP traffic categories served by the shared clients
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpTrafficCategory {
    /// Fiat prices and fee estimates
    General,
    /// Payjoin directory and OHTTP relay traffic
    Payjoin,
    /// Encrypted diagnostics report uploads
    Diagnostics,
}

/// Traffic categories that the built-in Tor runtime isolates onto separate circuits
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TorTrafficCategory {
    /// Blockchain node traffic
    Node,
    /// One of the HTTP traffic categories
    Http(HttpTrafficCategory),
}

impl TorTrafficCategory {
    /// Password paired with every isolation username
    ///
    /// Both vendored reqwest versions drop URL userinfo when the password is
    /// empty, so the label only survives to the proxy with a non-empty constant
    pub const SOCKS_PASSWORD: &'static str = "cove";

    /// SOCKS username presented to the built-in proxy as an isolation label
    pub const fn socks_username(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Http(HttpTrafficCategory::General) => "http",
            Self::Http(HttpTrafficCategory::Payjoin) => "payjoin",
            Self::Http(HttpTrafficCategory::Diagnostics) => "diagnostics",
        }
    }
}

/// Typed SOCKS5 proxy endpoint for HTTP and node clients
///
/// Isolation labels are only ever attached to Cove's own built-in Tor endpoint;
/// a user-supplied proxy is constructed without them and never receives credentials
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Socks5Proxy {
    host: String,
    port: u16,
    isolation: Option<TorTrafficCategory>,
}

/// Reason a user-supplied SOCKS5 endpoint cannot be used
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidSocks5Proxy {
    /// The host is not a valid domain name or IP literal
    #[error("the proxy host is not a valid host name or IP address")]
    Host,

    /// The port is outside the usable range
    #[error("the proxy port must be between 1 and 65535")]
    Port,
}

impl Socks5Proxy {
    /// Creates an unlabelled SOCKS5 proxy endpoint from a known-good host and port
    ///
    /// Use [`Socks5Proxy::validated`] for user-supplied endpoints
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port, isolation: None }
    }

    /// Creates a labelled endpoint for Cove's own built-in Tor proxy
    ///
    /// The label is presented as SOCKS credentials, which the built-in proxy reads
    /// as a circuit isolation key; never use this for a proxy Cove does not run
    pub fn isolated(host: impl Into<String>, port: u16, isolation: TorTrafficCategory) -> Self {
        Self { host: host.into(), port, isolation: Some(isolation) }
    }

    /// Parses and validates a user-supplied host and port
    ///
    /// The host is checked against the URL host grammar, which rejects credentials,
    /// paths, embedded ports, and whitespace that would otherwise be interpreted
    /// differently by the HTTP client than by a socket-address resolver
    pub fn validated(host: &str, port: u16) -> Result<Self, InvalidSocks5Proxy> {
        if port == 0 {
            return Err(InvalidSocks5Proxy::Port);
        }

        let host = host.trim();
        if host.is_empty() {
            return Err(InvalidSocks5Proxy::Host);
        }

        // a bare IPv6 literal is a valid entry but only parses in its bracketed form
        let parsed = url::Host::parse(host)
            .or_else(|_| url::Host::parse(&format!("[{host}]")))
            .map_err(|_| InvalidSocks5Proxy::Host)?;

        let host = match parsed {
            url::Host::Domain(domain) if domain.is_empty() => {
                return Err(InvalidSocks5Proxy::Host);
            }
            url::Host::Domain(domain) => domain,
            url::Host::Ipv4(address) => address.to_string(),
            url::Host::Ipv6(address) => address.to_string(),
        };

        Ok(Self { host, port, isolation: None })
    }

    /// Returns the proxy host
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the proxy port
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the `host:port` authority, bracketing IPv6 literals
    pub fn authority(&self) -> String {
        format!("{}:{}", self.url_host(), self.port)
    }

    /// Returns the SOCKS credentials carrying this endpoint's isolation label, if any
    pub fn credentials(&self) -> Option<(&'static str, &'static str)> {
        self.isolation
            .map(|isolation| (isolation.socks_username(), TorTrafficCategory::SOCKS_PASSWORD))
    }

    /// Returns the remote-DNS proxy URL, with credentials when the endpoint is labelled
    pub fn url(&self) -> String {
        match self.credentials() {
            Some((username, password)) => {
                format!("socks5h://{username}:{password}@{}", self.authority())
            }
            None => format!("socks5h://{}", self.authority()),
        }
    }

    fn url_host(&self) -> String {
        if self.host.parse::<Ipv6Addr>().is_ok() {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }
}

/// Build a reqwest Client that uses webpki-roots for TLS cert verification,
/// bypassing rustls-platform-verifier (which requires Android JNI init)
pub fn new_client() -> Result<reqwest::Client, reqwest::Error> {
    build_client(None, Redirects::Follow)
}

/// Builds a reqwest client with an optional remote-DNS SOCKS5 proxy
pub fn new_client_with_socks_proxy(
    proxy: Option<&Socks5Proxy>,
) -> Result<reqwest::Client, reqwest::Error> {
    build_client(proxy, Redirects::Follow)
}

/// Build a reqwest Client with the shared TLS configuration that does not follow redirects
pub fn new_client_without_redirects() -> Result<reqwest::Client, reqwest::Error> {
    new_client_without_redirects_with_socks_proxy(None)
}

/// Builds a no-redirect reqwest client with an optional remote-DNS SOCKS5 proxy
pub fn new_client_without_redirects_with_socks_proxy(
    proxy: Option<&Socks5Proxy>,
) -> Result<reqwest::Client, reqwest::Error> {
    build_client(proxy, Redirects::Blocked)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redirects {
    Follow,
    Blocked,
}

fn build_client(
    proxy: Option<&Socks5Proxy>,
    redirects: Redirects,
) -> Result<reqwest::Client, reqwest::Error> {
    let mut builder = client_builder();
    if redirects == Redirects::Blocked {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy.url())?);
    }

    builder.build()
}

fn client_builder() -> reqwest::ClientBuilder {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config =
        rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();

    reqwest::ClientBuilder::new()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .tls_backend_preconfigured(tls_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socks_proxy_uses_remote_dns() {
        let proxy = Socks5Proxy::new("proxy.example", 9050);

        assert_eq!(proxy.url(), "socks5h://proxy.example:9050");
    }

    #[test]
    fn socks_proxy_brackets_ipv6_hosts() {
        let proxy = Socks5Proxy::new("::1", 9050);

        assert_eq!(proxy.url(), "socks5h://[::1]:9050");
        assert_eq!(proxy.authority(), "[::1]:9050");
    }

    #[test]
    fn socks_proxy_formats_authorities() {
        for (host, expected) in [
            ("127.0.0.1", "127.0.0.1:9050"),
            ("proxy.example", "proxy.example:9050"),
            ("::1", "[::1]:9050"),
        ] {
            assert_eq!(Socks5Proxy::new(host, 9050).authority(), expected, "host: {host}");
        }
    }

    #[test]
    fn isolated_proxies_carry_their_category_label() {
        for (category, username) in [
            (TorTrafficCategory::Node, "node"),
            (TorTrafficCategory::Http(HttpTrafficCategory::General), "http"),
            (TorTrafficCategory::Http(HttpTrafficCategory::Payjoin), "payjoin"),
            (TorTrafficCategory::Http(HttpTrafficCategory::Diagnostics), "diagnostics"),
        ] {
            let proxy = Socks5Proxy::isolated("127.0.0.1", 9050, category);

            assert_eq!(proxy.credentials(), Some((username, "cove")));
            assert_eq!(proxy.url(), format!("socks5h://{username}:cove@127.0.0.1:9050"));
        }
    }

    #[test]
    fn isolation_labels_never_reach_a_user_supplied_proxy() {
        for proxy in [
            Socks5Proxy::new("proxy.example", 9050),
            Socks5Proxy::validated("proxy.example", 9050).unwrap(),
        ] {
            assert_eq!(proxy.credentials(), None);
            assert_eq!(proxy.url(), "socks5h://proxy.example:9050");
        }
    }

    #[test]
    fn every_isolation_label_ships_a_non_empty_password() {
        // reqwest drops URL userinfo entirely when the password is empty
        assert!(!TorTrafficCategory::SOCKS_PASSWORD.is_empty());
    }

    #[test]
    fn validated_accepts_host_names_and_ip_literals() {
        for (host, expected) in [
            ("proxy.example", "proxy.example"),
            ("PROXY.Example", "proxy.example"),
            (" 127.0.0.1 ", "127.0.0.1"),
            ("::1", "::1"),
            ("[::1]", "::1"),
        ] {
            let proxy = Socks5Proxy::validated(host, 9050).unwrap();

            assert_eq!(proxy, Socks5Proxy::new(expected, 9050), "host: {host}");
        }
    }

    #[test]
    fn validated_rejects_hosts_that_reqwest_and_socket_resolvers_read_differently() {
        for host in [
            "user@proxy.example",
            "proxy.example/path",
            "proxy.example:9050",
            "proxy example",
            "proxy\t.example",
            "",
            "   ",
        ] {
            assert_eq!(
                Socks5Proxy::validated(host, 9050),
                Err(InvalidSocks5Proxy::Host),
                "host: {host}",
            );
        }
    }

    #[test]
    fn validated_rejects_port_zero() {
        assert_eq!(Socks5Proxy::validated("proxy.example", 0), Err(InvalidSocks5Proxy::Port));
    }
}
