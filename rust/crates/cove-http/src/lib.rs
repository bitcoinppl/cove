use std::{net::Ipv6Addr, time::Duration};

/// Typed SOCKS5 proxy endpoint for HTTP clients
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Socks5Proxy {
    host: String,
    port: u16,
}

impl Socks5Proxy {
    /// Creates a SOCKS5 proxy endpoint from a host and port
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port }
    }

    fn url(&self) -> String {
        let host = if self.host.parse::<Ipv6Addr>().is_ok() {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };

        format!("socks5h://{host}:{}", self.port)
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
    }
}
