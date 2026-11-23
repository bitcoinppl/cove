//! A SOCKS5 proxy that records every stream it opens, standing in for Tor in leak tests
//!
//! It is the built-in Tor bridge with a connector that dials loopback instead of Tor. It resolves
//! host names only from its own table, the way a Tor exit resolves them. A client that looked a
//! name up itself before connecting would fail on the `.invalid` names the tests use, so a
//! recorded name proves the name went through the proxy

#![allow(clippy::disallowed_methods, reason = "the fixture dials the servers behind it")]

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use cove_http::{ProxyAddress, SocksCredentials, SocksProxy};
use cove_tor::{
    SocksAddr, SocksStatus,
    bridge::{BoundBridge, Connector},
};
use parking_lot::Mutex;
use tokio::{net::TcpStream, sync::MutexGuard};
use tokio_util::sync::CancellationToken;

use crate::{
    database::Database,
    network::Network,
    node::Node,
    test_support::{ensure_tokio_runtime, global_state_test_lock},
    tor::{TorService, TorSettings},
};

/// A name only the test proxy resolves; a system lookup of it fails
pub(crate) const PROXY_ONLY_HOST: &str = "fake-node.invalid";

/// A stream the proxy was asked to open
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProxiedTarget {
    /// The client sent a name for the proxy to resolve
    Name(String, u16),
    /// The client resolved the name itself and sent an address
    Address(SocketAddr),
}

pub(crate) struct RecordingSocksProxy {
    proxy: SocksProxy,
    targets: Arc<Mutex<Vec<ProxiedTarget>>>,
}

impl RecordingSocksProxy {
    /// Starts a proxy without auth, as an external Tor proxy such as Orbot runs
    pub(crate) fn start() -> Self {
        Self::start_with(None)
    }

    /// Starts a proxy that needs a username and password, as built-in Tor's bridge does
    pub(crate) fn start_with_credentials() -> Self {
        let credentials =
            SocksCredentials { username: "cove".to_string(), password: "test-secret".to_string() };
        Self::start_with(Some(credentials))
    }

    fn start_with(credentials: Option<SocksCredentials>) -> Self {
        let bridge = BoundBridge::bind(credentials).expect("proxy binds");
        let proxy = bridge.proxy().clone();
        let targets = Arc::new(Mutex::new(Vec::new()));
        let connector = Arc::new(RecordingConnector { targets: targets.clone() });

        // its own runtime outlives each test's runtime and never waits on the app's
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("proxy runtime starts");

            runtime.block_on(bridge.serve(connector, CancellationToken::new()));
        });

        Self { proxy, targets }
    }

    pub(crate) fn proxy_address(&self) -> ProxyAddress {
        self.proxy.address.clone()
    }

    /// The proxy with its credentials, as a route into Tor carries it
    pub(crate) fn socks_proxy(&self) -> SocksProxy {
        self.proxy.clone()
    }

    pub(crate) fn targets(&self) -> Vec<ProxiedTarget> {
        self.targets.lock().clone()
    }
}

struct RecordingConnector {
    targets: Arc<Mutex<Vec<ProxiedTarget>>>,
}

impl Connector for RecordingConnector {
    type Stream = TcpStream;

    async fn connect(&self, target: SocksAddr, port: u16) -> Result<TcpStream, SocksStatus> {
        let (recorded, destination) = match target {
            SocksAddr::Ip(ip) => {
                let address = SocketAddr::new(ip, port);
                (ProxiedTarget::Address(address), Some(address))
            }

            SocksAddr::Hostname(name) => {
                let name: &str = name.as_ref();
                let destination = (name == PROXY_ONLY_HOST)
                    .then(|| SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
                (ProxiedTarget::Name(name.to_string(), port), destination)
            }
        };

        self.targets.lock().push(recorded);

        let destination = destination.ok_or(SocksStatus::HOST_UNREACHABLE)?;
        TcpStream::connect(destination).await.map_err(|_| SocksStatus::CONNECTION_REFUSED)
    }
}

/// A listener that counts connections and closes them, standing in for a server that must
/// never be reached directly
pub(crate) struct ConnectionCounter {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
}

impl ConnectionCounter {
    pub(crate) fn start() -> Self {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("counter binds");
        let address = listener.local_addr().expect("counter has an address");
        let count = Arc::new(AtomicUsize::new(0));
        let counted = count.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });

        Self { address, count }
    }

    pub(crate) const fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

/// A loopback address with nothing listening, for a proxy that is down
pub(crate) fn closed_proxy_address() -> ProxyAddress {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a port can be bound");
    let address = listener.local_addr().expect("listener has an address");
    drop(listener);

    ProxyAddress::new_ip(address.ip(), address.port())
}

/// Holds the global test lock with Tor off, and turns Tor back off when the test ends
pub(crate) struct TorTest {
    _lock: MutexGuard<'static, ()>,
}

impl TorTest {
    pub(crate) async fn start() -> Self {
        ensure_tokio_runtime();
        let lock = global_state_test_lock().lock().await;
        reset_tor();

        Self { _lock: lock }
    }

    pub(crate) fn use_proxy(&self, proxy: ProxyAddress) {
        TorService::global().set_settings(TorSettings::Proxy(proxy)).expect("Tor proxy is saved");
    }
}

impl Drop for TorTest {
    fn drop(&mut self) {
        reset_tor();
    }
}

fn reset_tor() {
    // an onion node would block turning Tor off
    let config = &Database::global().global_config;
    config.set_selected_node(&Node::default(Network::Bitcoin)).expect("default node is saved");
    TorService::global().set_settings(TorSettings::Off).expect("Tor turns off");
}
