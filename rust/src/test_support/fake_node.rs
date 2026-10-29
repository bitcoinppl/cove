//! In-process fake Esplora and Electrum nodes for tests
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

mod chain;
mod control;
mod electrum;
mod esplora;

use std::{
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use bitcoin::{Network, ScriptBuf};
use tokio::{net::TcpListener, sync::watch};

use crate::{database::Database, network::Network as CoveNetwork, node::Node};

pub(crate) use chain::{FakeChain, TxLocation};
pub(crate) use control::{LoggedRequest, Protocol, RuleBuilder};

use control::Control;

#[cfg(test)]
mod tests;

const WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// State both servers read from
pub(crate) struct Shared {
    chain: FakeChain,
    control: Control,
}

struct Server {
    address: SocketAddr,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start<F, Fut>(shared: Arc<Shared>, serve: F) -> Self
    where
        F: FnOnce(TcpListener, Arc<Shared>, watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = ()>,
    {
        // bind before spawning so the address is known and connectable at once
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("fake node binds a port");
        listener.set_nonblocking(true).expect("listener becomes non-blocking");
        let address = listener.local_addr().expect("listener has an address");

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

        Self { address, shutdown, thread: Some(thread) }
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
pub(crate) struct FakeNode {
    shared: Arc<Shared>,
    esplora: Server,
    electrum: Server,
}

impl FakeNode {
    /// Starts both servers on a chain that holds only the genesis block of `network`
    pub(crate) fn start(network: Network) -> Self {
        let shared =
            Arc::new(Shared { chain: FakeChain::new(network), control: Control::default() });
        let esplora = Server::start(shared.clone(), esplora::serve);
        let electrum = Server::start(shared.clone(), electrum::serve);

        Self { shared, esplora, electrum }
    }

    pub(crate) fn chain(&self) -> &FakeChain {
        &self.shared.chain
    }

    pub(crate) fn esplora_url(&self) -> String {
        format!("http://{}", self.esplora.address)
    }

    pub(crate) fn electrum_url(&self) -> String {
        format!("tcp://{}", self.electrum.address)
    }

    pub(crate) fn esplora_node(&self, name: &str) -> Node {
        Node::new_esplora(name.to_string(), self.esplora_url(), self.cove_network())
    }

    pub(crate) fn electrum_node(&self, name: &str) -> Node {
        Node::new_electrum(name.to_string(), self.electrum_url(), self.cove_network())
    }

    /// Makes the wallet code connect to this node's Esplora server
    pub(crate) fn select_esplora(&self) -> Node {
        self.select(self.esplora_node("fake esplora"))
    }

    /// Makes the wallet code connect to this node's Electrum server
    pub(crate) fn select_electrum(&self) -> Node {
        self.select(self.electrum_node("fake electrum"))
    }

    fn select(&self, node: Node) -> Node {
        Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("the fake node can be selected");

        node
    }

    fn cove_network(&self) -> CoveNetwork {
        CoveNetwork::try_from(self.shared.chain.network()).expect("fake node network is supported")
    }

    /// Scripts requests whose Esplora path or Electrum method contains `pattern`
    pub(crate) fn on(&self, pattern: &str) -> RuleBuilder<'_> {
        RuleBuilder { control: &self.shared.control, pattern: pattern.to_string() }
    }

    /// Removes every scripted rule so later requests are served normally
    pub(crate) fn clear_rules(&self) {
        self.shared.control.clear_rules();
    }

    /// Stops both servers; new connections are refused and open ones are closed
    pub(crate) fn go_offline(&mut self) {
        self.esplora.stop();
        self.electrum.stop();
    }

    pub(crate) fn requests(&self) -> Vec<LoggedRequest> {
        self.shared.control.requests()
    }

    pub(crate) fn clear_requests(&self) {
        self.shared.control.clear_requests();
    }

    /// Requests that asked about the history of `script` on either protocol
    pub(crate) fn history_requests_for(&self, script: &ScriptBuf) -> Vec<LoggedRequest> {
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
pub(crate) fn unreachable_url() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("a port can be bound");
    let address = listener.local_addr().expect("listener has an address");
    drop(listener);

    format!("http://{address}")
}

/// Polls `condition` on real time until it holds
///
/// Real time rather than tokio timers, so it keeps working when the test
/// paused the tokio clock
pub(crate) async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT_TIMEOUT;

    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");

        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(2));
    }
}
