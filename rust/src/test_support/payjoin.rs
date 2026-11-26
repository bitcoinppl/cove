//! An in-process BIP 77 Payjoin directory, OHTTP relays and a v2 receiver for end-to-end tests
//!
//! The directory and relays are `payjoin-mailroom` instances on loopback ports: the directory
//! serves TLS with a self-signed `localhost` certificate and each relay serves plain HTTP and
//! trusts that certificate, the same layout rust-payjoin's own integration tests use. The
//! receiver is driven step by step from the test and signs with a BDK wallet whose coin lives on
//! the fake node, so the node's script check passes when the sender broadcasts the Payjoin

use std::time::Duration;

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
