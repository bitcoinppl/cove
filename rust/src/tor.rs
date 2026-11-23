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

    /// The client was built for a route that's no longer current, such as before Tor turned on
    #[error("the Tor setting changed, so this connection was stopped")]
    Changed,
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

/// What the service last published, read by every client builder and the settings screen
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorState {
    /// `None` when the saved settings can't be read
    pub settings: Option<TorSettings>,
    pub route: Result<NetworkRoute, RouteError>,
    pub status: TorStatus,
}

#[derive(Debug)]
pub struct TorService {
    state: watch::Sender<TorState>,
    /// The running built-in client. Every change holds this lock from saving to publishing, so
    /// two quick changes can't save in one order and apply in the other
    built_in: Mutex<Option<BuiltInTor>>,
    /// Bumped on every apply, so a stopped client's late status updates are dropped
    generation: Arc<AtomicU64>,
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
        let loading = TorState {
            settings: None,
            route: Err(RouteError::TorUnavailable("loading".into())),
            status: TorStatus::Off,
        };

        let service = Self {
            state: watch::Sender::new(loading),
            built_in: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
        };

        service.apply(&mut service.built_in.lock(), read_saved_settings());
        service
    }

    pub fn route(&self) -> Result<NetworkRoute, RouteError> {
        self.state.borrow().route.clone()
    }

    /// Every published change to the settings, route, or status
    pub fn subscribe(&self) -> watch::Receiver<TorState> {
        self.state.subscribe()
    }

    /// Saves and applies new settings
    ///
    /// Turning Tor off is refused while any network's selected node is an onion node, since
    /// that node would stop working; saving the mode that's already applied changes nothing
    pub fn set_settings(&self, settings: TorSettings) -> Result<(), SetTorError> {
        let mut built_in = self.built_in.lock();

        // checked under the change lock, so it compares with the mode the last change applied;
        // choosing the running mode again keeps built-in Tor running instead of restarting it
        if self.state.borrow().settings.as_ref() == Some(&settings) {
            return Ok(());
        }

        if settings == TorSettings::Off
            && let Some(node_name) = selected_onion_node()
        {
            return Err(SetTorError::OnionNodeSelected { node_name });
        }

        Database::global()
            .global_config
            .set_tor_settings(&settings)
            .map_err(|error| SetTorError::Save(error.to_string()))?;

        self.apply(&mut built_in, Ok(Some(settings)));
        Ok(())
    }

    /// Restarts Tor with the saved settings, such as after the built-in client failed
    pub fn restart(&self) {
        let mut built_in = self.built_in.lock();
        self.apply(&mut built_in, read_saved_settings());
    }

    /// Stops Tor and removes its files ahead of a full wipe
    ///
    /// Connections stay blocked until [`Self::restart`] reads the wiped database, so a wipe that
    /// fails partway never falls back to a direct route while Tor is still saved as on
    pub fn stop_for_wipe(&self) -> std::io::Result<()> {
        let mut built_in = self.built_in.lock();
        self.generation.fetch_add(1, Ordering::AcqRel);
        *built_in = None;
        self.publish(None, Err(RouteError::TorUnavailable("resetting".into())), TorStatus::Off);

        match std::fs::remove_dir_all(tor_data_dir()) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Replaces the running setup with `settings`, publishing its route and status
    ///
    /// The caller holds the change lock, whose guard is `built_in`
    fn apply(
        &self,
        built_in: &mut Option<BuiltInTor>,
        settings: Result<Option<TorSettings>, String>,
    ) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;

        // the old client stops before the new route goes out, so no stream outlives its mode
        *built_in = None;

        let settings = match settings {
            Ok(settings) => settings.unwrap_or(TorSettings::Off),
            Err(error) => {
                warn!("Tor settings could not be read, blocking connections: {error}");

                let message = "Cove couldn't read your Tor settings. Choose a Tor mode again";
                let failed = TorStatus::Failed { message: message.to_string() };
                self.publish(None, Err(RouteError::TorUnavailable(message.to_string())), failed);
                return;
            }
        };

        let saved = Some(settings.clone());
        match settings {
            TorSettings::Off => self.publish(saved, Ok(NetworkRoute::Direct), TorStatus::Off),

            TorSettings::Proxy(address) => {
                let proxy = SocksProxy { address, credentials: None };
                self.publish(saved, Ok(NetworkRoute::Tor(proxy)), TorStatus::UsingProxy);
            }

            TorSettings::BuiltIn => match start_built_in() {
                Ok(started) => {
                    let route = Ok(NetworkRoute::Tor(started.proxy().clone()));
                    self.publish(saved, route, TorStatus::Starting { percent: 0 });
                    self.forward_bootstrap_state(&started, generation);
                    *built_in = Some(started);
                }

                Err(message) => {
                    warn!("built-in Tor failed to start: {message}");
                    let failed = TorStatus::Failed { message: message.clone() };
                    self.publish(saved, Err(RouteError::TorUnavailable(message)), failed);
                }
            },
        }
    }

    fn publish(
        &self,
        settings: Option<TorSettings>,
        route: Result<NetworkRoute, RouteError>,
        status: TorStatus,
    ) {
        self.state.send_replace(TorState { settings, route, status });
    }

    fn forward_bootstrap_state(&self, built_in: &BuiltInTor, generation: u64) {
        let Some(runtime) = cove_tokio::handle() else { return };

        let mut state = built_in.state();
        let published = self.state.clone();
        let current_generation = self.generation.clone();

        // the first pass reads the state the client already reached
        state.mark_changed();

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

                // checked under the channel's lock, so a newer apply's status is never overwritten
                let updated = published.send_if_modified(|published| {
                    if current_generation.load(Ordering::Acquire) != generation {
                        return false;
                    }

                    published.status = next;
                    true
                });

                if !updated {
                    return;
                }
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
