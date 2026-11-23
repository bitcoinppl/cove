//! The Tor client that runs inside the app, built on Arti

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use arti_client::{
    BootstrapBehavior, ErrorKind, HasKind as _, IsolationToken, StreamPrefs, TorClient,
    config::{BoolOrAuto, TorClientConfigBuilder},
};
use cove_http::SocksProxy;
use futures::StreamExt as _;
use parking_lot::Mutex;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tor_rtcompat::PreferredRuntime;
use tracing::{info, warn};

use crate::{
    bridge::{BoundBridge, Connector},
    socks::{Reply, Target, TargetHost},
};

/// Waits between bootstrap attempts after one fails, such as while the device is offline
const BOOTSTRAP_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// Where the built-in client keeps its files
#[derive(Debug, Clone)]
pub struct BuiltInTorConfig {
    /// Guard choices and other state Tor keeps between launches
    pub state_dir: PathBuf,
    /// The downloaded network directory, which makes later starts fast
    pub cache_dir: PathBuf,
}

/// How far the built-in client is from carrying traffic
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapState {
    /// Joining the Tor network; streams wait until it is done
    Starting { percent: u8 },
    /// Ready to carry traffic
    Ready,
    /// The last attempt failed, and another starts after a delay
    Failed { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("could not open the local Tor proxy: {0}")]
    Bind(#[from] std::io::Error),

    #[error("could not set up Tor: {0}")]
    Client(String),
}

/// A running built-in Tor client behind its local SOCKS5 proxy
///
/// Dropping it stops the client and closes every stream through it
#[derive(Debug)]
pub struct BuiltInTor {
    proxy: SocksProxy,
    state: watch::Receiver<BootstrapState>,
    cancel: CancellationToken,
}

impl BuiltInTor {
    /// Binds the local proxy and starts joining the Tor network in the background
    ///
    /// The proxy is usable as soon as this returns: streams opened before the client is ready
    /// wait for it, and fail rather than leave Tor if it never gets there
    pub fn start(
        config: BuiltInTorConfig,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, StartError> {
        let bridge = BoundBridge::bind()?;
        let proxy = bridge.proxy().clone();

        let client = {
            let _entered = runtime.enter();
            create_client(&config)?
        };

        let (state_sender, state) = watch::channel(BootstrapState::Starting { percent: 0 });
        let cancel = CancellationToken::new();

        runtime.spawn(watch_bootstrap(client.clone(), state_sender, cancel.clone()));

        let connector = Arc::new(ArtiConnector { client, isolation: Mutex::default() });
        runtime.spawn(bridge.serve(connector, cancel.clone()));

        Ok(Self { proxy, state, cancel })
    }

    /// The local proxy every client on the Tor route connects through
    pub const fn proxy(&self) -> &SocksProxy {
        &self.proxy
    }

    /// Bootstrap progress, updated as it changes
    pub fn state(&self) -> watch::Receiver<BootstrapState> {
        self.state.clone()
    }
}

impl Drop for BuiltInTor {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn create_client(
    config: &BuiltInTorConfig,
) -> Result<Arc<TorClient<PreferredRuntime>>, StartError> {
    for dir in [&config.state_dir, &config.cache_dir] {
        std::fs::create_dir_all(dir)?;
    }

    let mut builder =
        TorClientConfigBuilder::from_directories(&config.state_dir, &config.cache_dir);

    // Arti refuses directories that other users could write to, checking every parent. The app
    // sandbox already keeps other apps out of these, and on Android and iOS the parents belong
    // to system users Arti doesn't know to trust
    builder.storage().permissions().dangerously_trust_everyone();

    let tor_config = builder.build().map_err(|error| StartError::Client(error.to_string()))?;

    TorClient::builder()
        .config(tor_config)
        .bootstrap_behavior(BootstrapBehavior::OnDemand)
        .create_unbootstrapped()
        .map_err(|error| StartError::Client(error.to_string()))
}

async fn watch_bootstrap(
    client: Arc<TorClient<PreferredRuntime>>,
    state: watch::Sender<BootstrapState>,
    cancel: CancellationToken,
) {
    let mut events = client.bootstrap_events();
    let progress = async {
        while let Some(status) = events.next().await {
            if status.ready_for_traffic() {
                state.send_replace(BootstrapState::Ready);
                continue;
            }

            // a failed attempt stays on screen until the next one makes progress
            if matches!(*state.borrow(), BootstrapState::Failed { .. }) && status.as_frac() == 0.0 {
                continue;
            }

            let percent = (status.as_frac().clamp(0.0, 1.0) * 100.0) as u8;
            state.send_replace(BootstrapState::Starting { percent: percent.min(99) });
        }
    };

    let attempts = async {
        let mut failures = 0;
        loop {
            match client.bootstrap().await {
                Ok(()) => {
                    info!("built-in Tor is ready");
                    return;
                }

                Err(error) => {
                    warn!("built-in Tor bootstrap failed: {error}");
                    state.send_replace(BootstrapState::Failed { message: error.to_string() });

                    let delay =
                        BOOTSTRAP_RETRY_DELAYS[failures.min(BOOTSTRAP_RETRY_DELAYS.len() - 1)];
                    failures += 1;
                    tokio::time::sleep(delay).await;
                }
            }
        }
    };

    tokio::select! {
        () = cancel.cancelled() => {}
        () = async { tokio::join!(progress, attempts); } => {}
    }
}

/// Opens streams through Arti, one circuit family per destination host
///
/// Without isolation, streams to the node and to the price server could share a circuit, and
/// its exit relay could tie them to one user
struct ArtiConnector {
    client: Arc<TorClient<PreferredRuntime>>,
    isolation: Mutex<HashMap<String, IsolationToken>>,
}

impl ArtiConnector {
    fn isolation_for(&self, host: &str) -> IsolationToken {
        *self.isolation.lock().entry(host.to_ascii_lowercase()).or_insert_with(IsolationToken::new)
    }
}

impl Connector for ArtiConnector {
    type Stream = arti_client::DataStream;

    async fn connect(&self, target: Target) -> Result<Self::Stream, Reply> {
        let host = match &target.host {
            TargetHost::Name(name) => name.clone(),
            TargetHost::Ip(ip) => ip.to_string(),
        };

        let mut prefs = StreamPrefs::new();
        prefs.connect_to_onion_services(BoolOrAuto::Explicit(true));
        prefs.set_isolation(self.isolation_for(&host));

        self.client.connect_with_prefs((host.as_str(), target.port), &prefs).await.map_err(
            |error| {
                warn!("Tor stream failed: {}", error.kind());
                reply_for(error.kind())
            },
        )
    }
}

fn reply_for(kind: ErrorKind) -> Reply {
    match kind {
        ErrorKind::ForbiddenStreamTarget | ErrorKind::InvalidStreamTarget => Reply::NotAllowed,
        ErrorKind::RemoteConnectionRefused => Reply::ConnectionRefused,
        ErrorKind::RemoteHostNotFound | ErrorKind::OnionServiceNotFound => Reply::HostUnreachable,
        ErrorKind::RemoteNetworkTimeout | ErrorKind::TorNetworkTimeout => Reply::TimedOut,
        _ => Reply::GeneralFailure,
    }
}
