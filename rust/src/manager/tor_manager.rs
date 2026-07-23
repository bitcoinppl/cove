use std::{
    io::{Read as _, Write as _},
    net::{TcpStream, ToSocketAddrs as _},
    sync::{Arc, LazyLock},
    time::Duration,
};

use arc_swap::ArcSwap;
use cove_http::Socks5Proxy;
use parking_lot::Mutex;
use tokio::{sync::Mutex as AsyncMutex, task::AbortHandle};

use crate::{
    database::Database,
    manager::{deferred_sender::SingleOrMany, reconcile_channel::ReconcileChannel},
    node::{Node, client::NodeClientOptions},
    node_connect::{node_implies_tor, switch_to_first_clearnet_preset},
    tor::TorConfig,
    tor_runtime::{self, BootstrapBlockage, Status},
};

type Message = TorManagerReconcileMessage;
type Result<T, E = TorError> = std::result::Result<T, E>;

const PROXY_TEST_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpRoute {
    Direct,
    Proxy(Socks5Proxy),
    Blocked,
}

impl From<&TorConfig> for HttpRoute {
    fn from(config: &TorConfig) -> Self {
        match config {
            TorConfig::Off => Self::Direct,
            TorConfig::BuiltIn => Self::Blocked,
            TorConfig::External { host, port } => {
                Self::Proxy(Socks5Proxy::new(host.clone(), *port))
            }
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
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TorDisableWarning {
    /// The selected node is only reachable through onion routing
    SelectedNodeIsOnion,
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
    /// A Tor lifecycle operation failed
    Failed(TorError),
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

/// Rust implementation of the Tor manager singleton
#[derive(Debug, uniffi::Object)]
pub struct RustTorManager {
    reconciler: ReconcileChannel<Message>,
    operations: AsyncMutex<()>,
    status_forwarder: Mutex<Option<AbortHandle>>,
    http_client: ArcSwap<reqwest::Client>,
    http_route: ArcSwap<HttpRoute>,
}

impl RustTorManager {
    fn init() -> Arc<Self> {
        let (http_client, http_route, initial_error) = match Database::global()
            .global_config
            .tor_config()
        {
            Ok(config) => initial_http_client(&config),
            Err(_) => {
                (fail_closed_http_client(), HttpRoute::Blocked, Some(TorError::ConfigUnreadable))
            }
        };

        let manager = Arc::new(Self {
            reconciler: ReconcileChannel::new(1000),
            operations: AsyncMutex::new(()),
            status_forwarder: Mutex::new(None),
            http_client: ArcSwap::from_pointee(http_client),
            http_route: ArcSwap::from_pointee(http_route),
        });
        if let Some(error) = initial_error {
            manager.send(Message::Failed(error));
        }

        manager
    }

    /// Starts configured built-in Tor during application initialization
    pub(crate) async fn warmup(&self) {
        let _operation = self.operations.lock().await;
        let config = match Database::global().global_config.tor_config() {
            Ok(config) => config,
            Err(_) => {
                self.block_http_requests();
                self.send(Message::Failed(TorError::ConfigUnreadable));
                return;
            }
        };

        self.send(Message::ConfigChanged(config.clone()));
        match config {
            TorConfig::BuiltIn => {
                self.block_http_requests();
                if let Err(error) = self.start_built_in().await {
                    self.send(Message::Failed(error));
                }
            }
            TorConfig::Off | TorConfig::External { .. } => {
                let route = HttpRoute::from(&config);
                match http_client_for_route(&route) {
                    Ok(client) => self.set_http_client(client, route),
                    Err(_) => {
                        self.block_http_requests();
                        self.send(Message::Failed(http_client_error()));
                    }
                }
            }
        }
    }

    async fn apply_config(&self, config: TorConfig) -> Result<()> {
        if let TorConfig::External { host, port } = &config
            && (host.trim().is_empty() || host.contains(char::is_whitespace) || *port == 0)
        {
            return Err(TorError::InvalidExternalProxy);
        }

        // a user-driven config change is the recovery path for unreadable persisted state
        let current = Database::global().global_config.tor_config().ok();
        let prepared_http_route = match config {
            TorConfig::BuiltIn => None,
            TorConfig::Off | TorConfig::External { .. } => {
                let route = HttpRoute::from(&config);
                let client = http_client_for_route(&route).map_err(|_| http_client_error())?;

                Some((client, route))
            }
        };

        if current.as_ref() != Some(&config) {
            self.block_http_requests();
            self.stop_status_forwarder();
            if let Err(error) = tor_runtime::stop().await {
                let error = TorError::from(error);
                self.send(Message::Failed(error.clone()));
                return Err(error);
            }

            if Database::global().global_config.set_tor_config(config.clone()).is_err() {
                let error = TorError::Persistence;
                self.send(Message::Failed(error.clone()));
                return Err(error);
            }
        }

        self.send(Message::ConfigChanged(config.clone()));
        if config == TorConfig::BuiltIn {
            self.block_http_requests();
            self.start_built_in().await?;
        } else {
            let Some((client, route)) = prepared_http_route else {
                let error = http_client_error();
                self.send(Message::Failed(error.clone()));
                return Err(error);
            };
            self.set_http_client(client, route);

            if current.as_ref() != Some(&config) {
                self.send(Message::Stopped);
            }
        }

        Ok(())
    }

    async fn start_built_in(&self) -> Result<()> {
        self.start_status_forwarder();
        tor_runtime::start().await.map_err(TorError::from)?;
        spawn_built_in_http_client_swap();

        Ok(())
    }

    fn block_http_requests(&self) {
        self.set_http_client(fail_closed_http_client(), HttpRoute::Blocked);
    }

    fn set_http_client(&self, client: reqwest::Client, route: HttpRoute) {
        self.http_client.store(Arc::new(client));
        self.http_route.store(Arc::new(route));
    }

    fn http_client(&self) -> reqwest::Client {
        self.http_client.load().as_ref().clone()
    }

    fn http_client_without_redirects(&self) -> reqwest::Client {
        let route = self.http_route.load();

        http_client_without_redirects_for_route(&route)
            .unwrap_or_else(|_| fail_closed_http_client_without_redirects())
    }

    fn start_status_forwarder(&self) {
        self.stop_status_forwarder();

        let mut statuses = tor_runtime::subscribe_status();
        let reconciler = self.reconciler.clone();
        let task = cove_tokio::task::spawn(async move {
            let initial = statuses.borrow_and_update().clone();
            if let Some(message) = reconcile_runtime_status(initial) {
                reconciler.send_sync(message);
            }

            while statuses.changed().await.is_ok() {
                let status = statuses.borrow_and_update().clone();
                if let Some(message) = reconcile_runtime_status(status) {
                    reconciler.send_sync(message);
                }
            }
        });

        self.status_forwarder.lock().replace(task.abort_handle());
    }

    fn stop_status_forwarder(&self) {
        if let Some(forwarder) = self.status_forwarder.lock().take() {
            forwarder.abort();
        }
    }

    fn send(&self, message: Message) {
        self.reconciler.send_sync(message);
    }

    fn send_test_update(&self, step: TorTestStep, state: TorTestState) {
        self.send(Message::ConnectionTest(TorTestUpdate { step, state }));
    }

    async fn run_connection_test(&self) {
        self.send_test_update(TorTestStep::ProxyReachable, TorTestState::Running);

        let config = {
            let _operation = self.operations.lock().await;
            let config = match Database::global().global_config.tor_config() {
                Ok(config) => config,
                Err(_) => {
                    self.block_http_requests();
                    self.send(Message::Failed(TorError::ConfigUnreadable));
                    self.send_test_update(
                        TorTestStep::ProxyReachable,
                        TorTestState::Failed("Unable to read Tor settings".to_string()),
                    );
                    return;
                }
            };

            if config == TorConfig::BuiltIn && self.start_built_in().await.is_err() {
                self.send_test_update(
                    TorTestStep::ProxyReachable,
                    TorTestState::Failed("The built-in Tor proxy is unavailable".to_string()),
                );
                return;
            }

            config
        };

        let endpoint = match &config {
            TorConfig::Off => {
                self.send_test_update(
                    TorTestStep::ProxyReachable,
                    TorTestState::Failed("Tor is disabled".to_string()),
                );
                return;
            }
            TorConfig::BuiltIn => {
                match tokio::time::timeout(
                    CONNECTION_TEST_TIMEOUT,
                    tor_runtime::built_in_socks_endpoint(),
                )
                .await
                {
                    Ok(Ok(endpoint)) => (endpoint.ip().to_string(), endpoint.port()),
                    Ok(Err(error)) => {
                        self.send(Message::Failed(error.into()));
                        self.send_test_update(
                            TorTestStep::ProxyReachable,
                            TorTestState::Failed(
                                "The built-in Tor proxy is unavailable".to_string(),
                            ),
                        );
                        return;
                    }
                    Err(_) => {
                        self.send_test_update(
                            TorTestStep::ProxyReachable,
                            TorTestState::Failed("Tor did not become ready in time".to_string()),
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
            self.send_test_update(
                TorTestStep::ProxyReachable,
                TorTestState::Failed(error.to_string()),
            );
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
                    self.send_test_update(
                        TorTestStep::NodeReachableViaTor,
                        TorTestState::Failed("Tor settings changed during the test".to_string()),
                    );
                    return;
                }
                Err(_) => {
                    self.block_http_requests();
                    self.send(Message::Failed(TorError::ConfigUnreadable));
                    self.send_test_update(
                        TorTestStep::NodeReachableViaTor,
                        TorTestState::Failed("Unable to read Tor settings".to_string()),
                    );
                    return;
                }
            }

            let node = database.global_config.selected_node();
            let options = match NodeClientOptions::from_db_for_node(&node) {
                Ok(options) => options,
                Err(_) => {
                    self.block_http_requests();
                    self.send(Message::Failed(TorError::ConfigUnreadable));
                    self.send_test_update(
                        TorTestStep::NodeReachableViaTor,
                        TorTestState::Failed("Unable to read Tor settings".to_string()),
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
            Ok(Err(_)) => self.send_test_update(
                TorTestStep::NodeReachableViaTor,
                TorTestState::Failed("The selected node is unavailable through Tor".to_string()),
            ),
            Err(_) => self.send_test_update(
                TorTestStep::NodeReachableViaTor,
                TorTestState::Failed("The selected node connection test timed out".to_string()),
            ),
        }
    }
}

/// Returns the currently configured shared HTTP client
pub(crate) fn http_client() -> reqwest::Client {
    TOR_MANAGER.http_client()
}

/// Returns a route-matched HTTP client that does not follow redirects
pub(crate) fn http_client_without_redirects() -> reqwest::Client {
    TOR_MANAGER.http_client_without_redirects()
}

/// Starts built-in Tor on demand before constructing a routed node client
pub(crate) async fn ensure_built_in_started() -> Result<()> {
    let manager = TOR_MANAGER.clone();
    let _operation = manager.operations.lock().await;
    let config = match Database::global().global_config.tor_config() {
        Ok(config) => config,
        Err(_) => {
            manager.block_http_requests();
            let error = TorError::ConfigUnreadable;
            manager.send(Message::Failed(error.clone()));
            return Err(error);
        }
    };

    if config != TorConfig::BuiltIn {
        return Err(TorError::Runtime(TorRuntimeError::NotRunning));
    }

    manager.start_built_in().await
}

/// Swaps the shared HTTP client to the built-in endpoint once bootstrap
/// finishes, keeping requests fail-closed in the meantime so enabling Tor
/// never blocks on bootstrap
fn spawn_built_in_http_client_swap() {
    cove_tokio::task::spawn(async move {
        let endpoint = match tor_runtime::built_in_socks_endpoint().await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                TOR_MANAGER.send(Message::Failed(error.into()));
                return;
            }
        };

        // config may have changed while bootstrap was in flight
        let manager = TOR_MANAGER.clone();
        let _operation = manager.operations.lock().await;
        match Database::global().global_config.tor_config() {
            Ok(TorConfig::BuiltIn) => {}
            Ok(TorConfig::Off | TorConfig::External { .. }) => return,
            Err(_) => {
                manager.block_http_requests();
                manager.send(Message::Failed(TorError::ConfigUnreadable));
                return;
            }
        }

        let proxy = Socks5Proxy::new(endpoint.ip().to_string(), endpoint.port());
        match cove_http::new_client_with_socks_proxy(Some(&proxy)) {
            Ok(client) => manager.set_http_client(client, HttpRoute::Proxy(proxy)),
            Err(_) => {
                manager.block_http_requests();
                manager.send(Message::Failed(http_client_error()));
            }
        }
    });
}

fn initial_http_client(config: &TorConfig) -> (reqwest::Client, HttpRoute, Option<TorError>) {
    let route = HttpRoute::from(config);
    match http_client_for_route(&route) {
        Ok(client) => (client, route, None),
        Err(_) => (fail_closed_http_client(), HttpRoute::Blocked, Some(http_client_error())),
    }
}

fn http_client_for_route(route: &HttpRoute) -> Result<reqwest::Client, reqwest::Error> {
    match route {
        HttpRoute::Direct => cove_http::new_client_with_socks_proxy(None),
        HttpRoute::Proxy(proxy) => cove_http::new_client_with_socks_proxy(Some(proxy)),
        HttpRoute::Blocked => Ok(fail_closed_http_client()),
    }
}

fn http_client_without_redirects_for_route(
    route: &HttpRoute,
) -> Result<reqwest::Client, reqwest::Error> {
    match route {
        HttpRoute::Direct => cove_http::new_client_without_redirects_with_socks_proxy(None),
        HttpRoute::Proxy(proxy) => {
            cove_http::new_client_without_redirects_with_socks_proxy(Some(proxy))
        }
        HttpRoute::Blocked => Ok(fail_closed_http_client_without_redirects()),
    }
}

fn fail_closed_http_client() -> reqwest::Client {
    // route requests to an invalid local SOCKS endpoint so initialization cannot use clearnet
    let proxy = Socks5Proxy::new("127.0.0.1", 0);

    cove_http::new_client_with_socks_proxy(Some(&proxy))
        .expect("the fail-closed HTTP proxy endpoint is valid")
}

fn fail_closed_http_client_without_redirects() -> reqwest::Client {
    let proxy = Socks5Proxy::new("127.0.0.1", 0);

    cove_http::new_client_without_redirects_with_socks_proxy(Some(&proxy))
        .expect("the fail-closed HTTP proxy endpoint is valid")
}

const fn http_client_error() -> TorError {
    TorError::Runtime(TorRuntimeError::Configure)
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

    /// Handles one Tor user intent
    pub async fn dispatch(&self, action: TorManagerAction) -> Result<TorManagerDispatchResult> {
        if action == TorManagerAction::RunConnectionTest {
            cove_tokio::task::spawn(TOR_MANAGER.run_connection_test());
            return Ok(TorManagerDispatchResult::Applied);
        }

        let _operation = self.operations.lock().await;
        match action {
            TorManagerAction::Enable => {
                let current = match Database::global().global_config.tor_config() {
                    Ok(config) => config,
                    Err(_) => {
                        self.block_http_requests();
                        let error = TorError::ConfigUnreadable;
                        self.send(Message::Failed(error.clone()));
                        return Err(error);
                    }
                };
                let config = if current == TorConfig::Off { TorConfig::BuiltIn } else { current };

                self.apply_config(config).await?;
            }
            TorManagerAction::Disable | TorManagerAction::SetConfig(TorConfig::Off) => {
                let selected_node = Database::global().global_config.selected_node();
                if let Some(warning) = disable_warning(&selected_node) {
                    return Ok(TorManagerDispatchResult::DisableWarning(warning));
                }

                self.apply_config(TorConfig::Off).await?;
            }
            TorManagerAction::DisableConfirmed => {
                let selected_node = Database::global().global_config.selected_node();
                if node_implies_tor(&selected_node) && switch_to_first_clearnet_preset().is_err() {
                    let error = TorError::ClearnetFallback;
                    self.send(Message::Failed(error.clone()));
                    return Err(error);
                }

                self.apply_config(TorConfig::Off).await?;
            }
            TorManagerAction::SetConfig(config) => self.apply_config(config).await?,
            TorManagerAction::RunConnectionTest => return Ok(TorManagerDispatchResult::Applied),
        }

        Ok(TorManagerDispatchResult::Applied)
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
        Status::Failed(error) => Some(Message::Failed(error.into())),
    }
}

fn disable_warning(selected_node: &Node) -> Option<TorDisableWarning> {
    node_implies_tor(selected_node).then_some(TorDisableWarning::SelectedNodeIsOnion)
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

    use cove_types::Network;

    use super::*;

    #[test]
    fn tor_config_maps_to_the_expected_http_route() {
        assert_eq!(HttpRoute::from(&TorConfig::Off), HttpRoute::Direct);
        assert_eq!(HttpRoute::from(&TorConfig::BuiltIn), HttpRoute::Blocked);
        assert_eq!(
            HttpRoute::from(&TorConfig::External { host: "proxy.example".to_string(), port: 9050 }),
            HttpRoute::Proxy(Socks5Proxy::new("proxy.example", 9050))
        );
    }

    #[test]
    fn disabling_with_an_onion_node_requires_confirmation() {
        let onion = Node::new_electrum(
            "onion".to_string(),
            "ssl://example.onion:50002".to_string(),
            Network::Bitcoin,
        );
        let clearnet = Node::new_electrum(
            "clearnet".to_string(),
            "ssl://example.com:50002".to_string(),
            Network::Bitcoin,
        );

        assert_eq!(disable_warning(&onion), Some(TorDisableWarning::SelectedNodeIsOnion));
        assert_eq!(disable_warning(&clearnet), None);
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
}
