//! HTTP clients and the network route every outbound connection takes
//!
//! Every reqwest client in the app is built here from a [`NetworkRoute`], so a client can't be
//! created without deciding whether its traffic goes over Tor. `rust/clippy.toml` forbids the
//! raw reqwest constructors everywhere else

pub mod route;

use std::time::Duration;

use parking_lot::Mutex;

pub use route::{NetworkRoute, ProxyAddress, ProxyAddressError, SocksCredentials, SocksProxy};

/// Build a reqwest Client that uses webpki-roots for TLS cert verification,
/// bypassing rustls-platform-verifier (which requires Android JNI init)
pub fn new_client(route: &NetworkRoute) -> Result<reqwest::Client, reqwest::Error> {
    client_builder(route)?.build()
}

/// Build a reqwest Client with the shared TLS configuration that does not follow redirects
pub fn new_client_without_redirects(
    route: &NetworkRoute,
) -> Result<reqwest::Client, reqwest::Error> {
    client_builder(route)?.redirect(reqwest::redirect::Policy::none()).build()
}

#[allow(clippy::disallowed_methods, reason = "the one place that builds raw reqwest clients")]
fn client_builder(route: &NetworkRoute) -> Result<reqwest::ClientBuilder, reqwest::Error> {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config =
        rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();

    let timeouts = route.timeouts();
    let builder = reqwest::ClientBuilder::new()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.request)
        .tls_backend_preconfigured(tls_config);

    let builder = match route {
        NetworkRoute::Direct => builder,

        // `Proxy::all` also replaces any system proxy, so nothing picks a different path
        NetworkRoute::Tor(proxy) => builder.proxy(reqwest::Proxy::all(proxy.url())?),
    };

    Ok(builder)
}

/// How long a client waits on a route
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteTimeouts {
    /// Opening the connection, including the SOCKS handshake and Tor circuit setup
    pub connect: Duration,
    /// The whole request
    pub request: Duration,
}

/// A reqwest client kept for reuse until the route changes
///
/// Long-lived services hold one of these instead of a client, so turning Tor on or off takes
/// effect on their next request instead of reusing a pooled connection on the old route
#[derive(Debug)]
pub struct RoutedClient {
    redirects: bool,
    cached: Mutex<Option<(NetworkRoute, reqwest::Client)>>,
}

impl RoutedClient {
    /// A client that follows redirects
    pub const fn new() -> Self {
        Self { redirects: true, cached: Mutex::new(None) }
    }

    /// The client for `route`, building a new one when the route changed since the last call
    pub fn get(&self, route: &NetworkRoute) -> Result<reqwest::Client, reqwest::Error> {
        let mut cached = self.cached.lock();
        if let Some((cached_route, client)) = cached.as_ref()
            && cached_route == route
        {
            return Ok(client.clone());
        }

        let client =
            if self.redirects { new_client(route)? } else { new_client_without_redirects(route)? };

        *cached = Some((route.clone(), client.clone()));
        Ok(client)
    }
}

impl Default for RoutedClient {
    fn default() -> Self {
        Self::new()
    }
}
