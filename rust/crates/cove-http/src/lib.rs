use std::time::Duration;

/// Build a reqwest Client that uses webpki-roots for TLS cert verification,
/// bypassing rustls-platform-verifier (which requires Android JNI init)
pub fn new_client() -> Result<reqwest::Client, reqwest::Error> {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config =
        rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();

    reqwest::ClientBuilder::new()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .tls_backend_preconfigured(tls_config)
        .build()
}
