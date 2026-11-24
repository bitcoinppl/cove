//! A BIP 77 Payjoin directory, OHTTP relay and receiver for the UI tests that pay a pj URI
//!
//! The layout of the Rust end-to-end tests' harness in `rust/src/test_support/payjoin.rs`: two
//! `payjoin-mailroom` instances on IPv4 loopback, a directory that serves TLS with a self-signed
//! `localhost` certificate on a free port, and an OHTTP relay that serves plain HTTP on
//! [`UiTestPorts::payjoin_relay`] and trusts that certificate. The app reaches only the relay,
//! which debug builds use instead of the public ones while `COVE_UI_TEST_PAYJOIN_RELAY` names it.
//! The control API's `/payjoin/...` routes play the receiver, whose one coin is on the fake node:
//!
//! - `POST /payjoin/receiver` funds that coin, opens a session and answers with its pj URI
//! - `POST /payjoin/original` waits until the app's Original PSBT reaches the directory, then
//!   gives the app [`ORIGINAL_SAVE_GRACE`] to save the directory's reply, so a test knows the app
//!   has posted it and can stop the app with the session resumable; the Original stays unanswered
//! - `POST /payjoin/respond` waits for the Original PSBT if it hasn't yet, adds the coin, signs it
//!   and posts the proposal, as a receiver that joins the payment does

use std::{
    fmt::Display,
    time::{Duration, Instant},
};

use bdk_wallet::{KeychainKind, SignOptions, Wallet};
use bip39::Mnemonic;
use bitcoin::{consensus, Amount, FeeRate, Network, OutPoint, Psbt, Transaction, TxOut};
use color_eyre::{
    eyre::{bail, eyre, Context},
    Report, Result,
};
use parking_lot::Mutex;
use payjoin::{
    persist::{InMemoryPersister, OptionalTransitionOutcome},
    receive::{
        v2::{
            Initialized, Receiver, ReceiverBuilder, SessionEvent as ReceiverEvent,
            UncheckedOriginalPayload,
        },
        InputPair,
    },
    ImplementationError, OhttpKeys, Request,
};
use rustls::{pki_types::CertificateDer, RootCertStore};
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::{runtime::Runtime, task::AbortHandle};

use crate::{common::print_success, ui_test_ports::UiTestPorts, util};

/// The receiver's wallet, a BIP 39 test vector, so its coin never mixes with the app's test
/// wallet (abandon ×11 about)
const RECEIVER_MNEMONIC: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

/// The fake node's chain, which the app's tests point it at
const NETWORK: Network = Network::Signet;

/// How long the directory holds a long poll open before answering that nothing arrived yet
const DIRECTORY_LONG_POLL: Duration = Duration::from_secs(2);

/// How long `/payjoin/respond` waits for the app's Original PSBT, inside the iOS tests' 60 second
/// request timeout
const ORIGINAL_WAIT: Duration = Duration::from_secs(45);

/// The fee rate the receiver pays for its own input, as the Rust end-to-end tests' receiver does
const RECEIVER_MAX_FEE_RATE: u32 = 10;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReceiverRequest {
    /// The amount the pj URI asks for
    amount_sats: u64,

    /// The value of the receiver's coin, which it adds to the payment when it joins
    #[serde(default = "default_coin_sats")]
    coin_sats: u64,
}

const fn default_coin_sats() -> u64 {
    50_000
}

/// How long `/payjoin/original` waits after the Original arrives, for the app to save the
/// directory's reply, which crosses only the relay and takes milliseconds, so this leaves a wide
/// margin
const ORIGINAL_SAVE_GRACE: Duration = Duration::from_secs(3);

/// The directory, the relay and the open receiver session; the servers stop when it drops
pub(super) struct PayjoinHost {
    directory: Mailroom,
    relay: Mailroom,
    client: reqwest::blocking::Client,
    ports: UiTestPorts,
    receiver: Mutex<Option<HostReceiver>>,

    // dropped after the servers it runs
    _runtime: Runtime,
}

impl PayjoinHost {
    pub(super) fn start(ports: UiTestPorts) -> Result<Self> {
        // the mailroom's rustls and the directory's TLS config need a process-wide provider
        let _ = rustls::crypto::ring::default_provider().install_default();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("payjoin-mailroom")
            .enable_all()
            .build()
            .wrap_err("Failed to start the Payjoin services' runtime")?;

        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .wrap_err("Failed to make the directory's certificate")?;
        let certificate = certified.cert.der().to_vec();
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(certificate.clone()))?;

        let tls = runtime
            .block_on(axum_server::tls_rustls::RustlsConfig::from_der(
                vec![certificate.clone()],
                certified.signing_key.serialize_der(),
            ))
            .wrap_err("Failed to build the directory's TLS config")?;

        let directory = Mailroom::start(&runtime, 0, Some(tls), roots.clone())?;
        let relay = Mailroom::start(&runtime, ports.payjoin_relay(), None, roots)?;

        let client = reqwest::blocking::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_der(&certificate)?)
            .timeout(Duration::from_secs(10))
            .build()?;

        print_success(&format!(
            "Payjoin up: OHTTP relay http://127.0.0.1:{}, directory https://localhost:{}",
            relay.port, directory.port
        ));

        Ok(Self { directory, relay, client, ports, receiver: Mutex::new(None), _runtime: runtime })
    }

    /// Funds the receiver's coin, opens a session for `amount_sats` and answers with its pj URI
    ///
    /// A new session replaces one still open, so each test starts from its own
    pub(super) fn open_receiver(&self, request: ReceiverRequest) -> Result<Value> {
        let mnemonic = Mnemonic::parse(RECEIVER_MNEMONIC)?;
        let mut wallet = util::bip84_signing_wallet(&mnemonic, NETWORK)?;

        let coin_address = wallet.peek_address(KeychainKind::External, 0).address;
        let coin_tx = self.fund(&coin_address.to_string(), request.coin_sats)?;
        let vout = coin_tx
            .output
            .iter()
            .position(|output| output.script_pubkey == coin_address.script_pubkey())
            .ok_or_else(|| eyre!("The funding transaction doesn't pay the receiver"))?;
        let coin =
            (OutPoint::new(coin_tx.compute_txid(), vout as u32), coin_tx.output[vout].clone());
        wallet.apply_unconfirmed_txs([(coin_tx, 0)]);

        // receive on a fresh address so the coin's address is not reused
        let receive_address = wallet.peek_address(KeychainKind::External, 1).address;
        let persister = InMemoryPersister::default();
        let session = ReceiverBuilder::new(
            receive_address.clone(),
            self.directory_url().as_str(),
            self.ohttp_keys()?,
        )
        .map_err(failed("Failed to build the receiver session"))?
        .with_amount(Amount::from_sat(request.amount_sats))
        .build()
        .save(&persister)
        .map_err(failed("Failed to save the receiver session"))?;

        let pj_uri = session.pj_uri().to_string();
        let coin_outpoint = coin.0.to_string();
        *self.receiver.lock() =
            Some(HostReceiver { wallet, coin, persister, session, original: None });

        Ok(json!({
            "pj_uri": pj_uri,
            "address": receive_address.to_string(),
            "coin": coin_outpoint,
        }))
    }

    /// Waits until the app's Original PSBT reaches the directory, and the app has had time to
    /// save the directory's reply, and keeps the Original unanswered
    pub(super) fn wait_for_original(&self) -> Result<Value> {
        let mut receiver = self.receiver.lock();
        let Some(receiver) = receiver.as_mut() else {
            bail!("No Payjoin receiver session is open; POST /payjoin/receiver first");
        };

        if receiver.original.is_none() {
            receiver.original = Some(self.poll_for_original(receiver)?);

            // the app saves its session as posted only once the directory's reply comes back
            // through the relay, after the receiver can already read the Original, and an app
            // stopped before that save sends the fallback at its next launch
            std::thread::sleep(ORIGINAL_SAVE_GRACE);
        }

        Ok(json!({ "ok": true }))
    }

    /// Waits for the app's Original PSBT if it hasn't arrived, then joins it with the
    /// receiver's coin
    pub(super) fn respond(&self) -> Result<Value> {
        let Some(mut receiver) = self.receiver.lock().take() else {
            bail!("No Payjoin receiver session is open; POST /payjoin/receiver first");
        };

        let original = match receiver.original.take() {
            Some(original) => original,
            None => self.poll_for_original(&mut receiver)?,
        };
        self.join(&receiver, original)?;

        Ok(json!({ "ok": true, "coin": receiver.coin.0.to_string() }))
    }

    fn poll_for_original(
        &self,
        receiver: &mut HostReceiver,
    ) -> Result<Receiver<UncheckedOriginalPayload>> {
        let deadline = Instant::now() + ORIGINAL_WAIT;

        loop {
            let (request, context) = receiver
                .session
                .create_poll_request(self.relay_url().as_str())
                .map_err(failed("Failed to build the receiver's poll"))?;
            let response = self.post(request)?;
            let outcome = receiver
                .session
                .clone()
                .process_response(&response, context)
                .save(&receiver.persister)
                .map_err(failed("The directory's answer to the receiver's poll"))?;

            match outcome {
                OptionalTransitionOutcome::Progress(original) => return Ok(original),
                OptionalTransitionOutcome::Stasis(session) => receiver.session = session,
            }

            if Instant::now() >= deadline {
                bail!(
                    "The app's Original PSBT didn't reach the directory within {} seconds",
                    ORIGINAL_WAIT.as_secs()
                );
            }
        }
    }

    /// Checks the Original PSBT, adds the receiver's coin, signs it and posts the proposal
    fn join(
        &self,
        receiver: &HostReceiver,
        original: Receiver<UncheckedOriginalPayload>,
    ) -> Result<()> {
        let p = &receiver.persister;
        let wallet = &receiver.wallet;

        let proposal = original
            .check_broadcast_suitability(None, |_| Ok(true))
            .save(p)
            .map_err(failed("The Original PSBT isn't broadcastable"))?
            .check_inputs_not_owned(&mut |outpoint| Ok(wallet.get_utxo(*outpoint).is_some()))
            .save(p)
            .map_err(failed("The Original PSBT spends the receiver's coin"))?
            .check_no_inputs_seen_before(&mut |_| Ok(false))
            .save(p)
            .map_err(failed("The Original PSBT's inputs were seen before"))?
            .identify_receiver_outputs(&mut |script| Ok(wallet.is_mine(script.to_owned())))
            .save(p)
            .map_err(failed("The Original PSBT doesn't pay the receiver"))?
            .commit_outputs()
            .save(p)
            .map_err(failed("Failed to commit the outputs"))?;

        let (outpoint, txout) = receiver.coin.clone();
        let input = InputPair::new_p2wpkh(txout, outpoint)
            .map_err(failed("The receiver's coin isn't P2WPKH"))?;

        let proposal = proposal
            .contribute_inputs([input])
            .map_err(failed("Failed to add the receiver's coin"))?
            .commit_inputs()
            .save(p)
            .map_err(failed("Failed to commit the inputs"))?
            .apply_fee_range(None, Some(FeeRate::from_sat_per_vb_u32(RECEIVER_MAX_FEE_RATE)))
            .save(p)
            .map_err(failed("Failed to pay for the receiver's input"))?
            .finalize_proposal(|psbt| sign(wallet, psbt))
            .save(p)
            .map_err(failed("Failed to sign the receiver's input"))?;

        let (request, context) = proposal
            .create_post_request(self.relay_url().as_str())
            .map_err(failed("Failed to build the proposal request"))?;
        let response = self.post(request)?;

        proposal
            .process_response(&response, context)
            .save(p)
            .map_err(failed("The directory refused the proposal"))?;

        Ok(())
    }

    /// Pays `sats` to `address` on the fake node in a confirmed transaction and returns it
    fn fund(&self, address: &str, sats: u64) -> Result<Transaction> {
        let funded: Value = self
            .client
            .post(format!("http://127.0.0.1:{}/fund", self.ports.control()))
            .json(&json!({ "address": address, "sats": [sats], "confirm": true }))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .wrap_err("The fake node didn't fund the Payjoin receiver")?
            .json()?;

        let txid = funded["outputs"][0]["txid"]
            .as_str()
            .ok_or_else(|| eyre!("The fake node's /fund answer has no txid: {funded}"))?;

        let raw = self
            .client
            .get(format!("http://127.0.0.1:{}/tx/{txid}/raw", self.ports.esplora()))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .wrap_err("The fake node didn't serve the receiver's funding transaction")?
            .bytes()?;

        consensus::deserialize(&raw).wrap_err("Failed to parse the funding transaction")
    }

    /// The directory's OHTTP gateway keys, fetched directly as a receiver bootstraps them
    fn ohttp_keys(&self) -> Result<OhttpKeys> {
        let url = self.directory_url().join("/.well-known/ohttp-gateway")?;
        let body = self
            .client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .wrap_err("The directory didn't serve its OHTTP keys")?
            .bytes()?;

        OhttpKeys::decode(&body).map_err(failed("Failed to decode the directory's OHTTP keys"))
    }

    fn post(&self, request: Request) -> Result<Vec<u8>> {
        let response = self
            .client
            .post(request.url.as_str())
            .header("Content-Type", request.content_type)
            .body(request.body)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .wrap_err("The Payjoin relay didn't answer the receiver")?;

        Ok(response.bytes()?.to_vec())
    }

    fn directory_url(&self) -> url::Url {
        url::Url::parse(&format!("https://localhost:{}", self.directory.port))
            .expect("a localhost URL is valid")
    }

    fn relay_url(&self) -> url::Url {
        url::Url::parse(&format!("http://127.0.0.1:{}", self.relay.port))
            .expect("a loopback URL is valid")
    }
}

/// One receiver session and the coin it adds when it joins
struct HostReceiver {
    wallet: Wallet,
    coin: (OutPoint, TxOut),
    persister: InMemoryPersister<ReceiverEvent>,
    session: Receiver<Initialized>,

    // the app's Original PSBT once `/payjoin/original` has seen it
    original: Option<Receiver<UncheckedOriginalPayload>>,
}

fn sign(wallet: &Wallet, psbt: &Psbt) -> Result<Psbt, ImplementationError> {
    let mut psbt = psbt.clone();
    let options = SignOptions { trust_witness_utxo: true, ..SignOptions::default() };
    wallet.sign(&mut psbt, options).map_err(ImplementationError::new)?;

    Ok(psbt)
}

/// Wraps a payjoin error, whose types don't all meet eyre's bounds, with what failed
fn failed<E: Display>(context: &'static str) -> impl FnOnce(E) -> Report {
    move |error| eyre!("{context}: {error}")
}

/// One payjoin-mailroom instance, which serves as a directory or an OHTTP relay
struct Mailroom {
    port: u16,
    server: AbortHandle,

    // the server's storage, deleted when the instance drops, after its server is aborted
    _storage: TempDir,
}

impl Mailroom {
    /// Starts an instance on IPv4 loopback with its own storage, serving TLS when `tls` is given;
    /// port 0 takes a free one
    fn start(
        runtime: &Runtime,
        port: u16,
        tls: Option<axum_server::tls_rustls::RustlsConfig>,
        roots: RootCertStore,
    ) -> Result<Self> {
        let storage = tempfile::tempdir()?;
        let listener = format!("127.0.0.1:{port}")
            .parse()
            .map_err(failed("Invalid mailroom listener address"))?;
        let config = payjoin_mailroom::config::Config::new(
            listener,
            storage.path().to_path_buf(),
            DIRECTORY_LONG_POLL,
            None,
        );

        let (port, server) = runtime
            .block_on(payjoin_mailroom::serve_manual_tls(config, tls, roots, None))
            .map_err(|error| eyre!("Failed to start a Payjoin mailroom on {port}: {error:#}"))?;

        Ok(Self { port, server: server.abort_handle(), _storage: storage })
    }
}

impl Drop for Mailroom {
    fn drop(&mut self) {
        self.server.abort();
    }
}
