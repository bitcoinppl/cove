use std::borrow::Borrow;
use std::net::TcpStream;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bdk_electrum::electrum_client::raw_client::{ElectrumSslStream, RawClient};
use bdk_electrum::electrum_client::{
    Batch, BroadcastPackageRes, Client, ElectrumApi, Error, EstimationMode, GetBalanceRes,
    GetHeadersRes, GetHistoryRes, GetMerkleRes, ListUnspentRes, MempoolInfoRes, Param,
    RawHeaderNotification, ScriptStatus, ServerFeaturesRes, TxidFromPosRes,
};
use bitcoin::{Script, Txid};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConnection, StreamOwned};
use tracing::debug;
use url::{Host, Url};

use crate::node::tls::{self, TlsTrust};

const DEFAULT_SSL_PORT: u16 = 50002;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounds a peer that accepts a connection and then stalls, which would
/// otherwise hold a blocking thread for the lifetime of the process. Generous
/// enough that a slow batch response is not mistaken for a dead connection.
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// Which Electrum client backs a connection.
///
/// [`Client`] stays the default. A custom certificate needs a TLS session built
/// from our own [`rustls::ClientConfig`], and the only electrum-client entry
/// point that accepts one is [`RawClient`].
pub enum Transport {
    Default(Box<Client>),
    Pinned(Box<Pinned>),
}

impl Transport {
    pub fn connect_pinned(url: &str, trust: &TlsTrust) -> Result<Self, ConnectError> {
        Ok(Self::Pinned(Box::new(Pinned::connect(url, trust)?)))
    }
}

/// A connection with custom certificate settings, plus what is needed to
/// rebuild it.
///
/// [`Client`] reconnects internally on failure and [`RawClient`] does not, but
/// the client is cached for the lifetime of a wallet, so without this a single
/// dropped socket would break the node until the app restarts.
pub struct Pinned {
    url: String,
    trust: TlsTrust,
    connection: RwLock<Connection>,
}

struct Connection {
    client: RawClient<ElectrumSslStream>,
    /// Bumped on every reconnect, so callers that failed on an older connection
    /// can tell it has already been replaced.
    generation: u64,
}

impl Pinned {
    fn connect(url: &str, trust: &TlsTrust) -> Result<Self, ConnectError> {
        let client = connect(url, trust)?;

        Ok(Self {
            url: url.to_string(),
            trust: trust.clone(),
            connection: RwLock::new(Connection { client, generation: 0 }),
        })
    }

    /// Run `call`, reconnecting once if the connection turned out to be dead.
    ///
    /// Mirrors electrum-client's retry policy: a protocol error came from the
    /// server and will repeat, anything else may be a broken socket.
    fn call<T>(
        &self,
        call: impl Fn(&RawClient<ElectrumSslStream>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        // Scoped so the read guard is released before the write below.
        let (error, generation) = {
            let connection = self.read();

            match call(&connection.client) {
                Ok(value) => return Ok(value),
                Err(error @ (Error::Protocol(_) | Error::AlreadySubscribed(_))) => {
                    return Err(error);
                }
                Err(error) => (error, connection.generation),
            }
        };

        debug!("electrum call failed ({error}), reconnecting to {}", self.url);

        {
            let mut connection = self.write();

            // Another caller may already have replaced the dead connection.
            if connection.generation == generation {
                connection.client = connect(&self.url, &self.trust)
                    .map_err(|error| Error::Message(error.to_string()))?;
                connection.generation += 1;
            }
        }

        call(&self.read().client)
    }

    /// A panic in one call must not wedge the node for the rest of the session,
    /// so poisoning is ignored rather than propagated.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Connection> {
        self.connection.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Connection> {
        self.connection.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("custom certificates require an ssl:// url")]
    NotSsl,

    #[error("node url has no host")]
    MissingHost,

    #[error("invalid host name: {0}")]
    InvalidHost(String),

    #[error("the server's certificate does not match this node's certificate settings: {0}")]
    CertificateRejected(CertificateError),

    #[error(transparent)]
    Tls(#[from] tls::Error),

    #[error("the server did not present a certificate")]
    NoCertificate,

    #[error("failed to establish a TLS session: {0}")]
    Session(rustls::Error),

    #[error("failed to connect: {0}")]
    Connect(std::io::Error),
}

fn connect(url: &str, trust: &TlsTrust) -> Result<RawClient<ElectrumSslStream>, ConnectError> {
    let (host, port) = target(url)?;

    // A pinned fingerprint ignores this name, but rustls still requires a
    // syntactically valid one to open the session.
    let server_name = ServerName::try_from(host.as_str())
        .map_err(|_| ConnectError::InvalidHost(host.clone()))?
        .to_owned();

    handshake(server_name, tls::client_config(trust)?, &host, port).map(RawClient::from)
}

/// Read the certificate a server presents, so the user can confirm it before
/// pinning it.
///
/// Nothing about the certificate is verified here, and the connection is closed
/// without being used. The value only becomes trusted once the user accepts it.
pub(crate) fn peer_certificate(url: &str) -> Result<CertificateDer<'static>, ConnectError> {
    let (host, port) = target(url)?;

    let server_name = ServerName::try_from(host.as_str())
        .map_err(|_| ConnectError::InvalidHost(host.clone()))?
        .to_owned();

    let capture = Arc::new(tls::CapturedCertificate::default());
    handshake(server_name, tls::capture_config(capture.clone())?, &host, port)?;

    capture.take().ok_or(ConnectError::NoCertificate)
}

/// Connect and complete the TLS handshake, so a rejected certificate is
/// reported while creating the client rather than on the first call, which is
/// what the default path already does.
fn handshake(
    server_name: ServerName<'static>,
    config: rustls::ClientConfig,
    host: &str,
    port: u16,
) -> Result<ElectrumSslStream, ConnectError> {
    let mut session =
        ClientConnection::new(Arc::new(config), server_name).map_err(ConnectError::Session)?;

    let mut tcp = connect_timeout(host, port)?;
    tcp.set_read_timeout(Some(IO_TIMEOUT)).map_err(ConnectError::Connect)?;
    tcp.set_write_timeout(Some(IO_TIMEOUT)).map_err(ConnectError::Connect)?;

    session.complete_io(&mut tcp).map_err(handshake_error)?;

    Ok(StreamOwned::new(session, tcp))
}

/// Split an `ssl://` node url into a host that the resolver and rustls both accept.
fn target(url: &str) -> Result<(String, u16), ConnectError> {
    if !url.starts_with("ssl://") {
        return Err(ConnectError::NotSsl);
    }

    let parsed = Url::parse(url).map_err(|_| ConnectError::InvalidHost(url.to_string()))?;
    let port = parsed.port().unwrap_or(DEFAULT_SSL_PORT);

    let host = match parsed.host().ok_or(ConnectError::MissingHost)? {
        // Displaying an IPv6 host keeps the brackets, which neither rustls nor
        // the resolver accepts.
        Host::Ipv6(ip) => ip.to_string(),
        host => host.to_string(),
    };

    Ok((host, port))
}

/// Only a rejected certificate counts as one. Every other handshake failure,
/// such as an `ssl://` url pointed at a plaintext port, stays a connection
/// error so the user is never asked to trust their way out of it.
fn handshake_error(error: std::io::Error) -> ConnectError {
    match error.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(reason)) => {
            ConnectError::CertificateRejected(reason.clone())
        }
        _ => ConnectError::Connect(error),
    }
}

fn connect_timeout(host: &str, port: u16) -> Result<TcpStream, ConnectError> {
    use std::net::ToSocketAddrs as _;

    let mut last = None;
    for addr in (host, port).to_socket_addrs().map_err(ConnectError::Connect)? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }

    Err(ConnectError::Connect(last.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no addresses for host")
    })))
}

/// `ElectrumApi` is not object safe, since `raw_call` is generic over its
/// parameters, so the two backends are dispatched by hand.
macro_rules! dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            Transport::Default(inner) => inner.$method($($arg),*),
            Transport::Pinned(pinned) => pinned.call(|inner| inner.$method($($arg),*)),
        }
    };
}

impl ElectrumApi for Transport {
    fn raw_call(
        &self,
        method_name: &str,
        params: impl IntoIterator<Item = Param>,
    ) -> Result<serde_json::Value, Error> {
        // Collected so that a retry can replay them.
        let params = params.into_iter().collect::<Vec<_>>();
        dispatch!(self, raw_call, method_name, params.clone())
    }

    fn batch_call(&self, batch: &Batch) -> Result<Vec<serde_json::Value>, Error> {
        dispatch!(self, batch_call, batch)
    }

    fn block_headers_subscribe_raw(&self) -> Result<RawHeaderNotification, Error> {
        dispatch!(self, block_headers_subscribe_raw)
    }

    fn block_headers_pop_raw(&self) -> Result<Option<RawHeaderNotification>, Error> {
        dispatch!(self, block_headers_pop_raw)
    }

    fn block_header_raw(&self, height: usize) -> Result<Vec<u8>, Error> {
        dispatch!(self, block_header_raw, height)
    }

    fn block_headers(&self, start_height: usize, count: usize) -> Result<GetHeadersRes, Error> {
        dispatch!(self, block_headers, start_height, count)
    }

    fn estimate_fee(&self, number: usize, mode: Option<EstimationMode>) -> Result<f64, Error> {
        dispatch!(self, estimate_fee, number, mode)
    }

    fn relay_fee(&self) -> Result<f64, Error> {
        dispatch!(self, relay_fee)
    }

    fn script_subscribe(&self, script: &Script) -> Result<Option<ScriptStatus>, Error> {
        dispatch!(self, script_subscribe, script)
    }

    fn script_unsubscribe(&self, script: &Script) -> Result<bool, Error> {
        dispatch!(self, script_unsubscribe, script)
    }

    fn script_pop(&self, script: &Script) -> Result<Option<ScriptStatus>, Error> {
        dispatch!(self, script_pop, script)
    }

    fn script_get_balance(&self, script: &Script) -> Result<GetBalanceRes, Error> {
        dispatch!(self, script_get_balance, script)
    }

    fn script_get_history(&self, script: &Script) -> Result<Vec<GetHistoryRes>, Error> {
        dispatch!(self, script_get_history, script)
    }

    fn script_list_unspent(&self, script: &Script) -> Result<Vec<ListUnspentRes>, Error> {
        dispatch!(self, script_list_unspent, script)
    }

    fn transaction_get_raw(&self, txid: &Txid) -> Result<Vec<u8>, Error> {
        dispatch!(self, transaction_get_raw, txid)
    }

    fn transaction_broadcast_raw(&self, raw_tx: &[u8]) -> Result<Txid, Error> {
        dispatch!(self, transaction_broadcast_raw, raw_tx)
    }

    fn transaction_get_merkle(&self, txid: &Txid, height: usize) -> Result<GetMerkleRes, Error> {
        dispatch!(self, transaction_get_merkle, txid, height)
    }

    fn txid_from_pos(&self, height: usize, tx_pos: usize) -> Result<Txid, Error> {
        dispatch!(self, txid_from_pos, height, tx_pos)
    }

    fn server_features(&self) -> Result<ServerFeaturesRes, Error> {
        dispatch!(self, server_features)
    }

    fn mempool_get_info(&self) -> Result<MempoolInfoRes, Error> {
        dispatch!(self, mempool_get_info)
    }

    fn ping(&self) -> Result<(), Error> {
        dispatch!(self, ping)
    }

    fn calls_made(&self) -> Result<usize, Error> {
        dispatch!(self, calls_made)
    }

    fn batch_script_subscribe<'s, I>(&self, scripts: I) -> Result<Vec<Option<ScriptStatus>>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<&'s Script>,
    {
        dispatch!(self, batch_script_subscribe, scripts.clone())
    }

    fn batch_script_get_balance<'s, I>(&self, scripts: I) -> Result<Vec<GetBalanceRes>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<&'s Script>,
    {
        dispatch!(self, batch_script_get_balance, scripts.clone())
    }

    fn batch_script_get_history<'s, I>(&self, scripts: I) -> Result<Vec<Vec<GetHistoryRes>>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<&'s Script>,
    {
        dispatch!(self, batch_script_get_history, scripts.clone())
    }

    fn batch_script_list_unspent<'s, I>(
        &self,
        scripts: I,
    ) -> Result<Vec<Vec<ListUnspentRes>>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<&'s Script>,
    {
        dispatch!(self, batch_script_list_unspent, scripts.clone())
    }

    fn batch_transaction_get_raw<'t, I>(&self, txids: I) -> Result<Vec<Vec<u8>>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<&'t Txid>,
    {
        dispatch!(self, batch_transaction_get_raw, txids.clone())
    }

    fn batch_block_header_raw<I>(&self, heights: I) -> Result<Vec<Vec<u8>>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<u32>,
    {
        dispatch!(self, batch_block_header_raw, heights.clone())
    }

    fn batch_estimate_fee<I>(&self, numbers: I) -> Result<Vec<f64>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<usize>,
    {
        dispatch!(self, batch_estimate_fee, numbers.clone())
    }

    fn transaction_broadcast_package_raw<T: AsRef<[u8]>>(
        &self,
        raw_txs: &[T],
    ) -> Result<BroadcastPackageRes, Error> {
        dispatch!(self, transaction_broadcast_package_raw, raw_txs)
    }

    fn batch_transaction_get_merkle<I>(
        &self,
        txids_and_heights: I,
    ) -> Result<Vec<GetMerkleRes>, Error>
    where
        I: IntoIterator + Clone,
        I::Item: Borrow<(Txid, usize)>,
    {
        dispatch!(self, batch_transaction_get_merkle, txids_and_heights.clone())
    }

    fn txid_from_pos_with_merkle(
        &self,
        height: usize,
        tx_pos: usize,
    ) -> Result<TxidFromPosRes, Error> {
        dispatch!(self, txid_from_pos_with_merkle, height, tx_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::client::electrum::ElectrumClient;
    use crate::node::client::electrum::test_server::{
        Authority, TEST_HEIGHT, TestServer, self_signed, setup,
    };
    use crate::node::{ApiType, Node};
    use cove_types::network::Network;

    fn test_node(host: &str, port: u16, trust: Option<TlsTrust>) -> Node {
        Node {
            name: "test".to_string(),
            network: Network::Bitcoin,
            api_type: ApiType::Electrum,
            url: format!("ssl://{host}:{port}"),
            tls: trust,
        }
    }

    async fn get_height(node: &Node) -> Result<usize, String> {
        let client = ElectrumClient::new_from_node(node).await.map_err(|e| e.to_string())?;
        client.get_height().await.map_err(|e| e.to_string())
    }

    #[tokio::test]
    async fn pinned_fingerprint_accepts_a_self_signed_server() {
        setup();
        let server = TestServer::self_signed("localhost");
        let node = test_node("localhost", server.port, Some(server.fingerprint_trust()));

        assert_eq!(get_height(&node).await.unwrap(), TEST_HEIGHT);
    }

    /// The case from the bug report: a server reached by IP, with a certificate
    /// that has no matching SAN.
    #[tokio::test]
    async fn pinned_fingerprint_ignores_a_hostname_mismatch() {
        setup();
        let server = TestServer::self_signed("fulcrum.local");
        let node = test_node("127.0.0.1", server.port, Some(server.fingerprint_trust()));

        assert_eq!(get_height(&node).await.unwrap(), TEST_HEIGHT);
    }

    #[tokio::test]
    async fn pinned_fingerprint_connects_over_ipv6() {
        setup();
        let Some(server) = TestServer::self_signed_ipv6("fulcrum.local") else { return };
        let node = test_node("[::1]", server.port, Some(server.fingerprint_trust()));

        assert_eq!(get_height(&node).await.unwrap(), TEST_HEIGHT);
    }

    #[tokio::test]
    async fn pinned_fingerprint_rejects_a_different_certificate() {
        setup();
        let server = TestServer::self_signed("localhost");
        let other = TlsTrust::PinnedFingerprint {
            sha256: tls::fingerprint(&self_signed("localhost").0).to_vec(),
        };

        let error =
            get_height(&test_node("localhost", server.port, Some(other))).await.unwrap_err();
        assert!(error.contains("does not match this node's certificate settings"), "{error}");
    }

    /// A privately issued leaf must validate against its CA, which is what lets
    /// the leaf rotate without the user pinning it again.
    #[tokio::test]
    async fn custom_ca_accepts_a_leaf_it_issued() {
        setup();
        let authority = Authority::new();
        let server = TestServer::issued_by(&authority, "localhost");
        let node = test_node("localhost", server.port, Some(authority.trust()));

        assert_eq!(get_height(&node).await.unwrap(), TEST_HEIGHT);

        // A freshly issued leaf from the same CA must still be trusted.
        let rotated = TestServer::issued_by(&authority, "localhost");
        let rotated = test_node("localhost", rotated.port, Some(authority.trust()));

        assert_eq!(get_height(&rotated).await.unwrap(), TEST_HEIGHT);
    }

    #[tokio::test]
    async fn custom_ca_rejects_a_leaf_from_another_authority() {
        setup();
        let server = TestServer::issued_by(&Authority::new(), "localhost");
        let node = test_node("localhost", server.port, Some(Authority::new().trust()));

        let error = get_height(&node).await.unwrap_err();
        assert!(error.contains("UnknownIssuer"), "{error}");
    }

    /// Unlike a pinned fingerprint, a custom CA still enforces the hostname.
    #[tokio::test]
    async fn custom_ca_still_checks_the_hostname() {
        setup();
        let authority = Authority::new();
        let server = TestServer::issued_by(&authority, "fulcrum.home.arpa");
        let node = test_node("127.0.0.1", server.port, Some(authority.trust()));

        let error = get_height(&node).await.unwrap_err();
        assert!(error.contains("not valid for name"), "{error}");
    }

    /// A node with no certificate settings must behave exactly as before.
    #[tokio::test]
    async fn default_trust_still_rejects_a_self_signed_server() {
        setup();
        let server = TestServer::self_signed("localhost");

        let error = get_height(&test_node("localhost", server.port, None)).await.unwrap_err();
        assert!(error.contains("UnknownIssuer"), "{error}");
    }

    /// The client is cached for the lifetime of a wallet, so it has to survive
    /// the server dropping the connection.
    #[tokio::test]
    async fn a_pinned_connection_recovers_when_the_server_hangs_up() {
        setup();
        let server = TestServer::self_signed("localhost");
        let node = test_node("localhost", server.port, Some(server.fingerprint_trust()));

        let client = ElectrumClient::new_from_node(&node).await.unwrap();
        assert_eq!(client.get_height().await.unwrap(), TEST_HEIGHT);

        server.hang_up_once();
        assert_eq!(client.get_height().await.unwrap(), TEST_HEIGHT);
        assert_eq!(client.get_height().await.unwrap(), TEST_HEIGHT, "did not reconnect");
    }

    /// The UI offers to trust a certificate only for this failure, so it must
    /// not be confused with a node that is simply unreachable.
    #[tokio::test]
    async fn a_rejected_certificate_is_reported_as_a_certificate_problem() {
        setup();
        let server = TestServer::self_signed("localhost");

        let error = test_node("localhost", server.port, None).check_url().await.unwrap_err();
        assert!(error.is_certificate_error(), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_node_is_not_a_certificate_problem() {
        setup();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let error = test_node("127.0.0.1", port, None).check_url().await.unwrap_err();
        assert!(!error.is_certificate_error(), "{error}");
    }

    /// The whole journey the settings screen puts a user through.
    #[tokio::test]
    async fn accepting_a_certificate_ends_in_a_working_connection() {
        setup();
        let server = TestServer::self_signed("fulcrum.local");
        let url = format!("ssl://127.0.0.1:{}", server.port);

        // The node cannot be verified, which is what prompts the question.
        let error = test_node("127.0.0.1", server.port, None).check_url().await.unwrap_err();
        assert!(error.is_certificate_error(), "{error}");

        // The certificate is read so its fingerprint can be shown.
        let certificate = peer_certificate(&url).unwrap();
        let sha256 = tls::fingerprint(&certificate);

        // Accepting it pins that certificate, and the node now connects.
        let trust = TlsTrust::PinnedFingerprint { sha256: sha256.to_vec() };
        test_node("127.0.0.1", server.port, Some(trust)).check_url().await.unwrap();
    }

    /// The reconnect path takes a read lock and then a write lock, so several
    /// callers hitting a dead connection at once must not deadlock.
    #[tokio::test]
    async fn concurrent_calls_survive_a_dropped_connection() {
        setup();
        let server = TestServer::self_signed("localhost");
        let node = test_node("localhost", server.port, Some(server.fingerprint_trust()));

        let client = ElectrumClient::new_from_node(&node).await.unwrap();
        server.hang_up_once();

        let mut calls = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            calls.push(tokio::spawn(async move { client.get_height().await }));
        }

        for call in calls {
            assert_eq!(call.await.unwrap().unwrap(), TEST_HEIGHT);
        }
    }

    /// The fingerprint offered to the user has to be the server's real one.
    #[tokio::test]
    async fn the_certificate_read_matches_what_the_server_presents() {
        setup();
        let server = TestServer::self_signed("localhost");

        let read = peer_certificate(&format!("ssl://localhost:{}", server.port)).unwrap();

        assert_eq!(
            TlsTrust::PinnedFingerprint { sha256: tls::fingerprint(&read).to_vec() },
            server.fingerprint_trust()
        );
    }

    #[tokio::test]
    async fn reading_a_certificate_from_an_unreachable_host_fails() {
        setup();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let error = peer_certificate(&format!("ssl://127.0.0.1:{port}")).unwrap_err();
        assert!(matches!(error, ConnectError::Connect(_)), "{error}");
    }

    #[test]
    fn a_custom_certificate_needs_an_ssl_url() {
        assert!(matches!(target("tcp://localhost:50001"), Err(ConnectError::NotSsl)));
    }

    #[test]
    fn targets_default_to_the_electrum_ssl_port() {
        assert_eq!(target("ssl://node.example.com").unwrap(), ("node.example.com".into(), 50002));
        assert_eq!(target("ssl://node.example.com:993").unwrap(), ("node.example.com".into(), 993));
    }

    #[test]
    fn targets_strip_ipv6_brackets() {
        assert_eq!(target("ssl://[::1]:50002").unwrap(), ("::1".into(), 50002));
    }
}
