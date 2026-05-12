use std::time::Duration;

/// Build a reqwest Client that uses webpki-roots for TLS cert verification,
/// bypassing rustls-platform-verifier (which requires Android JNI init)
pub fn new_client() -> Result<reqwest::Client, reqwest::Error> {
    build_client(None)
}

/// Build a reqwest Client with an optional SOCKS5 proxy for TOR support.
/// The `socks5_url` should be in the format "socks5://127.0.0.1:9050".
pub fn new_client_with_proxy(socks5_url: &str) -> Result<reqwest::Client, reqwest::Error> {
    build_client(Some(socks5_url))
}

fn build_client(socks5_url: Option<&str>) -> Result<reqwest::Client, reqwest::Error> {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config =
        rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();

    let mut builder = reqwest::ClientBuilder::new()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .tls_backend_preconfigured(tls_config);

    if let Some(proxy_url) = socks5_url {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }

    builder.build()
}
