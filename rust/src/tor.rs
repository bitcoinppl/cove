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
use cove_tor::{BootstrapState, BuiltInTor};
use cove_util::ResultExt as _;
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

/// The saved mode and what Tor is doing, as the settings screen shows them
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TorState {
    /// The saved mode, `None` when the saved setting can't be read
    pub mode: Option<TorMode>,
    pub status: TorStatus,
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
    /// Published with every route, so a mode change the screen didn't ask for still reaches it
    state: watch::Sender<TorState>,
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
            state: watch::Sender::new(TorState { mode: None, status: TorStatus::Off }),
            generation: Arc::new(AtomicU64::new(0)),
        };

        service.restart();
        service
    }

    pub fn route(&self) -> Result<NetworkRoute, RouteError> {
        self.route.load().as_ref().clone()
    }

    pub fn state(&self) -> TorState {
        self.state.borrow().clone()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<TorState> {
        self.state.subscribe()
    }

    /// Saves and applies new settings
    ///
    /// Turning Tor off is refused while any network's selected node is an onion node, since
    /// that node would stop working
    pub fn set_settings(&self, settings: TorSettings) -> Result<(), SetTorError> {
        // saving and applying share the lock, so two changes in flight can't leave the saved
        // setting and the published route on different modes
        let mut applied = self.applied.lock();
        if settings == TorSettings::Off
            && let Some(node_name) = selected_onion_node()
        {
            return Err(SetTorError::OnionNodeSelected { node_name });
        }

        Database::global()
            .global_config
            .set_tor_settings(&settings)
            .map_err_str(SetTorError::Save)?;

        self.apply(&mut applied, Ok(Some(settings)));
        Ok(())
    }

    /// Restarts Tor with the saved settings, such as after the built-in client failed
    pub fn restart(&self) {
        // the read happens under the lock, so a change saved meanwhile is never overwritten by
        // the older value
        let mut applied = self.applied.lock();
        self.apply(&mut applied, read_saved_settings());
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
        self.publish(&applied, Err(RouteError::TorUnavailable("resetting".into())), TorStatus::Off);

        match std::fs::remove_dir_all(tor_data_dir()) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Replaces the running setup with `settings`, publishing its route and status
    fn apply(&self, applied: &mut Applied, settings: Result<Option<TorSettings>, String>) {
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
                self.publish(applied, Err(RouteError::TorUnavailable(message.to_string())), failed);
                return;
            }
        };

        applied.settings = Some(settings.clone());
        match settings {
            TorSettings::Off => self.publish(applied, Ok(NetworkRoute::Direct), TorStatus::Off),

            TorSettings::Proxy(address) => {
                let proxy = SocksProxy { address, credentials: None };
                self.publish(applied, Ok(NetworkRoute::Tor(proxy)), TorStatus::UsingProxy);
            }

            TorSettings::BuiltIn => match start_built_in() {
                Ok(built_in) => {
                    let proxy = built_in.proxy().clone();
                    let bootstrap = built_in.state();
                    applied.built_in = Some(built_in);

                    // the first status goes out before the forwarder starts, so a fast
                    // bootstrap's update always lands after it
                    let starting = TorStatus::Starting { percent: 0 };
                    self.publish(applied, Ok(NetworkRoute::Tor(proxy)), starting);
                    self.forward_bootstrap_state(bootstrap, generation);
                }

                Err(message) => {
                    warn!("built-in Tor failed to start: {message}");
                    let failed = TorStatus::Failed { message: message.clone() };
                    self.publish(applied, Err(RouteError::TorUnavailable(message)), failed);
                }
            },
        }
    }

    fn publish(
        &self,
        applied: &Applied,
        route: Result<NetworkRoute, RouteError>,
        status: TorStatus,
    ) {
        self.route.store(Arc::new(route));

        let mode = applied.settings.as_ref().map(TorMode::from);
        self.state.send_replace(TorState { mode, status });
    }

    fn forward_bootstrap_state(&self, mut state: watch::Receiver<BootstrapState>, generation: u64) {
        let Some(runtime) = cove_tokio::handle() else { return };

        let screen = self.state.clone();
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

                // checked under the state's lock, so a newer mode published meanwhile always wins
                let is_current = screen.send_if_modified(|screen| {
                    let is_current = current_generation.load(Ordering::Acquire) == generation;
                    if is_current {
                        screen.status = next;
                    }

                    is_current
                });

                if !is_current {
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

    let built_in =
        BuiltInTor::start(&tor_data_dir(), &runtime).map_err(|error| error.to_string())?;
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
