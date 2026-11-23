//! A SOCKS5 proxy that records every stream it opens, standing in for Tor in leak tests
//!
//! It resolves host names only from its own table, the way a Tor exit resolves them. A client
//! that looked a name up itself before connecting would fail on the `.invalid` names the tests
//! use, so a recorded name proves the name went through the proxy

#![allow(clippy::disallowed_methods, reason = "the fixture dials the servers behind it")]

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use cove_http::ProxyAddress;
use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::MutexGuard,
};

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
    address: SocketAddr,
    targets: Arc<Mutex<Vec<ProxiedTarget>>>,
}

impl RecordingSocksProxy {
    /// Starts a proxy that resolves [`PROXY_ONLY_HOST`] to loopback
    pub(crate) fn start() -> Self {
        let names =
            HashMap::from([(PROXY_ONLY_HOST.to_string(), IpAddr::from(Ipv4Addr::LOCALHOST))]);
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("proxy binds");
        listener.set_nonblocking(true).expect("proxy listener is non-blocking");

        let address = listener.local_addr().expect("proxy has an address");
        let targets = Arc::new(Mutex::new(Vec::new()));
        let recorded = targets.clone();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("proxy runtime starts");

            runtime.block_on(serve(listener, Arc::new(names), recorded));
        });

        Self { address, targets }
    }

    pub(crate) fn proxy_address(&self) -> ProxyAddress {
        ProxyAddress::new_ip(self.address.ip(), self.address.port())
    }

    pub(crate) fn targets(&self) -> Vec<ProxiedTarget> {
        self.targets.lock().clone()
    }
}

async fn serve(
    listener: StdTcpListener,
    names: Arc<HashMap<String, IpAddr>>,
    targets: Arc<Mutex<Vec<ProxiedTarget>>>,
) {
    let listener = TcpListener::from_std(listener).expect("proxy listener joins the runtime");
    while let Ok((stream, _)) = listener.accept().await {
        let names = names.clone();
        let targets = targets.clone();
        tokio::spawn(async move {
            let _ = handle(stream, &names, &targets).await;
        });
    }
}

async fn handle(
    mut stream: TcpStream,
    names: &HashMap<String, IpAddr>,
    targets: &Mutex<Vec<ProxiedTarget>>,
) -> std::io::Result<()> {
    // greeting: accept without auth, as an external Tor proxy such as Orbot does
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await?;
    let mut methods = vec![0; greeting[1].into()];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[5, 0]).await?;

    let mut request = [0; 4];
    stream.read_exact(&mut request).await?;

    let target = match request[3] {
        1 => {
            let mut ip = [0; 4];
            stream.read_exact(&mut ip).await?;
            let port = stream.read_u16().await?;
            ProxiedTarget::Address(SocketAddr::from((ip, port)))
        }

        3 => {
            let length = stream.read_u8().await?;
            let mut name = vec![0; length.into()];
            stream.read_exact(&mut name).await?;
            let port = stream.read_u16().await?;
            ProxiedTarget::Name(String::from_utf8_lossy(&name).into_owned(), port)
        }

        _ => {
            let mut ip = [0; 16];
            stream.read_exact(&mut ip).await?;
            let port = stream.read_u16().await?;
            ProxiedTarget::Address(SocketAddr::from((ip, port)))
        }
    };

    targets.lock().push(target.clone());

    let destination = match &target {
        ProxiedTarget::Address(address) => Some(*address),
        ProxiedTarget::Name(name, port) => names.get(name).map(|ip| SocketAddr::new(*ip, *port)),
    };

    let remote = match destination {
        Some(destination) => TcpStream::connect(destination).await.ok(),
        None => None,
    };

    let Some(mut remote) = remote else {
        stream.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    };

    stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut stream, &mut remote).await?;
    Ok(())
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
