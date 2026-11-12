//! An in-process BIP 77 Payjoin directory, OHTTP relays and a v2 receiver for end-to-end tests
//!
//! The directory and relays are `payjoin-mailroom` instances on loopback ports: the directory
//! serves TLS with a self-signed `localhost` certificate and each relay serves plain HTTP and
//! trusts that certificate, the same layout rust-payjoin's own integration tests use. The
//! receiver is driven step by step from the test and signs with a BDK wallet whose coin lives on
//! the fake node, so the node's script check passes when the sender broadcasts the Payjoin

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Bytes,
    extract::State,
    http::{
        HeaderMap, StatusCode, Uri,
        header::{CONTENT_TYPE, LOCATION},
    },
    response::{IntoResponse as _, Response},
};
use bdk_wallet::{KeychainKind, SignOptions, Wallet as BdkWallet};
use bitcoin::{Amount, FeeRate, OutPoint, Psbt, TxOut};
use cove_fake_node::FakeChain;
use payjoin::{
    ImplementationError, OhttpKeys, PjUri, Request,
    persist::{InMemoryPersister, OptionalTransitionOutcome},
    receive::{
        InputPair,
        v2::{
            Initialized, Monitor, Receiver, ReceiverBuilder, SessionEvent as ReceiverEvent,
            UncheckedOriginalPayload,
        },
    },
};
use rustls::{RootCertStore, pki_types::CertificateDer};
use tempfile::TempDir;
use tokio::task::AbortHandle;
use url::Url;

use crate::manager::wallet_manager::payjoin::{OhttpRelays, PayjoinConfig};

/// How long the directory holds a long poll open before answering that nothing arrived yet
const DIRECTORY_LONG_POLL: Duration = Duration::from_secs(2);

/// A running directory and its relays, stopped when dropped
pub(crate) struct PayjoinServices {
    directory: Mailroom,
    relays: Vec<Mailroom>,
    certificate: CertificateDer<'static>,
}

impl PayjoinServices {
    /// Starts one TLS directory and `relay_count` plain HTTP relays on loopback ports
    pub(crate) async fn start(relay_count: usize) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("a self-signed localhost certificate");
        let certificate = CertificateDer::from(certified.cert.der().to_vec());
        let mut roots = RootCertStore::empty();
        roots.add(certificate.clone()).expect("the test certificate is a valid root");

        let tls = axum_server::tls_rustls::RustlsConfig::from_der(
            vec![certificate.to_vec()],
            certified.signing_key.serialize_der(),
        )
        .await
        .expect("the directory TLS config builds");

        let directory = Mailroom::start(Some(tls), roots.clone()).await;
        let mut relays = Vec::with_capacity(relay_count);
        for _ in 0..relay_count {
            relays.push(Mailroom::start(None, roots.clone()).await);
        }

        Self { directory, relays, certificate }
    }

    pub(crate) fn directory_url(&self) -> Url {
        Url::parse(&format!("https://localhost:{}", self.directory.port)).expect("a valid URL")
    }

    pub(crate) fn relay_urls(&self) -> Vec<Url> {
        self.relays
            .iter()
            .map(|relay| {
                Url::parse(&format!("http://localhost:{}", relay.port)).expect("a valid URL")
            })
            .collect()
    }

    /// A sender config that sends through these relays and waits up to `session_timeout`
    pub(crate) fn sender_config(&self, session_timeout: Duration) -> PayjoinConfig {
        let relays = OhttpRelays::loopback_for_test(self.relay_urls())
            .expect("loopback relays are allowed in tests");

        PayjoinConfig::new(relays, session_timeout)
    }

    /// A client that trusts the directory's certificate, for the receiver's requests
    pub(crate) fn http_client(&self) -> reqwest::Client {
        let mut roots = RootCertStore::empty();
        roots.add(self.certificate.clone()).expect("the test certificate is a valid root");
        let tls =
            rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();

        reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("the test HTTP client builds")
    }

    /// The directory's OHTTP gateway keys, fetched directly as a receiver bootstraps them
    pub(crate) async fn ohttp_keys(&self) -> OhttpKeys {
        let url = self.directory_url().join("/.well-known/ohttp-gateway").expect("a valid URL");
        let body = self
            .http_client()
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("the directory serves its OHTTP keys")
            .bytes()
            .await
            .expect("the OHTTP keys body reads");

        OhttpKeys::decode(&body).expect("the directory's OHTTP keys decode")
    }
}

/// An OHTTP relay that accepts every connection and never answers, so a sender's request
/// through it stays in flight until the sender gives up on it
pub(crate) struct SilentRelay {
    port: u16,
    connections: Arc<AtomicUsize>,
    server: AbortHandle,
}

impl SilentRelay {
    /// Listens on IPv4 loopback, as the mailroom instances do
    pub(crate) async fn start() -> Self {
        let listener =
            tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("the silent relay listens");
        let port = listener.local_addr().expect("the silent relay has an address").port();
        let connections = Arc::new(AtomicUsize::new(0));

        let accepted = connections.clone();
        let server = tokio::spawn(async move {
            // kept open and unread until the relay drops, so no request ever gets an answer
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::SeqCst);
                held.push(stream);
            }
        });

        Self { port, connections, server: server.abort_handle() }
    }

    /// A sender config whose only relay is this one, waiting up to `session_timeout`
    pub(crate) fn sender_config(&self, session_timeout: Duration) -> PayjoinConfig {
        let url = Url::parse(&format!("http://localhost:{}", self.port)).expect("a valid URL");
        let relays = OhttpRelays::loopback_for_test(vec![url])
            .expect("loopback relays are allowed in tests");

        PayjoinConfig::new(relays, session_timeout)
    }

    /// How many connections senders have opened to this relay
    pub(crate) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for SilentRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// How a [`TestRelay`] answers the requests the sender sends it
#[derive(Clone)]
pub(crate) enum TestRelayBehavior {
    /// Passes each request on to a real relay and returns its answer
    Forward(Url),
    /// Answers every request with 502, as a relay that cannot reach the directory does
    Down,
    /// Answers every request with 200 and a body that is not an OHTTP response, so the request
    /// itself succeeds but the sender's `process_response` rejects the answer
    Garbled,
    /// Answers every request with a 307 redirect to the same path on another relay
    Redirect(Url),
    /// Holds every request open without answering, as a relay that stops responding does
    Silent,
}

/// A relay a sender is configured with in place of a real one, which counts the requests it
/// gets, so a test can tell which relays a session used
pub(crate) struct TestRelay {
    url: Url,
    state: Arc<TestRelayState>,
    server: AbortHandle,
}

struct TestRelayState {
    behavior: parking_lot::Mutex<TestRelayBehavior>,
    client: reqwest::Client,
    requests: AtomicUsize,
    held: AtomicUsize,
}

impl TestRelay {
    /// A relay that forwards to the services' relay at `index`
    pub(crate) async fn forwarding_to(services: &PayjoinServices, index: usize) -> Self {
        let upstream = services.relay_urls().into_iter().nth(index).expect("the relay exists");
        Self::start(TestRelayBehavior::Forward(upstream), services.http_client()).await
    }

    /// Starts a relay on an IPv4 loopback port
    pub(crate) async fn start(behavior: TestRelayBehavior, client: reqwest::Client) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port for the test relay");
        let port = listener.local_addr().expect("the test relay has an address").port();
        let state = Arc::new(TestRelayState {
            behavior: parking_lot::Mutex::new(behavior),
            client,
            requests: AtomicUsize::new(0),
            held: AtomicUsize::new(0),
        });

        let app = axum::Router::new().fallback(relay_request).with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the test relay serves");
        });

        let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("a valid URL");
        Self { url, state, server: server.abort_handle() }
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    /// How many requests reached this relay
    pub(crate) fn requests(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }

    /// How many requests the relay is holding open without an answer
    pub(crate) fn held(&self) -> usize {
        self.state.held.load(Ordering::SeqCst)
    }

    /// Changes how the relay answers every later request
    pub(crate) fn set_behavior(&self, behavior: TestRelayBehavior) {
        *self.state.behavior.lock() = behavior;
    }

    /// A sender config that tries `relays` in the given order and waits up to `session_timeout`
    pub(crate) fn sender_config(relays: &[&Self], session_timeout: Duration) -> PayjoinConfig {
        let relays = OhttpRelays::loopback_for_test(relays.iter().map(|relay| relay.url.clone()))
            .expect("loopback relays are allowed in tests")
            .in_listed_order();

        PayjoinConfig::new(relays, session_timeout)
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn relay_request(
    State(state): State<Arc<TestRelayState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);

    // the path carries the directory's origin, which the real relay forwards to
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    let behavior = state.behavior.lock().clone();
    let upstream = match behavior {
        TestRelayBehavior::Forward(upstream) => upstream,
        TestRelayBehavior::Down => return StatusCode::BAD_GATEWAY.into_response(),
        TestRelayBehavior::Garbled => {
            return (StatusCode::OK, "not an OHTTP response").into_response();
        }
        TestRelayBehavior::Redirect(target) => {
            let location = format!("{}{path}", target.as_str().trim_end_matches('/'));
            return (StatusCode::TEMPORARY_REDIRECT, [(LOCATION, location)]).into_response();
        }
        TestRelayBehavior::Silent => {
            state.held.fetch_add(1, Ordering::SeqCst);
            return std::future::pending().await;
        }
    };

    let url = format!("{}{path}", upstream.as_str().trim_end_matches('/'));
    let mut request = state.client.post(url).body(body);
    if let Some(content_type) = headers.get(CONTENT_TYPE) {
        request = request.header(CONTENT_TYPE, content_type);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = response.status();
    match response.bytes().await {
        Ok(body) => (status, body).into_response(),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// One payjoin-mailroom instance, which serves as a directory or an OHTTP relay
struct Mailroom {
    port: u16,
    server: AbortHandle,

    // the server's storage, deleted when the instance drops, after its server is aborted
    _storage: TempDir,
}

impl Mailroom {
    /// Starts an instance with its own storage, serving TLS when `tls` is given
    ///
    /// It listens on IPv4 loopback only, so nothing off this machine can reach it. URLs still
    /// name `localhost`, which the directory's certificate is issued for
    async fn start(
        tls: Option<axum_server::tls_rustls::RustlsConfig>,
        roots: RootCertStore,
    ) -> Self {
        let storage = tempfile::tempdir().expect("a mailroom storage directory");
        let config = payjoin_mailroom::config::Config::new(
            "127.0.0.1:0".parse().expect("a valid listener address"),
            storage.path().to_path_buf(),
            DIRECTORY_LONG_POLL,
            None,
        );

        let (port, server) = payjoin_mailroom::serve_manual_tls(config, tls, roots, None)
            .await
            .expect("the mailroom instance starts");

        Self { port, server: server.abort_handle(), _storage: storage }
    }
}

impl Drop for Mailroom {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A BIP 77 receiver whose one coin is on the fake node, signed for by a BDK wallet
pub(crate) struct PayjoinReceiver {
    wallet: BdkWallet,
    coin: (OutPoint, TxOut),
    persister: InMemoryPersister<ReceiverEvent>,
    session: Receiver<Initialized>,
    relay: Url,
    client: reqwest::Client,
}

impl PayjoinReceiver {
    /// Funds `wallet` with `coin_value` on the fake chain and opens a session on `services`
    pub(crate) async fn start(
        services: &PayjoinServices,
        chain: &FakeChain,
        mut wallet: BdkWallet,
        coin_value: Amount,
    ) -> Self {
        let coin_script = wallet.peek_address(KeychainKind::External, 0).script_pubkey();
        let coin_txid = chain.pay(&coin_script, coin_value);
        let coin_tx = chain.tx(coin_txid).expect("the funding transaction is on the chain").tx;
        let coin = (OutPoint { txid: coin_txid, vout: 0 }, coin_tx.output[0].clone());
        wallet.apply_unconfirmed_txs([(coin_tx, 0)]);

        // receive on a fresh address so the coin's address is not reused
        let receive_address = wallet.peek_address(KeychainKind::External, 1).address;
        let persister = InMemoryPersister::default();
        let session = ReceiverBuilder::new(
            receive_address,
            services.directory_url().as_str(),
            services.ohttp_keys().await,
        )
        .expect("the receiver session builds")
        .build()
        .save(&persister)
        .expect("the receiver session saves");

        let relay = services.relay_urls().into_iter().next().expect("at least one relay");

        Self { wallet, coin, persister, session, relay, client: services.http_client() }
    }

    pub(crate) fn pj_uri(&self) -> PjUri {
        self.session.pj_uri()
    }

    pub(crate) fn coin_outpoint(&self) -> OutPoint {
        self.coin.0
    }

    /// Polls the directory until the sender's Original PSBT arrives
    pub(crate) async fn wait_for_original(&mut self) -> Receiver<UncheckedOriginalPayload> {
        loop {
            let (request, context) = self
                .session
                .create_poll_request(self.relay.as_str())
                .expect("the receiver poll request builds");
            let response = self.post(request).await;
            let outcome = self
                .session
                .clone()
                .process_response(&response, context)
                .save(&self.persister)
                .expect("the directory answers the receiver's poll");

            match outcome {
                OptionalTransitionOutcome::Progress(original) => return original,
                OptionalTransitionOutcome::Stasis(session) => self.session = session,
            }
        }
    }

    /// Checks the Original PSBT, adds the receiver's coin, signs it and posts the proposal
    pub(crate) async fn respond(
        &self,
        original: Receiver<UncheckedOriginalPayload>,
    ) -> Receiver<Monitor> {
        let p = &self.persister;
        let owns = |script: &bitcoin::Script| self.wallet.is_mine(script.to_owned());

        let proposal = original
            .check_broadcast_suitability(None, |_| Ok(true))
            .save(p)
            .expect("the Original PSBT is broadcastable")
            .check_inputs_not_owned(&mut |outpoint| Ok(self.wallet.get_utxo(*outpoint).is_some()))
            .save(p)
            .expect("the sender spends none of the receiver's coins")
            .check_no_inputs_seen_before(&mut |_| Ok(false))
            .save(p)
            .expect("the sender's inputs are new")
            .identify_receiver_outputs(&mut |script| Ok(owns(script)))
            .save(p)
            .expect("the Original PSBT pays the receiver")
            .commit_outputs()
            .save(p)
            .expect("the outputs commit");

        let (outpoint, txout) = self.coin.clone();
        let input = InputPair::new_p2wpkh(txout, outpoint).expect("the coin is P2WPKH");
        let proposal = proposal
            .contribute_inputs([input])
            .expect("the receiver's coin is contributed")
            .commit_inputs()
            .save(p)
            .expect("the inputs commit")
            .apply_fee_range(None, Some(FeeRate::from_sat_per_vb_u32(10)))
            .save(p)
            .expect("the receiver pays for its input")
            .finalize_proposal(|psbt| self.sign(psbt))
            .save(p)
            .expect("the receiver signs its input");

        let (request, context) =
            proposal.create_post_request(self.relay.as_str()).expect("the proposal request builds");
        let response = self.post(request).await;

        proposal
            .process_response(&response, context)
            .save(p)
            .expect("the directory accepts the proposal")
    }

    /// Rejects the Original PSBT as one it would not broadcast, and posts that well-known
    /// error to the directory for the sender
    pub(crate) async fn reject(&self, original: Receiver<UncheckedOriginalPayload>) {
        let p = &self.persister;
        let rejection = original
            .check_broadcast_suitability(None, |_| Ok(false))
            .save(p)
            .expect_err("the receiver rejects the Original PSBT")
            .fatal_state()
            .expect("the rejection can be sent to the sender");

        let (request, context) = rejection
            .create_error_request(self.relay.as_str())
            .expect("the rejection request builds");
        let response = self.post(request).await;

        rejection
            .process_error_response(&response, context)
            .save(p)
            .expect("the directory accepts the rejection");
    }

    fn sign(&self, psbt: &Psbt) -> Result<Psbt, ImplementationError> {
        let mut psbt = psbt.clone();
        let options = SignOptions { trust_witness_utxo: true, ..SignOptions::default() };
        self.wallet.sign(&mut psbt, options).map_err(ImplementationError::new)?;

        Ok(psbt)
    }

    async fn post(&self, request: Request) -> Vec<u8> {
        self.client
            .post(request.url.as_str())
            .header("Content-Type", request.content_type)
            .body(request.body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("the relay answers")
            .bytes()
            .await
            .expect("the relay's answer reads")
            .to_vec()
    }
}
