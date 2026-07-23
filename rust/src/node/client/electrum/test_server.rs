//! A TLS listener speaking just enough of the Electrum protocol to exercise
//! certificate verification.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::node::tls::{self, TlsTrust};

pub const TEST_HEIGHT: usize = 840_000;

const GENESIS_HEADER: &str = "0100000000000000000000000000000000000000000000000000000000000000000000003ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a29ab5f49ffff001d1dac2b7c";
pub const BITCOIN_GENESIS_HASH: &str =
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
pub const TESTNET_GENESIS_HASH: &str =
    "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943";

#[derive(Debug, Clone, Copy)]
enum Behavior {
    Electrum { genesis_hash: &'static str, server_features: bool },
    NonElectrum,
}

/// The client installs its own crypto provider, but the test server and cove's
/// blocking helper both need process-level setup.
pub fn setup() {
    crate::test_support::ensure_tokio_runtime();

    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

pub fn self_signed(name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let generated = rcgen::generate_simple_self_signed(vec![name.to_string()]).unwrap();

    (
        CertificateDer::from(generated.cert.der().to_vec()),
        PrivateKeyDer::try_from(generated.signing_key.serialize_der()).unwrap(),
    )
}

/// A certificate authority that issues leaves, standing in for a self hosted CA.
pub struct Authority {
    certificate: CertificateDer<'static>,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
}

impl Authority {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let key = rcgen::KeyPair::generate().unwrap();

        let mut params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        // A distinct name per authority, so an unrelated CA is rejected as an
        // unknown issuer rather than on a signature mismatch.
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            format!("cove test ca {}", NEXT.fetch_add(1, Ordering::Relaxed)),
        );
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages =
            vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];

        let certificate = params.self_signed(&key).unwrap();
        let certificate = CertificateDer::from(certificate.der().to_vec());

        Self { certificate, issuer: rcgen::Issuer::new(params, key) }
    }

    pub fn trust(&self) -> TlsTrust {
        TlsTrust::CustomCa { cert: self.certificate.as_ref().to_vec() }
    }

    fn issue(&self, name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap();
        let leaf = params.signed_by(&key, &self.issuer).unwrap();

        (
            CertificateDer::from(leaf.der().to_vec()),
            PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
        )
    }
}

pub struct TestServer {
    pub port: u16,
    certificate: CertificateDer<'static>,
    hang_up: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
}

impl TestServer {
    pub fn self_signed(name: &str) -> Self {
        let (certificate, key) = self_signed(name);
        Self::spawn(
            "127.0.0.1:0",
            certificate,
            vec![],
            key,
            Behavior::Electrum { genesis_hash: BITCOIN_GENESIS_HASH, server_features: true },
        )
        .expect("bind loopback")
    }

    pub fn self_signed_with_genesis(
        name: &str,
        genesis_hash: &'static str,
        server_features: bool,
    ) -> Self {
        let (certificate, key) = self_signed(name);
        Self::spawn(
            "127.0.0.1:0",
            certificate,
            vec![],
            key,
            Behavior::Electrum { genesis_hash, server_features },
        )
        .expect("bind loopback")
    }

    pub fn non_electrum(name: &str) -> Self {
        let (certificate, key) = self_signed(name);
        Self::spawn("127.0.0.1:0", certificate, vec![], key, Behavior::NonElectrum)
            .expect("bind loopback")
    }

    /// Returns `None` where IPv6 loopback is unavailable.
    pub fn self_signed_ipv6(name: &str) -> Option<Self> {
        let (certificate, key) = self_signed(name);
        Self::spawn(
            "[::1]:0",
            certificate,
            vec![],
            key,
            Behavior::Electrum { genesis_hash: BITCOIN_GENESIS_HASH, server_features: true },
        )
    }

    /// Serves a leaf issued by `authority`, presenting the CA in the chain the
    /// way a real server would.
    pub fn issued_by(authority: &Authority, name: &str) -> Self {
        let (certificate, key) = authority.issue(name);
        let chain = vec![authority.certificate.clone()];

        Self::spawn(
            "127.0.0.1:0",
            certificate,
            chain,
            key,
            Behavior::Electrum { genesis_hash: BITCOIN_GENESIS_HASH, server_features: true },
        )
        .expect("bind loopback")
    }

    pub fn fingerprint_trust(&self) -> TlsTrust {
        TlsTrust::PinnedFingerprint { sha256: tls::fingerprint(&self.certificate).to_vec() }
    }

    /// Close the connection after the next answer, so a caller has to
    /// reconnect. Only the next one, the way a real dropped socket behaves.
    pub fn hang_up_once(&self) {
        self.hang_up.store(true, Ordering::Relaxed);
    }

    pub fn request_count(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    fn spawn(
        bind: &str,
        certificate: CertificateDer<'static>,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        behavior: Behavior,
    ) -> Option<Self> {
        let mut presented = vec![certificate.clone()];
        presented.extend(chain);

        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(presented, key)
                .unwrap();

        let listener = TcpListener::bind(bind).ok()?;
        let port = listener.local_addr().unwrap().port();

        let config = Arc::new(config);
        let hang_up = Arc::new(AtomicBool::new(false));
        let server_hang_up = hang_up.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();

        std::thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                let config = config.clone();
                let hang_up = server_hang_up.clone();
                let requests = server_requests.clone();

                std::thread::spawn(move || {
                    let Ok(session) = ServerConnection::new(config) else { return };
                    let mut reader = BufReader::new(StreamOwned::new(session, tcp));
                    let mut line = String::new();

                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        requests.fetch_add(1, Ordering::Relaxed);
                        if reader.get_mut().write_all(response(&line, behavior).as_bytes()).is_err()
                        {
                            return;
                        }

                        let _ = reader.get_mut().flush();

                        if hang_up.swap(false, Ordering::Relaxed) {
                            return;
                        }

                        line.clear();
                    }
                });
            }
        });

        Some(Self { port, certificate, hang_up, requests })
    }
}

fn response(request: &str, behavior: Behavior) -> String {
    let id = request
        .split("\"id\":")
        .nth(1)
        .and_then(|rest| rest.trim_start().split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse::<u64>().ok())
        .unwrap_or(0);

    let Behavior::Electrum { genesis_hash, server_features } = behavior else {
        return "HTTP/1.1 400 Bad Request\r\n\r\n".to_string();
    };

    let result = if request.contains("\"server.version\"") {
        "[\"Cove test server\",\"1.4\"]".to_string()
    } else if request.contains("\"server.features\"") && server_features {
        format!(
            "{{\"server_version\":\"Fulcrum test\",\"genesis_hash\":\"{genesis_hash}\",\"protocol_min\":\"1.4\",\"protocol_max\":\"1.4\",\"hash_function\":\"sha256\",\"pruning\":null}}"
        )
    } else if request.contains("\"server.features\"") {
        return format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":-32601,\"message\":\"method not found\"}}}}\n"
        );
    } else if request.contains("\"blockchain.block.header\"") {
        format!("\"{GENESIS_HEADER}\"")
    } else {
        format!("{{\"height\":{TEST_HEIGHT},\"hex\":\"{GENESIS_HEADER}\"}}")
    };

    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}\n")
}
