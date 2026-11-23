//! Tor settings and the route every outbound connection takes
//!
//! [`TorService`] is the process-wide owner. It loads the saved [`TorSettings`], runs the
//! built-in Tor client when that mode is on, and publishes the [`NetworkRoute`] that every
//! client builder reads through [`current_route`] or [`route_for_host`]
//!
//! The invariant: while the saved setting is anything but [`TorSettings::Off`], no route
//! resolves to [`NetworkRoute::Direct`]. When Tor can't carry traffic, such as when the
//! built-in client failed to start or the settings can't be read, resolution fails and the
//! connection never starts. Connections fail instead of leaving Tor

mod settings;
#[cfg(test)]
mod tests;

use std::{
    path::PathBuf,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use arc_swap::ArcSwap;
use cove_http::{NetworkRoute, SocksProxy, route::is_onion_host};
use cove_tor::{BootstrapState, BuiltInTor, BuiltInTorConfig};
use parking_lot::Mutex;
use strum::IntoEnumIterator as _;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{database::Database, network::Network};

pub use settings::{TorMode, TorSettings, TorSettingsError};

static TOR: LazyLock<TorService> = LazyLock::new(TorService::load);

/// The route for a new connection to a fixed, well-known host
pub fn current_route() -> Result<NetworkRoute, RouteError> {
    TOR.route()
}

/// The route for a new connection to `host`, refusing an onion host when Tor is off
pub fn route_for_host(host: &str) -> Result<NetworkRoute, RouteError> {
    let route = current_route()?;
    if !route.is_tor() && is_onion_host(host) {
        return Err(RouteError::OnionNeedsTor);
    }

    Ok(route)
}

/// Why no route is available
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RouteError {
    /// Tor is on but can't carry traffic, so the connection doesn't start
    #[error("Tor is on but not connected: {0}")]
    TorUnavailable(String),

    /// An onion address needs Tor, and resolving it with the system resolver would leak it
    #[error("turn on Tor in Settings to connect to .onion addresses")]
    OnionNeedsTor,
}

/// What Tor is doing, for the settings screen
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TorStatus {
    Off,

    /// The built-in client is joining the Tor network, and connections wait for it
    Starting {
        percent: u8,
    },

    /// The built-in client is carrying traffic
    Ready,

    /// Tor is on but can't carry traffic, so nothing connects until this clears
    Failed {
        message: String,
    },

    /// Traffic goes to the user's SOCKS proxy, whose health only shows when a connection runs
    UsingProxy,
}

/// Why a Tor setting change was refused
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum SetTorError {
    #[error("{node_name} is a .onion node, which only works over Tor. Choose another node first")]
    OnionNodeSelected { node_name: String },

    #[error("could not save the Tor setting: {0}")]
    Save(String),
}

#[derive(Debug)]
pub struct TorService {
    route: ArcSwap<Result<NetworkRoute, RouteError>>,
    applied: Mutex<Applied>,
    status: watch::Sender<TorStatus>,
    /// Bumped on every apply, so a stopped client's late status updates are dropped
    generation: Arc<AtomicU64>,
}

#[derive(Debug)]
struct Applied {
    /// `None` when the saved settings can't be read
    settings: Option<TorSettings>,
    built_in: Option<BuiltInTor>,
}

impl TorService {
    pub fn global() -> &'static Self {
        &TOR
    }

    /// Loads the saved settings and starts Tor if it's on, so it starts joining the network at
    /// launch instead of on the first connection
    pub fn start() {
        LazyLock::force(&TOR);
    }

    fn load() -> Self {
        let service = Self {
            route: ArcSwap::from_pointee(Err(RouteError::TorUnavailable("loading".into()))),
            applied: Mutex::new(Applied { settings: None, built_in: None }),
            status: watch::Sender::new(TorStatus::Off),
            generation: Arc::new(AtomicU64::new(0)),
        };

        service.apply(read_saved_settings());
        service
    }

    pub fn route(&self) -> Result<NetworkRoute, RouteError> {
        self.route.load().as_ref().clone()
    }

    /// The saved settings, or `None` when they can't be read
    pub fn settings(&self) -> Option<TorSettings> {
        self.applied.lock().settings.clone()
    }

    pub fn status(&self) -> TorStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<TorStatus> {
        self.status.subscribe()
    }

    /// Saves and applies new settings
    ///
    /// Turning Tor off is refused while any network's selected node is an onion node, since
    /// that node would stop working
    pub fn set_settings(&self, settings: TorSettings) -> Result<(), SetTorError> {
        if settings == TorSettings::Off
            && let Some(node_name) = selected_onion_node()
        {
            return Err(SetTorError::OnionNodeSelected { node_name });
        }

        Database::global()
            .global_config
            .set_tor_settings(&settings)
            .map_err(|error| SetTorError::Save(error.to_string()))?;

        self.apply(Ok(Some(settings)));
        Ok(())
    }

    /// Restarts Tor with the saved settings, such as after the built-in client failed
    pub fn restart(&self) {
        self.apply(read_saved_settings());
    }

    /// Stops Tor and removes its files ahead of a full wipe
    ///
    /// Connections stay blocked until [`Self::restart`] reads the wiped database, so a wipe that
    /// fails partway never falls back to a direct route while Tor is still saved as on
    pub fn stop_for_wipe(&self) -> std::io::Result<()> {
        let mut applied = self.applied.lock();
        self.generation.fetch_add(1, Ordering::AcqRel);
        applied.built_in = None;
        applied.settings = None;
        self.publish(Err(RouteError::TorUnavailable("resetting".into())), TorStatus::Off);

        match std::fs::remove_dir_all(tor_data_dir()) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Replaces the running setup with `settings`, publishing its route and status
    fn apply(&self, settings: Result<Option<TorSettings>, String>) {
        let mut applied = self.applied.lock();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;

        // the old client stops before the new route goes out, so no stream outlives its mode
        applied.built_in = None;

        let settings = match settings {
            Ok(settings) => settings.unwrap_or(TorSettings::Off),
            Err(error) => {
                warn!("Tor settings could not be read, blocking connections: {error}");
                applied.settings = None;

                let message = "Cove couldn't read your Tor settings. Choose a Tor mode again";
                let failed = TorStatus::Failed { message: message.to_string() };
                self.publish(Err(RouteError::TorUnavailable(message.to_string())), failed);
                return;
            }
        };

        applied.settings = Some(settings.clone());
        match settings {
            TorSettings::Off => self.publish(Ok(NetworkRoute::Direct), TorStatus::Off),

            TorSettings::Proxy(address) => {
                let proxy = SocksProxy { address, credentials: None };
                self.publish(Ok(NetworkRoute::Tor(proxy)), TorStatus::UsingProxy);
            }

            TorSettings::BuiltIn => match start_built_in() {
                Ok(built_in) => {
                    let proxy = built_in.proxy().clone();
                    self.forward_bootstrap_state(&built_in, generation);
                    applied.built_in = Some(built_in);
                    self.publish(Ok(NetworkRoute::Tor(proxy)), TorStatus::Starting { percent: 0 });
                }

                Err(message) => {
                    warn!("built-in Tor failed to start: {message}");
                    let failed = TorStatus::Failed { message: message.clone() };
                    self.publish(Err(RouteError::TorUnavailable(message)), failed);
                }
            },
        }
    }

    fn publish(&self, route: Result<NetworkRoute, RouteError>, status: TorStatus) {
        self.route.store(Arc::new(route));
        self.status.send_replace(status);
    }

    fn forward_bootstrap_state(&self, built_in: &BuiltInTor, generation: u64) {
        let Some(runtime) = cove_tokio::handle() else { return };

        let mut state = built_in.state();
        let status = self.status.clone();
        let current_generation = self.generation.clone();

        runtime.spawn(async move {
            // ends when the client stops and drops its sender
            while state.changed().await.is_ok() {
                let next = match &*state.borrow_and_update() {
                    BootstrapState::Starting { percent } => {
                        TorStatus::Starting { percent: *percent }
                    }
                    BootstrapState::Ready => TorStatus::Ready,
                    BootstrapState::Failed { message } => {
                        TorStatus::Failed { message: message.clone() }
                    }
                };

                if current_generation.load(Ordering::Acquire) != generation {
                    return;
                }

                status.send_replace(next);
            }
        });
    }
}

fn read_saved_settings() -> Result<Option<TorSettings>, String> {
    Database::global().global_config.tor_settings().map_err(|error| error.to_string())
}

fn start_built_in() -> Result<BuiltInTor, String> {
    let runtime = cove_tokio::handle().ok_or("the app runtime isn't running yet")?;

    let dir = tor_data_dir();
    let config = BuiltInTorConfig { state_dir: dir.join("state"), cache_dir: dir.join("cache") };

    let built_in = BuiltInTor::start(config, &runtime).map_err(|error| error.to_string())?;
    info!("built-in Tor started");

    Ok(built_in)
}

fn tor_data_dir() -> PathBuf {
    cove_common::consts::ROOT_DATA_DIR.join("tor")
}

/// The name of the first selected node, across every network, that only Tor can reach
fn selected_onion_node() -> Option<String> {
    let config = &Database::global().global_config;
    Network::iter().map(|network| config.selected_node_for_network(network)).find_map(|node| {
        let url = url::Url::parse(&node.url).ok()?;
        is_onion_host(url.host_str()?).then_some(node.name)
    })
}
