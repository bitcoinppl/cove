//! Fake Esplora and Electrum nodes for tests
//!
//! Both servers answer from one shared [`FakeChain`], so a test can mutate the
//! tip or a transaction's status once and see both protocols agree. Each server
//! runs on its own thread with its own runtime: it keeps answering while the
//! test's runtime is paused or blocked, and tearing the node down closes every
//! socket, including stalled ones
//!
//! Requests are logged and can be scripted per test (see [`FakeNode::on`]) to
//! fail, stall, run slowly or wait on a [`Gate`]. Going offline drops the
//! listeners so new connections are refused
//!
//! Tests in another process, such as the iOS and Android UI tests, drive the
//! node through the JSON API that [`FakeNode::serve_control`] serves; the
//! `cove-fake-node` binary runs a node with that API on fixed ports

mod chain;
mod control;
mod control_api;
mod electrum;
mod esplora;
mod http;

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use bitcoin::{Network, ScriptBuf};
use tokio::{net::TcpListener, sync::watch};

pub use chain::{
    AlreadyConfirmed, Balance, ChainTx, FakeChain, HistoryEntry, MerkleProof, TxLocation,
};
pub use control::{Gate, LoggedRequest, Protocol, RuleBuilder, RuleHandle};
pub use control_api::{ControlServer, RESET_BLOCKS};

use control::Control;

#[cfg(test)]
mod tests;

const WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// State every server reads from
pub(crate) struct Shared {
    chain: FakeChain,
    control: Control,
}

impl Shared {
    /// Starts over on a fresh chain of [`RESET_BLOCKS`] empty blocks with no
    /// logged requests and no scripted rules
    fn reset(&self) {
        self.chain.reset();
        self.chain.mine(RESET_BLOCKS);
        self.control.clear_requests();
        self.control.clear_rules();
    }
}

struct Server {
    address: SocketAddr,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start<F, Fut>(address: SocketAddr, shared: Arc<Shared>, serve: F) -> io::Result<Self>
    where
        F: FnOnce(TcpListener, Arc<Shared>, watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = ()>,
    {
        // bind before spawning so the address is known and connectable at once
        let listener = StdTcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;

        let (shutdown, shutdown_rx) = watch::channel(false);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("fake node runtime builds");

            runtime.block_on(async {
                let listener = TcpListener::from_std(listener).expect("listener registers");
                serve(listener, shared, shutdown_rx).await;
            });
        });

        Ok(Self { address, shutdown, thread: Some(thread) })
    }

    fn stop(&mut self) {
        self.shutdown.send_replace(true);

        if let Some(thread) = self.thread.take() {
            thread.join().expect("fake node server thread does not panic");
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A fake Esplora and Electrum server pair backed by one chain
pub struct FakeNode {
    shared: Arc<Shared>,
    esplora: Server,
    electrum: Server,
}

impl FakeNode {
    /// Starts both servers on free loopback ports, on a chain that holds only
    /// the genesis block of `network`
    pub fn start(network: Network) -> Self {
        let any_port = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));

        Self::start_on(network, any_port, any_port).expect("fake node binds free loopback ports")
    }

    /// Starts both servers on the given addresses, on a chain that holds only
    /// the genesis block of `network`
    ///
    /// Port 0 picks a free port. Fails when an address is already in use
    pub fn start_on(
        network: Network,
        esplora_address: SocketAddr,
        electrum_address: SocketAddr,
    ) -> io::Result<Self> {
        let shared =
            Arc::new(Shared { chain: FakeChain::new(network), control: Control::default() });
        let esplora = Server::start(esplora_address, shared.clone(), esplora::serve)?;
        let electrum = Server::start(electrum_address, shared.clone(), electrum::serve)?;

        Ok(Self { shared, esplora, electrum })
    }

    /// Serves the JSON control API for this node on `address`
    ///
    /// The API keeps working after the node goes offline, and stops when the
    /// returned server drops
    pub fn serve_control(&self, address: SocketAddr) -> io::Result<ControlServer> {
        ControlServer::start(address, self.shared.clone(), self.esplora_url(), self.electrum_url())
    }

    /// The chain both servers answer from
    pub fn chain(&self) -> &FakeChain {
        &self.shared.chain
    }

    /// The Esplora base URL, such as `http://127.0.0.1:39001`
    pub fn esplora_url(&self) -> String {
        format!("http://{}", self.esplora.address)
    }

    /// The Electrum URL, such as `tcp://127.0.0.1:39002`
    pub fn electrum_url(&self) -> String {
        format!("tcp://{}", self.electrum.address)
    }

    /// Replaces the chain with genesis plus [`RESET_BLOCKS`] empty blocks and
    /// clears the request log, the broadcast log and every scripted rule
    pub fn reset(&self) {
        self.shared.reset();
    }

    /// Scripts requests whose Esplora path or Electrum method contains `pattern`
    pub fn on(&self, pattern: &str) -> RuleBuilder<'_> {
        RuleBuilder { control: &self.shared.control, pattern: pattern.to_string() }
    }

    /// Removes every scripted rule so later requests are served normally
    pub fn clear_rules(&self) {
        self.shared.control.clear_rules();
    }

    /// Stops both servers; new connections are refused and open ones are closed
    pub fn go_offline(&mut self) {
        self.esplora.stop();
        self.electrum.stop();
    }

    /// Every request either server received, oldest first
    pub fn requests(&self) -> Vec<LoggedRequest> {
        self.shared.control.requests()
    }

    /// Empties the request log
    pub fn clear_requests(&self) {
        self.shared.control.clear_requests();
    }

    /// Requests that asked about the history of `script` on either protocol
    pub fn history_requests_for(&self, script: &ScriptBuf) -> Vec<LoggedRequest> {
        use bitcoin::hashes::{Hash as _, sha256};

        let digest = sha256::Hash::hash(script.as_bytes()).to_byte_array();
        let forward = hex::encode(digest);
        let reversed = hex::encode(digest.iter().rev().copied().collect::<Vec<_>>());

        self.requests()
            .into_iter()
            .filter(|request| match request.protocol {
                Protocol::Esplora => request.name.starts_with(&format!("/scripthash/{forward}/")),
                Protocol::Electrum => {
                    request.name == "blockchain.scripthash.get_history"
                        && request.detail.contains(&reversed)
                }
            })
            .collect()
    }
}

/// Returns a URL on which nothing listens, so connecting is refused
pub fn unreachable_url() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("a port can be bound");
    let address = listener.local_addr().expect("listener has an address");
    drop(listener);

    format!("http://{address}")
}

/// Polls `condition` on real time until it holds
///
/// Real time rather than tokio timers, so it keeps working when the test
/// paused the tokio clock
pub async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT_TIMEOUT;

    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");

        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(2));
    }
}
