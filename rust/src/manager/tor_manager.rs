use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream, ToSocketAddrs as _},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use cove_http::{HttpTrafficCategory, Socks5Proxy, TorTrafficCategory};
use parking_lot::Mutex;
use strum::IntoEnumIterator as _;
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::AbortHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    database::{Database, global_config::GlobalConfigTable},
    manager::{deferred_sender::SingleOrMany, reconcile_channel::ReconcileChannel},
    network::Network,
    node::client::NodeClientOptions,
    node_connect::switch_onion_selections_to_clearnet_presets,
    tor::{TorConfig, TorLaunchHealth},
    tor_runtime::{self, BootstrapBlockage, EndpointWait, Status},
};

type Message = TorManagerReconcileMessage;
type Result<T, E = TorError> = std::result::Result<T, E>;

const PROXY_TEST_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Default bound on waiting for the configured HTTP route to become usable
pub(crate) const HTTP_ROUTE_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a failed built-in start suppresses another start attempt
const START_RETRY_BACKOFF: Duration = Duration::from_secs(30);

/// How long a built-in runtime must survive before it counts as a healthy launch
///
/// The breaker exists for one failure: Arti calls `std::process::exit(1)` five seconds
/// after the consensus says this build is too old to use. A launch that is still alive
/// well past that is not in that crash loop, and a cold bootstrap on a slow mobile
/// network routinely takes minutes, so waiting for readiness would trip on real users
const LAUNCH_HEALTH_GRACE: Duration = Duration::from_secs(15);

/// Loopback endpoint that no listener can bind, used to fail requests closed
static FAIL_CLOSED_PROXY: LazyLock<Socks5Proxy> =
    LazyLock::new(|| Socks5Proxy::new("127.0.0.1", 0));

/// Where HTTP requests are routed for the current Tor configuration
#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpRoute {
    /// Requests go straight out
    Direct,
    /// Requests go through a user-supplied SOCKS5 proxy, which never carries labels
    External(Socks5Proxy),
    /// Requests go through Cove's own loopback proxy, isolated per category
    BuiltIn(SocketAddr),
    /// A Tor route is configured but not usable yet; requests should wait
    PendingTor,
    /// Fail-closed: only user action can make the configured route usable
    Blocked(TorError),
}

impl HttpRoute {
    /// Derives the route a persisted configuration asks for
    fn for_config(config: &TorConfig) -> Result<Self> {
        match config {
            TorConfig::Off => Ok(Self::Direct),
            TorConfig::BuiltIn => Ok(Self::PendingTor),
            TorConfig::External { host, port } => Socks5Proxy::validated(host, *port)
                .map(Self::External)
                .map_err(|_| TorError::InvalidExternalProxy),
        }
    }

    /// Returns the SOCKS5 proxy one category's requests take, if any
    fn proxy(&self, category: HttpTrafficCategory) -> Option<Socks5Proxy> {
        match self {
            Self::Direct => None,
            // an arbitrary proxy may reject credentials it did not ask for, and Cove
            // has no reason to disclose its traffic categories to a third party
            Self::External(proxy) => Some(proxy.clone()),
            Self::BuiltIn(endpoint) => Some(Socks5Proxy::isolated(
                endpoint.ip().to_string(),
                endpoint.port(),
                TorTrafficCategory::Http(category),
            )),
            Self::PendingTor | Self::Blocked(_) => Some(FAIL_CLOSED_PROXY.clone()),
        }
    }

    /// Whether each category needs its own client because the route labels traffic
    const fn isolates_categories(&self) -> bool {
        matches!(self, Self::BuiltIn(_))
    }
}

/// Route and the per-category clients built for it, swapped as one value
#[derive(Debug)]
struct HttpClients {
    route: HttpRoute,
    general: reqwest::Client,
    payjoin: reqwest::Client,
    /// Built without redirect following: a redirected report would leak plaintext
    diagnostics: reqwest::Client,
}

impl HttpClients {
    fn client(&self, category: HttpTrafficCategory) -> reqwest::Client {
        match category {
            HttpTrafficCategory::General => self.general.clone(),
            HttpTrafficCategory::Payjoin => self.payjoin.clone(),
            HttpTrafficCategory::Diagnostics => self.diagnostics.clone(),
        }
    }
}

/// Global owner of Tor configuration, runtime lifecycle, and status reconciliation
pub static TOR_MANAGER: LazyLock<Arc<RustTorManager>> = LazyLock::new(RustTorManager::init);

/// User intent handled by the Tor manager
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorManagerAction {
    /// Enable Tor with the current route, defaulting to the built-in runtime
    Enable,
    /// Disable Tor unless the selected node requires onion routing
    Disable,
    /// Disable Tor after acknowledging the onion-node fallback warning
    DisableConfirmed,
    /// Replace the active Tor configuration
    SetConfig(TorConfig),
    /// Run the progressive proxy and selected-node connection test
    RunConnectionTest,
}

/// Result of dispatching a Tor manager action
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorManagerDispatchResult {
    /// The action was accepted
    Applied,
    /// The action requires confirmation before it can be applied
    DisableWarning(TorDisableWarning),
}

/// Warning that requires explicit confirmation before disabling Tor
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorDisableWarning {
    /// These networks have a selected node that is only reachable through onion routing
    OnionNodesSelected { networks: Vec<Network> },
}

/// Progressive Tor connection-test step
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorTestStep {
    /// Verify that the configured endpoint speaks SOCKS5
    ProxyReachable,
    /// Verify that the selected node is reachable through the configured Tor route
    NodeReachableViaTor,
}

/// Current state of one Tor connection-test step
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorTestState {
    /// The step is in progress
    Running,
    /// The step completed successfully
    Passed,
    /// The step failed with a reader-visible explanation
    Failed(String),
}

/// Typed update for a progressive Tor connection test
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TorTestUpdate {
    /// Step being updated
    pub step: TorTestStep,
    /// Current state of the step
    pub state: TorTestState,
}

/// What kind of Tor operation produced a failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorFailureOrigin {
    /// The configured Tor route itself failed to start, run, persist, or route HTTP
    Lifecycle,
    /// A user-initiated connection test failed; the route may still be healthy
    ConnectionTest,
}

/// Delta emitted by the Tor manager reconcile stream
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorManagerReconcileMessage {
    /// The persisted Tor route changed
    ConfigChanged(TorConfig),
    /// The built-in runtime reported bootstrap progress
    BootstrapProgress { percent: u8, message: String },
    /// The built-in runtime is ready for traffic
    Ready,
    /// The built-in runtime stopped
    Stopped,
    /// A Tor operation failed
    Failed { origin: TorFailureOrigin, error: TorError },
    /// Built-in Tor was not auto-started because repeated launches failed
    AutoStartSuppressed,
    /// A progressive connection-test step changed
    ConnectionTest(TorTestUpdate),
}

/// Public category for a built-in Tor runtime failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum TorRuntimeError {
    /// The runtime storage directory could not be created
    #[error("Unable to prepare Tor storage")]
    CreateDirectory,
    /// The runtime configuration could not be built
    #[error("Unable to configure Tor")]
    Configure,
    /// The dedicated runtime thread could not be started
    #[error("Unable to start the Tor runtime thread")]
    SpawnThread,
    /// The Arti-compatible runtime could not be created
    #[error("Unable to create the Tor runtime")]
    CreateRuntime,
    /// The local SOCKS listener could not be bound
    #[error("Unable to bind the Tor proxy")]
    BindListener,
    /// The local SOCKS listener address could not be read
    #[error("Unable to inspect the Tor proxy")]
    ReadListenerAddress,
    /// The Arti client could not be created
    #[error("Unable to create the Tor client")]
    CreateClient,
    /// Tor bootstrap failed
    #[error("Tor bootstrap failed")]
    Bootstrap,
    /// The local SOCKS proxy failed
    #[error("The Tor proxy failed")]
    Proxy,
    /// The local SOCKS proxy stopped unexpectedly
    #[error("The Tor proxy stopped unexpectedly")]
    ProxyStopped,
    /// The typed bootstrap status stream closed unexpectedly
    #[error("Tor bootstrap status stopped unexpectedly")]
    BootstrapStatusStreamClosed,
    /// The runtime stopped before becoming ready
    #[error("Tor stopped before it became ready")]
    StoppedBeforeReady,
    /// No manager-owned built-in runtime is active
    #[error("Tor is not running")]
    NotRunning,
    /// An internal lifecycle channel closed unexpectedly
    #[error("Tor lifecycle communication failed")]
    LifecycleChannelClosed,
    /// Tor did not finish bootstrapping inside the caller's wait bound
    #[error("Tor did not become ready in time")]
    ReadyTimeout,
}

impl From<tor_runtime::Error> for TorRuntimeError {
    fn from(error: tor_runtime::Error) -> Self {
        match error {
            tor_runtime::Error::CreateDirectory { .. } => Self::CreateDirectory,
            tor_runtime::Error::Configure(_) => Self::Configure,
            tor_runtime::Error::SpawnThread(_) => Self::SpawnThread,
            tor_runtime::Error::CreateRuntime(_) => Self::CreateRuntime,
            tor_runtime::Error::BindListener(_) => Self::BindListener,
            tor_runtime::Error::ReadListenerAddress(_) => Self::ReadListenerAddress,
            tor_runtime::Error::CreateClient(_) => Self::CreateClient,
            tor_runtime::Error::Bootstrap(_) => Self::Bootstrap,
            tor_runtime::Error::Proxy(_) => Self::Proxy,
            tor_runtime::Error::ProxyStopped => Self::ProxyStopped,
            tor_runtime::Error::BootstrapStatusStreamClosed => Self::BootstrapStatusStreamClosed,
            tor_runtime::Error::StoppedBeforeReady => Self::StoppedBeforeReady,
            tor_runtime::Error::NotRunning => Self::NotRunning,
            tor_runtime::Error::LifecycleChannelClosed => Self::LifecycleChannelClosed,
            // the platform only distinguishes "not ready in time" from other failures,
            // so both lifecycle bounds report the timeout the user can act on
            tor_runtime::Error::ReadyTimeout
            | tor_runtime::Error::StartupTimeout
            | tor_runtime::Error::ShutdownTimeout => Self::ReadyTimeout,
        }
    }
}

/// Error returned by a Tor manager lifecycle action
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum TorError {
    /// The built-in Tor runtime failed
    #[error("{0}")]
    Runtime(#[from] TorRuntimeError),
    /// The external proxy host or port is invalid
    #[error("Enter a valid SOCKS5 proxy host and port")]
    InvalidExternalProxy,
    /// A Tor configuration change could not be persisted
    #[error("Unable to save Tor settings")]
    Persistence,
    /// No clearnet preset could replace an onion node
    #[error("Unable to select a clearnet fallback node")]
    ClearnetFallback,
    /// The persisted Tor configuration could not be read safely
    #[error("Unable to read Tor settings")]
    ConfigUnreadable,
}

impl From<tor_runtime::Error> for TorError {
    fn from(error: tor_runtime::Error) -> Self {
        Self::Runtime(error.into())
    }
}

/// Who asked for a built-in Tor start
///
/// Only the crash-loop breaker cares: a user who explicitly asks for Tor is entitled
/// to a launch attempt even when Cove has stopped starting it on its own
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartIntent {
    /// The user asked for Tor, so the breaker cannot refuse the launch
    UserInitiated,
    /// Cove is starting Tor on its own behalf, so a tripped breaker refuses the launch
    Automatic,
}

/// Why a built-in Tor start did not leave a runtime running
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartRefusal {
    /// The breaker refused an automatic launch, and has already blocked the route
    /// and told the platform, so the caller must not report it a second time
    AutoStartSuppressed,
    /// A launch was attempted and failed
    Failed(TorError),
}

impl From<StartRefusal> for TorError {
    fn from(refusal: StartRefusal) -> Self {
        match refusal {
            StartRefusal::AutoStartSuppressed => Self::Runtime(TorRuntimeError::NotRunning),
            StartRefusal::Failed(error) => error,
        }
    }
}

/// Whether a requested configuration is a real route change
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigTransition {
    /// The persisted config already matches: reconcile the runtime, emit nothing
    NoOp,
    /// Stop the current route, persist, start the new route, emit ConfigChanged
    Switch { stop_runtime: bool },
}

fn config_transition(current: Option<&TorConfig>, next: &TorConfig) -> ConfigTransition {
    if current == Some(next) {
        return ConfigTransition::NoOp;
    }

    // only the built-in route owns a runtime, and an unreadable config may have left one behind
    let stop_runtime = !matches!(current, Some(TorConfig::Off | TorConfig::External { .. }));

    ConfigTransition::Switch { stop_runtime }
}

/// Rust implementation of the Tor manager singleton
#[derive(Debug, uniffi::Object)]
pub struct RustTorManager {
    reconciler: ReconcileChannel<Message>,
    operations: AsyncMutex<()>,
    http: watch::Sender<Arc<HttpClients>>,
    scan_epoch: Mutex<CancellationToken>,
    connection_test_in_flight: AtomicBool,
    swap_target_generation: AtomicU64,
    last_failed_start: Mutex<Option<(Instant, TorError)>>,
    status_forwarder: Mutex<Option<AbortHandle>>,
}

impl RustTorManager {
    fn init() -> Arc<Self> {
        let initial = Database::global()
            .global_config
            .tor_config()
            .map_err(|_| TorError::ConfigUnreadable)
            .and_then(|config| HttpRoute::for_config(&config))
            .and_then(http_clients_for_route);

        let (clients, initial_error) = match initial {
            Ok(clients) => (clients, None),
            Err(error) => (blocked_http_clients(error.clone()), Some(error)),
        };

        let (http, _) = watch::channel(Arc::new(clients));
        let manager = Arc::new(Self {
            reconciler: ReconcileChannel::new(1000),
            operations: AsyncMutex::new(()),
            http,
            scan_epoch: Mutex::new(CancellationToken::new()),
            connection_test_in_flight: AtomicBool::new(false),
            swap_target_generation: AtomicU64::new(0),
            last_failed_start: Mutex::new(None),
            status_forwarder: Mutex::new(None),
        });

        if let Some(error) = initial_error {
            manager.send(Message::Failed { origin: TorFailureOrigin::Lifecycle, error });
        }

        manager
    }

    /// Starts configured built-in Tor during application initialization
    pub(crate) async fn warmup(&self) {
        let _operation = self.operations.lock().await;
        let config = match Database::global().global_config.tor_config() {
            Ok(config) => config,
            Err(_) => {
                self.fail_lifecycle(TorError::ConfigUnreadable);
                return;
            }
        };

        // the unconditional snapshot late subscribers need, unlike the deltas apply_config emits
        self.send(Message::ConfigChanged(config.clone()));

        if config != TorConfig::BuiltIn {
            let _ = self.install_route_for_config(&config);
            return;
        }

        if let Err(StartRefusal::Failed(error)) =
            self.ensure_built_in_running(StartIntent::Automatic).await
        {
            self.fail_lifecycle(error);
        }
    }

    async fn apply_config(&self, config: TorConfig) -> Result<()> {
        // validating here normalizes the host, so a route derived from the persisted
        // config always compares equal to the route that was installed for it
        let config = match config {
            TorConfig::External { host, port } => {
                let proxy = Socks5Proxy::validated(&host, port)
                    .map_err(|_| TorError::InvalidExternalProxy)?;

                TorConfig::External { host: proxy.host().to_string(), port: proxy.port() }
            }
            unchanged => unchanged,
        };

        // a user-driven config change is the recovery path for unreadable persisted state
        let current = Database::global().global_config.tor_config().ok();

        match config_transition(current.as_ref(), &config) {
            ConfigTransition::NoOp => self.reconcile_current_config(&config).await,
            ConfigTransition::Switch { stop_runtime } => {
                self.switch_config(config, stop_runtime).await
            }
        }
    }

    /// Brings the runtime and HTTP route in line with a configuration that is already persisted
    ///
    /// Emits nothing when it succeeds: a redundant save must not reset bootstrap
    /// progress or test results. A failure is still reported, because the status it
    /// produces is the only feedback the platform gets for the save
    async fn reconcile_current_config(&self, config: &TorConfig) -> Result<()> {
        if *config == TorConfig::BuiltIn {
            return self
                .ensure_built_in_running(StartIntent::UserInitiated)
                .await
                .map_err(|refusal| self.fail_lifecycle(refusal.into()));
        }

        let route = HttpRoute::for_config(config).map_err(|error| self.fail_lifecycle(error))?;
        self.ensure_http_route(route);

        Ok(())
    }

    async fn switch_config(&self, config: TorConfig, stop_runtime: bool) -> Result<()> {
        // requests wait for the incoming route rather than taking the outgoing one
        self.install_http_route(HttpRoute::PendingTor);

        if stop_runtime && let Err(error) = tor_runtime::stop().await {
            return Err(self.fail_lifecycle(TorError::from(error)));
        }

        if Database::global().global_config.set_tor_config(config.clone()).is_err() {
            return Err(self.fail_lifecycle(TorError::Persistence));
        }

        // in-flight scans and cached node clients belong to the route that just went away
        self.rotate_scan_epoch();

        self.send(Message::ConfigChanged(config.clone()));

        if config == TorConfig::BuiltIn {
            return self
                .ensure_built_in_running(StartIntent::UserInitiated)
                .await
                .map_err(|refusal| self.fail_lifecycle(refusal.into()));
        }

        self.install_route_for_config(&config)?;
        self.send(Message::Stopped);

        Ok(())
    }

    /// Ensures the built-in runtime, its status forwarder, and its HTTP swap task are running
    ///
    /// Every start path funnels through here, so this is where the crash-loop breaker is
    /// consulted and fed. Cheap when everything is already running: the caller must hold
    /// the operations lock
    async fn ensure_built_in_running(&self, intent: StartIntent) -> Result<(), StartRefusal> {
        self.start_status_forwarder();

        // a persistently failing arti must not be respawned on every node-client construction
        let backoff = self.last_failed_start.lock().clone();
        if let Some((attempted_at, error)) = backoff
            && attempted_at.elapsed() < START_RETRY_BACKOFF
        {
            return Err(StartRefusal::Failed(error));
        }

        if starts_a_fresh_runtime() {
            self.claim_launch_attempt(intent)?;
        }

        if let Err(error) = tor_runtime::start().await {
            let error = TorError::from(error);
            *self.last_failed_start.lock() = Some((Instant::now(), error.clone()));

            return Err(StartRefusal::Failed(error));
        }
        self.last_failed_start.lock().take();

        let generation = tor_runtime::generation();
        let first_for_generation =
            self.swap_target_generation.swap(generation, Ordering::AcqRel) != generation;

        // a fresh generation has not bootstrapped yet and a blocked route means an
        // earlier swap gave up: either way requests wait instead of taking a route
        // that belongs to a runtime which is gone
        let rearmed = matches!(self.http.borrow().route, HttpRoute::Blocked(_));
        if first_for_generation || rearmed {
            self.install_http_route(HttpRoute::PendingTor);
            cove_tokio::task::spawn(swap_http_client_when_ready(generation));
        }

        if first_for_generation {
            cove_tokio::task::spawn(observe_launch_health(generation));
        }

        Ok(())
    }

    /// Consults and feeds the crash-loop breaker for one built-in launch
    fn claim_launch_attempt(&self, intent: StartIntent) -> Result<(), StartRefusal> {
        let global_config = &Database::global().global_config;
        let mut health = global_config.tor_launch_health();

        if !launch_is_allowed(intent, &health) {
            return Err(self.suppress_auto_start());
        }

        // persisted before the launch so a start that never returns still counts
        health.record_attempt();
        if let Err(error) = global_config.set_tor_launch_health(&health) {
            warn!("unable to persist Tor launch health, crash-loop detection is off: {error}");
        }

        Ok(())
    }

    /// Fails requests closed and reports one notice per transition into suppression
    fn suppress_auto_start(&self) -> StartRefusal {
        let blocked = HttpRoute::Blocked(TorError::Runtime(TorRuntimeError::NotRunning));
        let already_suppressed = self.http.borrow().route == blocked;

        // every automatic start is refused while the breaker is tripped, and the
        // platform only needs to hear about it when the route actually changes
        if !already_suppressed {
            self.install_http_route(blocked);
            self.send(Message::AutoStartSuppressed);
        }

        StartRefusal::AutoStartSuppressed
    }

    /// Clears the crash-loop breaker because the user explicitly asked for Tor
    fn clear_launch_breaker(&self) {
        let global_config = &Database::global().global_config;
        let mut health = global_config.tor_launch_health();
        if health == TorLaunchHealth::default() {
            return;
        }

        health.mark_healthy();
        if let Err(error) = global_config.set_tor_launch_health(&health) {
            warn!("unable to reset Tor launch health: {error}");
        }
    }

    fn install_route_for_config(&self, config: &TorConfig) -> Result<()> {
        match HttpRoute::for_config(config) {
            Ok(route) => {
                self.install_http_route(route);
                Ok(())
            }
            Err(error) => Err(self.fail_lifecycle(error)),
        }
    }

    fn install_http_route(&self, route: HttpRoute) {
        let clients = match http_clients_for_route(route) {
            Ok(clients) => clients,
            Err(error) => {
                self.send(Message::Failed {
                    origin: TorFailureOrigin::Lifecycle,
                    error: error.clone(),
                });

                blocked_http_clients(error)
            }
        };

        self.http.send_replace(Arc::new(clients));
    }

    /// Installs a route only when it differs, so an unchanged route keeps its connection pool
    fn ensure_http_route(&self, route: HttpRoute) {
        if self.http.borrow().route == route {
            return;
        }

        self.install_http_route(route);
    }

    fn block_http_requests(&self, error: TorError) {
        self.install_http_route(HttpRoute::Blocked(error));
    }

    /// Fails requests closed and reports one lifecycle failure, returning it for propagation
    fn fail_lifecycle(&self, error: TorError) -> TorError {
        self.block_http_requests(error.clone());
        self.send(Message::Failed { origin: TorFailureOrigin::Lifecycle, error: error.clone() });

        error
    }

    async fn clients(&self, bound: Duration) -> Result<Arc<HttpClients>> {
        let mut receiver = self.http.subscribe();
        let usable = async {
            loop {
                {
                    let clients = receiver.borrow_and_update();
                    match &clients.route {
                        HttpRoute::Direct | HttpRoute::External(_) | HttpRoute::BuiltIn(_) => {
                            return Ok(clients.clone());
                        }
                        HttpRoute::Blocked(error) => return Err(error.clone()),
                        HttpRoute::PendingTor => {}
                    }
                }

                receiver
                    .changed()
                    .await
                    .map_err(|_| TorError::Runtime(TorRuntimeError::LifecycleChannelClosed))?;
            }
        };

        tokio::time::timeout(bound, usable)
            .await
            .unwrap_or_else(|_| Err(TorError::Runtime(TorRuntimeError::ReadyTimeout)))
    }

    fn rotate_scan_epoch(&self) {
        let previous = std::mem::replace(&mut *self.scan_epoch.lock(), CancellationToken::new());
        previous.cancel();
    }

    /// Spawns the runtime status forwarder once for the manager's lifetime
    ///
    /// Never restarted, so a latched terminal status is never re-emitted to platforms
    fn start_status_forwarder(&self) {
        let mut forwarder = self.status_forwarder.lock();
        if forwarder.is_some() {
            return;
        }

        let mut statuses = tor_runtime::subscribe_status();
        let reconciler = self.reconciler.clone();
        let task = cove_tokio::task::spawn(async move {
            let mut last_sent: Option<Message> = None;
            let mut runtime_was_live = false;

            loop {
                let status = statuses.borrow_and_update().clone();
                if status == Status::Ready {
                    TOR_MANAGER.clear_launch_breaker();
                }

                // a runtime that reaches a terminal status has released its loopback
                // port, so the route built on it has to go before another local process
                // can bind that port and be handed Cove's labelled traffic
                match terminal_route_error(&status) {
                    Some(error) if runtime_was_live => {
                        runtime_was_live = false;
                        cove_tokio::task::spawn(invalidate_route_for_dead_runtime(
                            tor_runtime::generation(),
                            error,
                        ));
                    }
                    Some(_) => {}
                    None => runtime_was_live = true,
                }

                if let Some(message) = reconcile_runtime_status(status)
                    && last_sent.as_ref() != Some(&message)
                {
                    reconciler.send_sync(message.clone());
                    last_sent = Some(message);
                }

                if statuses.changed().await.is_err() {
                    return;
                }
            }
        });

        forwarder.replace(task.abort_handle());
    }

    fn send(&self, message: Message) {
        self.reconciler.send_sync(message);
    }

    /// How a blocked route is reported to a platform that asked for the current state
    ///
    /// A suppressed auto-start and a runtime that is merely not running both have their
    /// own platform affordance, and neither reads as an error the user has to decode
    fn blocked_route_message(&self, error: TorError) -> Message {
        if Database::global().global_config.tor_launch_health().suppresses_auto_start() {
            return Message::AutoStartSuppressed;
        }

        if error == TorError::Runtime(TorRuntimeError::NotRunning) {
            return Message::Stopped;
        }

        Message::Failed { origin: TorFailureOrigin::Lifecycle, error }
    }

    fn send_test_update(&self, step: TorTestStep, state: TorTestState) {
        self.send(Message::ConnectionTest(TorTestUpdate { step, state }));
    }

    fn fail_connection_test(&self, step: TorTestStep, reason: &str) {
        self.send_test_update(step, TorTestState::Failed(reason.to_string()));
    }

    async fn run_connection_test(&self) {
        self.send_test_update(TorTestStep::ProxyReachable, TorTestState::Running);

        let config = {
            let _operation = self.operations.lock().await;
            let config = match Database::global().global_config.tor_config() {
                Ok(config) => config,
                Err(_) => {
                    // an unreadable config blocks every route, not just this test
                    self.fail_lifecycle(TorError::ConfigUnreadable);
                    self.fail_connection_test(
                        TorTestStep::ProxyReachable,
                        "Unable to read Tor settings",
                    );

                    return;
                }
            };

            // the user asked for this test, so the breaker must not refuse its launch
            if config == TorConfig::BuiltIn
                && let Err(refusal) = self.ensure_built_in_running(StartIntent::UserInitiated).await
            {
                self.send(Message::Failed {
                    origin: TorFailureOrigin::ConnectionTest,
                    error: refusal.into(),
                });
                self.fail_connection_test(
                    TorTestStep::ProxyReachable,
                    "The built-in Tor proxy is unavailable",
                );

                return;
            }

            config
        };

        let endpoint = match &config {
            TorConfig::Off => {
                self.fail_connection_test(TorTestStep::ProxyReachable, "Tor is disabled");
                return;
            }
            TorConfig::BuiltIn => {
                let ready = tor_runtime::built_in_socks_endpoint(EndpointWait::Within(
                    CONNECTION_TEST_TIMEOUT,
                ))
                .await;

                match ready {
                    Ok(endpoint) => (endpoint.ip().to_string(), endpoint.port()),
                    Err(tor_runtime::Error::ReadyTimeout) => {
                        self.fail_connection_test(
                            TorTestStep::ProxyReachable,
                            "Tor did not become ready in time",
                        );

                        return;
                    }
                    Err(error) => {
                        self.send(Message::Failed {
                            origin: TorFailureOrigin::ConnectionTest,
                            error: error.into(),
                        });
                        self.fail_connection_test(
                            TorTestStep::ProxyReachable,
                            "The built-in Tor proxy is unavailable",
                        );

                        return;
                    }
                }
            }
            TorConfig::External { host, port } => (host.clone(), *port),
        };

        let proxy_result = cove_tokio::unblock::run_blocking(move || {
            verify_socks5_handshake(&endpoint.0, endpoint.1)
        })
        .await;

        if let Err(error) = proxy_result {
            self.fail_connection_test(TorTestStep::ProxyReachable, &error.to_string());
            return;
        }

        self.send_test_update(TorTestStep::ProxyReachable, TorTestState::Passed);
        self.send_test_update(TorTestStep::NodeReachableViaTor, TorTestState::Running);

        let (node, options) = {
            let _operation = self.operations.lock().await;
            let database = Database::global();
            match database.global_config.tor_config() {
                Ok(current) if current == config => {}
                Ok(_) => {
                    self.fail_connection_test(
                        TorTestStep::NodeReachableViaTor,
                        "Tor settings changed during the test",
                    );

                    return;
                }
                Err(_) => {
                    self.fail_lifecycle(TorError::ConfigUnreadable);
                    self.fail_connection_test(
                        TorTestStep::NodeReachableViaTor,
                        "Unable to read Tor settings",
                    );

                    return;
                }
            }

            let node = database.global_config.selected_node();
            let options = match NodeClientOptions::from_db_for_node(&node) {
                Ok(options) => options,
                Err(_) => {
                    self.fail_lifecycle(TorError::ConfigUnreadable);
                    self.fail_connection_test(
                        TorTestStep::NodeReachableViaTor,
                        "Unable to read Tor settings",
                    );

                    return;
                }
            };

            (node, options)
        };

        let result = tokio::time::timeout(CONNECTION_TEST_TIMEOUT, async {
            let client = crate::node::client::NodeClient::new_with_options(&node, options).await?;

            client.check_url().await
        })
        .await;

        match result {
            Ok(Ok(())) => {
                self.send_test_update(TorTestStep::NodeReachableViaTor, TorTestState::Passed);
            }
            Ok(Err(_)) => self.fail_connection_test(
                TorTestStep::NodeReachableViaTor,
                "The selected node is unavailable through Tor",
            ),
            Err(_) => self.fail_connection_test(
                TorTestStep::NodeReachableViaTor,
                "The selected node connection test timed out",
            ),
        }
    }

    async fn dispatch_inner(&self, action: TorManagerAction) -> Result<TorManagerDispatchResult> {
        if action == TorManagerAction::RunConnectionTest {
            return Ok(self.start_connection_test());
        }

        let _operation = self.operations.lock().await;
        match action {
            TorManagerAction::Enable => {
                let current = match Database::global().global_config.tor_config() {
                    Ok(config) => config,
                    Err(_) => return Err(self.fail_lifecycle(TorError::ConfigUnreadable)),
                };

                // explicit intent always overrides the crash-loop breaker
                self.clear_launch_breaker();

                let config = if current == TorConfig::Off { TorConfig::BuiltIn } else { current };
                self.apply_config(config).await?;
            }

            TorManagerAction::Disable | TorManagerAction::SetConfig(TorConfig::Off) => {
                let networks = onion_selected_networks(&Database::global().global_config);
                if !networks.is_empty() {
                    return Ok(TorManagerDispatchResult::DisableWarning(
                        TorDisableWarning::OnionNodesSelected { networks },
                    ));
                }

                self.apply_config(TorConfig::Off).await?;
            }

            TorManagerAction::DisableConfirmed => {
                if switch_onion_selections_to_clearnet_presets().is_err() {
                    let error = TorError::ClearnetFallback;
                    self.send(Message::Failed {
                        origin: TorFailureOrigin::Lifecycle,
                        error: error.clone(),
                    });

                    return Err(error);
                }

                self.apply_config(TorConfig::Off).await?;
            }

            TorManagerAction::SetConfig(config) => {
                self.apply_config(config).await?;
                self.clear_launch_breaker();
            }

            TorManagerAction::RunConnectionTest => return Ok(TorManagerDispatchResult::Applied),
        }

        Ok(TorManagerDispatchResult::Applied)
    }

    /// Starts one connection test, ignoring the request while another is streaming updates
    fn start_connection_test(&self) -> TorManagerDispatchResult {
        // the claim is moved into the task rather than made inside it, so a task that
        // is dropped before its first poll releases the claim instead of latching it
        // claimed on the singleton the spawned test runs against, which is the only
        // manager instance the platform can ever hold
        if let Some(in_flight) = ConnectionTestGuard::claim(&TOR_MANAGER.connection_test_in_flight)
        {
            cove_tokio::task::spawn(async move {
                let _in_flight = in_flight;

                TOR_MANAGER.run_connection_test().await;
            });
        }

        TorManagerDispatchResult::Applied
    }
}

/// Holds the single connection-test claim, clearing it on every drop path
struct ConnectionTestGuard(&'static AtomicBool);

impl ConnectionTestGuard {
    /// Claims the connection-test slot, or returns `None` while a test is running
    fn claim(in_flight: &'static AtomicBool) -> Option<Self> {
        in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(Self(in_flight))
    }
}

impl Drop for ConnectionTestGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Returns one traffic category's HTTP client once the configured route is usable
///
/// On the built-in route each category gets its own Tor circuits
pub(crate) async fn http_client(
    category: HttpTrafficCategory,
    bound: Duration,
) -> Result<reqwest::Client> {
    Ok(TOR_MANAGER.clients(bound).await?.client(category))
}

/// Cancellation token tied to the current Tor route configuration
///
/// Rotated, cancelling every token it issued, whenever the persisted config changes
pub(crate) fn scan_epoch_child_token() -> CancellationToken {
    TOR_MANAGER.scan_epoch.lock().child_token()
}

/// Networks whose selected node requires Tor, in stable network order
pub(crate) fn onion_selected_networks(config: &GlobalConfigTable) -> Vec<Network> {
    Network::iter().filter(|network| config.selected_node_for(*network).requires_tor()).collect()
}

/// Starts built-in Tor on demand before constructing a routed node client
pub(crate) async fn ensure_built_in_started() -> Result<()> {
    let manager = TOR_MANAGER.clone();
    let _operation = manager.operations.lock().await;
    let config = match Database::global().global_config.tor_config() {
        Ok(config) => config,
        Err(_) => return Err(manager.fail_lifecycle(TorError::ConfigUnreadable)),
    };

    if config != TorConfig::BuiltIn {
        return Err(TorError::Runtime(TorRuntimeError::NotRunning));
    }

    match manager.ensure_built_in_running(StartIntent::Automatic).await {
        Ok(()) => Ok(()),
        // the breaker has already blocked the route and reported itself
        Err(StartRefusal::AutoStartSuppressed) => {
            Err(TorError::Runtime(TorRuntimeError::NotRunning))
        }
        Err(StartRefusal::Failed(error)) => Err(manager.fail_lifecycle(error)),
    }
}

/// Whether the crash-loop breaker lets one launch through
///
/// A user who explicitly asks for Tor is always allowed to try: the breaker only stops
/// Cove from launching, on its own initiative, a runtime that keeps killing the process
const fn launch_is_allowed(intent: StartIntent, health: &TorLaunchHealth) -> bool {
    match intent {
        StartIntent::UserInitiated => true,
        StartIntent::Automatic => !health.suppresses_auto_start(),
    }
}

/// Whether the next start would launch a runtime instead of reusing a live one
///
/// The crash-loop budget only covers launches: an ensure over a runtime that is
/// already up happens on every node-client construction and must not spend it
fn starts_a_fresh_runtime() -> bool {
    matches!(*tor_runtime::subscribe_status().borrow(), Status::Stopped | Status::Failed(_))
}

/// The error a terminal runtime status leaves the built-in route in, if any
fn terminal_route_error(status: &Status) -> Option<TorError> {
    match status {
        Status::Starting | Status::Bootstrapping(_) | Status::Ready => None,
        // a stop nobody asked for still leaves a route the app cannot use
        Status::Stopped => Some(TorError::Runtime(TorRuntimeError::NotRunning)),
        Status::Failed(error) => Some(error.clone().into()),
    }
}

/// Fails requests closed once the runtime behind the installed route is gone
async fn invalidate_route_for_dead_runtime(expected_generation: u64, error: TorError) {
    let manager = TOR_MANAGER.clone();
    let _operation = manager.operations.lock().await;

    // both the config and the runtime may have changed while this task waited
    match Database::global().global_config.tor_config() {
        Ok(TorConfig::BuiltIn) => {}
        Ok(TorConfig::Off | TorConfig::External { .. }) => return,
        Err(_) => {
            manager.fail_lifecycle(TorError::ConfigUnreadable);
            return;
        }
    }

    // a newer generation owns the route, and its own start installed one
    if tor_runtime::generation() != expected_generation {
        return;
    }

    // the status forwarder already reported the failure this route is blocked on
    manager.block_http_requests(error);
}

/// Installs the built-in proxy client once bootstrap finishes for one runtime generation
async fn swap_http_client_when_ready(expected_generation: u64) {
    let ready = tor_runtime::built_in_socks_endpoint(EndpointWait::UntilTerminal).await;

    let manager = TOR_MANAGER.clone();
    let _operation = manager.operations.lock().await;

    // both the config and the runtime may have changed while bootstrap was in flight
    match Database::global().global_config.tor_config() {
        Ok(TorConfig::BuiltIn) => {}
        Ok(TorConfig::Off | TorConfig::External { .. }) => return,
        Err(_) => {
            manager.fail_lifecycle(TorError::ConfigUnreadable);
            return;
        }
    }

    if tor_runtime::generation() != expected_generation {
        return;
    }

    match ready {
        Ok(endpoint) => manager.install_http_route(HttpRoute::BuiltIn(endpoint)),

        Err(error) => {
            manager.fail_lifecycle(error.into());
        }
    }
}

/// Marks a launch healthy once its runtime survives the grace window without a terminal status
async fn observe_launch_health(expected_generation: u64) {
    tokio::time::sleep(LAUNCH_HEALTH_GRACE).await;

    if tor_runtime::generation() != expected_generation {
        return;
    }

    let status = tor_runtime::subscribe_status().borrow().clone();
    if matches!(status, Status::Stopped | Status::Failed(_)) {
        return;
    }

    TOR_MANAGER.clear_launch_breaker();
}

fn http_clients_for_route(route: HttpRoute) -> Result<HttpClients> {
    build_http_clients(route).map_err(|_| TorError::Runtime(TorRuntimeError::Configure))
}

fn build_http_clients(route: HttpRoute) -> Result<HttpClients, reqwest::Error> {
    let general =
        cove_http::new_client_with_socks_proxy(route.proxy(HttpTrafficCategory::General).as_ref())?;

    // only a labelling route needs a second pool: everything else shares one
    let payjoin = if route.isolates_categories() {
        cove_http::new_client_with_socks_proxy(route.proxy(HttpTrafficCategory::Payjoin).as_ref())?
    } else {
        general.clone()
    };

    let diagnostics = cove_http::new_client_without_redirects_with_socks_proxy(
        route.proxy(HttpTrafficCategory::Diagnostics).as_ref(),
    )?;

    Ok(HttpClients { route, general, payjoin, diagnostics })
}

fn blocked_http_clients(error: TorError) -> HttpClients {
    build_http_clients(HttpRoute::Blocked(error))
        .expect("the fail-closed HTTP proxy endpoint is valid")
}

#[uniffi::export(async_runtime = "tokio")]
impl RustTorManager {
    /// Returns the global Tor manager
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        TOR_MANAGER.clone()
    }

    /// Drains Tor lifecycle deltas into the platform reconciler
    pub fn listen_for_updates(&self, reconciler: Box<dyn TorManagerReconciler>) {
        self.reconciler.listen(move |field| match field {
            SingleOrMany::Single(message) => reconciler.reconcile(message),
            SingleOrMany::Many(messages) => {
                for message in messages {
                    reconciler.reconcile(message);
                }
            }
        });
    }

    /// Re-emits the current built-in runtime state so a foregrounding UI can resynchronize
    ///
    /// Sends only runtime-status deltas, never `ConfigChanged`: a refresh must not
    /// reset bootstrap progress display or wipe connection-test results
    pub fn refresh_status(&self) {
        let Ok(config) = Database::global().global_config.tor_config() else { return };
        if config != TorConfig::BuiltIn {
            return;
        }

        // requests take the route, not the runtime status: a runtime that is Ready
        // behind a blocked route must never be reported to the user as connected
        let blocked = match &self.http.borrow().route {
            HttpRoute::Blocked(error) => Some(error.clone()),
            HttpRoute::Direct
            | HttpRoute::External(_)
            | HttpRoute::BuiltIn(_)
            | HttpRoute::PendingTor => None,
        };

        if let Some(error) = blocked {
            self.send(self.blocked_route_message(error));
            return;
        }

        let status = tor_runtime::subscribe_status().borrow().clone();
        match status {
            // the forwarder reports progress for an in-flight start on its own
            Status::Starting => {}
            Status::Stopped => {
                let health = Database::global().global_config.tor_launch_health();
                let message = if health.suppresses_auto_start() {
                    Message::AutoStartSuppressed
                } else {
                    Message::Stopped
                };

                self.send(message);
            }
            other => {
                if let Some(message) = reconcile_runtime_status(other) {
                    self.send(message);
                }
            }
        }
    }

    /// Handles one Tor user intent
    pub async fn dispatch(&self, action: TorManagerAction) -> Result<TorManagerDispatchResult> {
        // detached so a cancelled platform task abandons only the await: a config
        // change can never be persisted without its runtime and route being applied
        let manager = TOR_MANAGER.clone();

        cove_tokio::task::spawn(async move { manager.dispatch_inner(action).await })
            .await
            .map_err(|_| TorError::Runtime(TorRuntimeError::LifecycleChannelClosed))?
    }
}

#[uniffi::export(callback_interface)]
pub trait TorManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Reconciles one Tor manager delta
    fn reconcile(&self, message: TorManagerReconcileMessage);
}

fn reconcile_runtime_status(status: Status) -> Option<Message> {
    match status {
        Status::Stopped | Status::Starting => None,
        Status::Bootstrapping(progress) => Some(Message::BootstrapProgress {
            percent: progress.percent,
            message: bootstrap_message(progress.blocked).to_string(),
        }),
        Status::Ready => Some(Message::Ready),
        Status::Failed(error) => {
            Some(Message::Failed { origin: TorFailureOrigin::Lifecycle, error: error.into() })
        }
    }
}

const fn bootstrap_message(blocked: Option<BootstrapBlockage>) -> &'static str {
    match blocked {
        None => "Connecting to the Tor network",
        Some(BootstrapBlockage::Disabled) => "Tor bootstrap is paused",
        Some(BootstrapBlockage::Offline) => "Waiting for an internet connection",
        Some(BootstrapBlockage::Filtering) => "The network may be filtering Tor",
        Some(BootstrapBlockage::CannotReachTor) => "Unable to reach the Tor network",
        Some(BootstrapBlockage::ClockSkewed) => "The device clock may be incorrect",
        Some(BootstrapBlockage::CannotBootstrapDirectory) => "Unable to download the Tor directory",
        Some(BootstrapBlockage::Unknown) => "Tor bootstrap is temporarily blocked",
    }
}

#[derive(Debug, thiserror::Error)]
enum ProxyTestError {
    #[error("The proxy address could not be resolved")]
    Resolve,
    #[error("The proxy could not be reached")]
    Connect,
    #[error("The proxy connection could not be configured")]
    Configure,
    #[error("The SOCKS5 handshake could not be sent")]
    Write,
    #[error("The SOCKS5 handshake did not complete")]
    Read,
    #[error("The endpoint is not a SOCKS5 proxy")]
    InvalidVersion,
    #[error("The SOCKS5 proxy does not accept unauthenticated connections")]
    AuthenticationRequired,
}

fn verify_socks5_handshake(host: &str, port: u16) -> Result<(), ProxyTestError> {
    let addresses = (host, port).to_socket_addrs().map_err(|_| ProxyTestError::Resolve)?;
    let mut last_stream = None;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, PROXY_TEST_TIMEOUT) {
            last_stream = Some(stream);
            break;
        }
    }
    let mut stream = last_stream.ok_or(ProxyTestError::Connect)?;

    stream.set_read_timeout(Some(PROXY_TEST_TIMEOUT)).map_err(|_| ProxyTestError::Configure)?;
    stream.set_write_timeout(Some(PROXY_TEST_TIMEOUT)).map_err(|_| ProxyTestError::Configure)?;
    stream.write_all(&[0x05, 0x01, 0x00]).map_err(|_| ProxyTestError::Write)?;

    let mut response = [0_u8; 2];
    stream.read_exact(&mut response).map_err(|_| ProxyTestError::Read)?;
    if response[0] != 0x05 {
        return Err(ProxyTestError::InvalidVersion);
    }
    if response[1] != 0x00 {
        return Err(ProxyTestError::AuthenticationRequired);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        thread,
    };

    use crate::node::Node;

    use super::*;

    fn external(host: &str, port: u16) -> TorConfig {
        TorConfig::External { host: host.to_string(), port }
    }

    #[test]
    fn tor_config_maps_to_the_expected_http_route() {
        assert_eq!(HttpRoute::for_config(&TorConfig::Off), Ok(HttpRoute::Direct));
        assert_eq!(HttpRoute::for_config(&TorConfig::BuiltIn), Ok(HttpRoute::PendingTor));
        assert_eq!(
            HttpRoute::for_config(&external("proxy.example", 9050)),
            Ok(HttpRoute::External(Socks5Proxy::new("proxy.example", 9050)))
        );
        assert_eq!(
            HttpRoute::for_config(&external("proxy.example/evil", 9050)),
            Err(TorError::InvalidExternalProxy)
        );
    }

    #[test]
    fn only_the_built_in_route_labels_traffic_per_category() {
        let built_in = HttpRoute::BuiltIn("127.0.0.1:9150".parse().unwrap());
        let usernames = [
            HttpTrafficCategory::General,
            HttpTrafficCategory::Payjoin,
            HttpTrafficCategory::Diagnostics,
        ]
        .map(|category| {
            built_in.proxy(category).and_then(|proxy| proxy.credentials()).map(|(user, _)| user)
        });

        assert_eq!(usernames, [Some("http"), Some("payjoin"), Some("diagnostics")]);
        assert!(built_in.isolates_categories());

        let external = HttpRoute::External(Socks5Proxy::new("proxy.example", 9050));
        for category in [
            HttpTrafficCategory::General,
            HttpTrafficCategory::Payjoin,
            HttpTrafficCategory::Diagnostics,
        ] {
            assert_eq!(external.proxy(category).unwrap().credentials(), None);
        }

        assert!(!external.isolates_categories());
        assert_eq!(HttpRoute::Direct.proxy(HttpTrafficCategory::General), None);
    }

    #[test]
    fn identical_configs_are_a_no_op_transition() {
        assert_eq!(
            config_transition(Some(&TorConfig::BuiltIn), &TorConfig::BuiltIn),
            ConfigTransition::NoOp
        );
        assert_eq!(
            config_transition(
                Some(&external("proxy.example", 9050)),
                &external("proxy.example", 9050)
            ),
            ConfigTransition::NoOp
        );

        assert_eq!(
            config_transition(Some(&TorConfig::Off), &TorConfig::BuiltIn),
            ConfigTransition::Switch { stop_runtime: false }
        );
        assert_eq!(
            config_transition(Some(&TorConfig::BuiltIn), &TorConfig::Off),
            ConfigTransition::Switch { stop_runtime: true }
        );

        // an unreadable current config may have left a runtime behind
        assert_eq!(
            config_transition(None, &TorConfig::Off),
            ConfigTransition::Switch { stop_runtime: true }
        );
    }

    #[test]
    fn onion_selected_networks_covers_every_network() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, config) = test_global_config();

        assert_eq!(onion_selected_networks(&config), Vec::new());

        config.set_tor_config(TorConfig::BuiltIn).unwrap();
        let onion = Node::new_electrum(
            "onion".to_string(),
            "tcp://example.onion:50001".to_string(),
            Network::Testnet,
        );
        config.set_selected_node(&onion).unwrap();

        // the active network is Bitcoin, so a Testnet-only onion selection still warns
        assert_eq!(onion_selected_networks(&config), vec![Network::Testnet]);
    }

    #[test]
    fn rotating_the_scan_epoch_cancels_previously_issued_tokens() {
        let epoch = Mutex::new(CancellationToken::new());
        let issued = epoch.lock().child_token();

        let previous = std::mem::replace(&mut *epoch.lock(), CancellationToken::new());
        previous.cancel();

        assert!(issued.is_cancelled());
        assert!(!epoch.lock().child_token().is_cancelled());
    }

    #[test]
    fn a_terminal_runtime_status_invalidates_the_installed_route() {
        let bootstrapping =
            Status::Bootstrapping(tor_runtime::BootstrapProgress { percent: 50, blocked: None });

        for live in [Status::Starting, bootstrapping, Status::Ready] {
            assert_eq!(terminal_route_error(&live), None, "status: {live:?}");
        }

        // the runtime released its loopback port, which another local process can take
        assert_eq!(
            terminal_route_error(&Status::Stopped),
            Some(TorError::Runtime(TorRuntimeError::NotRunning))
        );
        assert_eq!(
            terminal_route_error(&Status::Failed(tor_runtime::Error::ProxyStopped)),
            Some(TorError::Runtime(TorRuntimeError::ProxyStopped))
        );
    }

    #[test]
    fn the_breaker_refuses_automatic_launches_and_never_user_initiated_ones() {
        let mut health = TorLaunchHealth::default();

        assert!(launch_is_allowed(StartIntent::Automatic, &health));
        assert!(launch_is_allowed(StartIntent::UserInitiated, &health));

        while !health.suppresses_auto_start() {
            health.record_attempt();
        }

        assert!(!launch_is_allowed(StartIntent::Automatic, &health));
        assert!(launch_is_allowed(StartIntent::UserInitiated, &health));
    }

    #[test]
    fn one_connection_test_holds_the_claim_until_it_is_dropped() {
        static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

        let running = ConnectionTestGuard::claim(&IN_FLIGHT).expect("the slot starts free");
        assert!(ConnectionTestGuard::claim(&IN_FLIGHT).is_none());

        // a task dropped before its first poll must not latch the slot for good
        drop(running);
        assert!(ConnectionTestGuard::claim(&IN_FLIGHT).is_some());
    }

    #[test]
    fn proxy_test_requires_a_socks5_handshake() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 3];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request, [0x05, 0x01, 0x00]);
            stream.write_all(&[0x05, 0x00]).unwrap();
        });

        assert!(verify_socks5_handshake("127.0.0.1", port).is_ok());
        server.join().unwrap();
    }

    #[test]
    fn proxy_test_rejects_a_non_socks_endpoint() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 3];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(b"HT").unwrap();
        });

        assert!(matches!(
            verify_socks5_handshake("127.0.0.1", port),
            Err(ProxyTestError::InvalidVersion)
        ));
        server.join().unwrap();
    }

    fn test_global_config() -> (tempfile::TempDir, GlobalConfigTable) {
        let temp_dir = tempfile::tempdir().unwrap();
        let database = Arc::new(redb::Database::create(temp_dir.path().join("test.redb")).unwrap());
        let write_transaction = database.begin_write().unwrap();
        let config = GlobalConfigTable::new(database, &write_transaction);
        write_transaction.commit().unwrap();

        (temp_dir, config)
    }
}
