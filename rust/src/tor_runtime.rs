use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::LazyLock,
    thread,
};

use arti::proxy::ListenProtocols;
use arti_client::{
    BootstrapBehavior, TorClient, TorClientConfig,
    config::TorClientConfigBuilder,
    status::{BlockageKind, BootstrapStatus},
};
use futures::{StreamExt as _, future::pending};
use tokio::sync::{Mutex, oneshot, watch};
use tor_rtcompat::{
    NetStreamListener as _, NetStreamProvider as _, PreferredRuntime, ToplevelBlockOn as _,
};
use tracing::{debug, error};

use cove_common::consts::ROOT_DATA_DIR;

/// Error produced by the built-in Tor runtime
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A Tor storage directory could not be created
    #[error("failed to create built-in Tor directory {path}: {message}")]
    CreateDirectory { path: PathBuf, message: String },

    /// The Arti client configuration could not be built
    #[error("failed to configure built-in Tor: {0}")]
    Configure(String),

    /// The dedicated runtime thread could not be spawned
    #[error("failed to spawn built-in Tor runtime thread: {0}")]
    SpawnThread(String),

    /// The Arti-compatible async runtime could not be created
    #[error("failed to create built-in Tor async runtime: {0}")]
    CreateRuntime(String),

    /// The loopback SOCKS listener could not be bound
    #[error("failed to bind built-in Tor SOCKS listener: {0}")]
    BindListener(String),

    /// The bound SOCKS listener's address could not be read
    #[error("failed to read built-in Tor SOCKS listener address: {0}")]
    ReadListenerAddress(String),

    /// The Arti client could not be created
    #[error("failed to create built-in Tor client: {0}")]
    CreateClient(String),

    /// Arti could not bootstrap to a traffic-ready state
    #[error("built-in Tor bootstrap failed: {0}")]
    Bootstrap(String),

    /// The SOCKS proxy exited with an error
    #[error("built-in Tor SOCKS proxy failed: {0}")]
    Proxy(String),

    /// The SOCKS proxy exited without a shutdown request
    #[error("built-in Tor SOCKS proxy stopped unexpectedly")]
    ProxyStopped,

    /// Arti's typed bootstrap status stream ended unexpectedly
    #[error("built-in Tor bootstrap status stream closed")]
    BootstrapStatusStreamClosed,

    /// The runtime stopped before it became ready for traffic
    #[error("built-in Tor runtime stopped before it became ready")]
    StoppedBeforeReady,

    /// An internal runtime lifecycle channel closed unexpectedly
    #[error("built-in Tor runtime lifecycle channel closed")]
    LifecycleChannelClosed,
}

/// Typed reason Arti reports for blocked bootstrap progress
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapBlockage {
    /// Bootstrapping has not been enabled
    Disabled,
    /// The device appears to be offline
    Offline,
    /// The network appears to filter Tor traffic
    Filtering,
    /// The Tor network cannot currently be reached
    CannotReachTor,
    /// Clock skew prevents progress
    ClockSkewed,
    /// A usable Tor directory cannot be bootstrapped
    CannotBootstrapDirectory,
    /// Arti reported a blockage unknown to this version of Cove
    Unknown,
}

/// Typed snapshot of built-in Tor bootstrap progress
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapProgress {
    /// Rounded completion percentage in the inclusive range 0 through 100
    pub percent: u8,
    /// Typed reason bootstrap is currently blocked, when Arti reports one
    pub blocked: Option<BootstrapBlockage>,
}

/// Current lifecycle and bootstrap state of the built-in Tor runtime
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// No built-in Tor runtime is active
    Stopped,
    /// The runtime thread and SOCKS listener are being initialized
    Starting,
    /// Arti is bootstrapping and is not yet ready for traffic
    Bootstrapping(BootstrapProgress),
    /// Arti reports that the proxy is ready for traffic
    Ready,
    /// The runtime stopped because of a typed error
    Failed(Error),
}

struct RuntimeController {
    lifecycle: Mutex<Lifecycle>,
    status_tx: watch::Sender<Status>,
}

enum Lifecycle {
    Stopped,
    Running(RuntimeHandle),
}

struct RuntimeHandle {
    endpoint: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
    stopped_rx: oneshot::Receiver<Result<(), Error>>,
    shutdown_requested: bool,
}

struct PendingRuntime {
    startup_rx: oneshot::Receiver<Result<SocketAddr, Error>>,
    shutdown_tx: watch::Sender<bool>,
    stopped_rx: oneshot::Receiver<Result<(), Error>>,
}

static BUILT_IN_TOR: LazyLock<RuntimeController> = LazyLock::new(RuntimeController::new);

impl RuntimeController {
    fn new() -> Self {
        let (status_tx, _) = watch::channel(Status::Stopped);

        Self { lifecycle: Mutex::new(Lifecycle::Stopped), status_tx }
    }

    async fn start(&self) -> Result<(), Error> {
        let mut lifecycle = self.lifecycle.lock().await;
        let finished = match &mut *lifecycle {
            Lifecycle::Stopped => None,
            Lifecycle::Running(handle) if handle.shutdown_requested => {
                Some((&mut handle.stopped_rx).await.unwrap_or(Err(Error::LifecycleChannelClosed)))
            }
            Lifecycle::Running(handle) => match handle.stopped_rx.try_recv() {
                Ok(result) => Some(result),
                Err(oneshot::error::TryRecvError::Empty) => return Ok(()),
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(Err(Error::LifecycleChannelClosed))
                }
            },
        };

        if let Some(Err(error)) = &finished {
            self.status_tx.send_replace(Status::Failed(error.clone()));
        }
        if finished.is_some() {
            *lifecycle = Lifecycle::Stopped;
        }

        self.status_tx.send_replace(Status::Starting);
        let pending = match spawn_runtime_thread(self.status_tx.clone()) {
            Ok(pending) => pending,
            Err(error) => {
                self.status_tx.send_replace(Status::Failed(error.clone()));
                return Err(error);
            }
        };

        let PendingRuntime { startup_rx, shutdown_tx, stopped_rx } = pending;
        let endpoint = match startup_rx.await {
            Ok(Ok(endpoint)) => endpoint,
            Ok(Err(error)) => {
                self.status_tx.send_replace(Status::Failed(error.clone()));
                return Err(error);
            }
            Err(_) => {
                let error = Error::LifecycleChannelClosed;
                self.status_tx.send_replace(Status::Failed(error.clone()));
                return Err(error);
            }
        };

        *lifecycle = Lifecycle::Running(RuntimeHandle {
            endpoint,
            shutdown_tx,
            stopped_rx,
            shutdown_requested: false,
        });

        Ok(())
    }

    async fn stop(&self) -> Result<(), Error> {
        let mut lifecycle = self.lifecycle.lock().await;
        let result = match &mut *lifecycle {
            Lifecycle::Stopped => {
                self.status_tx.send_replace(Status::Stopped);
                return Ok(());
            }
            Lifecycle::Running(handle) => {
                handle.shutdown_requested = true;
                handle.shutdown_tx.send_replace(true);

                (&mut handle.stopped_rx).await.unwrap_or(Err(Error::LifecycleChannelClosed))
            }
        };

        *lifecycle = Lifecycle::Stopped;
        match &result {
            Ok(()) => {
                self.status_tx.send_replace(Status::Stopped);
            }
            Err(error) => {
                self.status_tx.send_replace(Status::Failed(error.clone()));
            }
        }

        result
    }

    async fn endpoint(&self) -> Result<SocketAddr, Error> {
        self.start().await?;

        let endpoint = {
            let lifecycle = self.lifecycle.lock().await;
            match &*lifecycle {
                Lifecycle::Stopped => return Err(Error::StoppedBeforeReady),
                Lifecycle::Running(handle) => handle.endpoint,
            }
        };
        let mut status_rx = self.status_tx.subscribe();

        loop {
            match status_rx.borrow_and_update().clone() {
                Status::Ready => return Ok(endpoint),
                Status::Failed(error) => return Err(error),
                Status::Stopped => return Err(Error::StoppedBeforeReady),
                Status::Starting | Status::Bootstrapping(_) => {}
            }

            status_rx.changed().await.map_err(|_| Error::LifecycleChannelClosed)?;
        }
    }
}

/// Starts the built-in Tor runtime and binds its loopback SOCKS listener
pub async fn start() -> Result<(), Error> {
    BUILT_IN_TOR.start().await
}

/// Stops the built-in Tor runtime and awaits completion of its dedicated thread
pub async fn stop() -> Result<(), Error> {
    BUILT_IN_TOR.stop().await
}

/// Returns the loopback SOCKS endpoint after Arti reports readiness for traffic
pub async fn built_in_socks_endpoint() -> Result<SocketAddr, Error> {
    BUILT_IN_TOR.endpoint().await
}

/// Subscribes to typed lifecycle and bootstrap status updates
pub fn subscribe_status() -> watch::Receiver<Status> {
    BUILT_IN_TOR.status_tx.subscribe()
}

fn spawn_runtime_thread(status_tx: watch::Sender<Status>) -> Result<PendingRuntime, Error> {
    let (startup_tx, startup_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (stopped_tx, stopped_rx) = oneshot::channel();

    thread::Builder::new()
        .name("cove-built-in-tor".to_string())
        .spawn(move || run_runtime_thread(startup_tx, shutdown_rx, stopped_tx, status_tx))
        .map_err(|error| Error::SpawnThread(error.to_string()))?;

    Ok(PendingRuntime { startup_rx, shutdown_tx, stopped_rx })
}

fn run_runtime_thread(
    startup_tx: oneshot::Sender<Result<SocketAddr, Error>>,
    shutdown_rx: watch::Receiver<bool>,
    stopped_tx: oneshot::Sender<Result<(), Error>>,
    status_tx: watch::Sender<Status>,
) {
    let config = match build_tor_client_config() {
        Ok(config) => config,
        Err(error) => {
            finish_failed_start(error, startup_tx, stopped_tx, &status_tx);
            return;
        }
    };
    let runtime = match PreferredRuntime::create() {
        Ok(runtime) => runtime,
        Err(error) => {
            finish_failed_start(
                Error::CreateRuntime(error.to_string()),
                startup_tx,
                stopped_tx,
                &status_tx,
            );
            return;
        }
    };

    let setup = runtime.block_on(async {
        let bind_address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let listener = runtime
            .listen(&bind_address, &Default::default())
            .await
            .map_err(|error| Error::BindListener(error.to_string()))?;
        let endpoint =
            listener.local_addr().map_err(|error| Error::ReadListenerAddress(error.to_string()))?;
        let client = TorClient::with_runtime(runtime.clone())
            .config(config)
            .bootstrap_behavior(BootstrapBehavior::OnDemand)
            .create_unbootstrapped_async()
            .await
            .map_err(|error| Error::CreateClient(error.to_string()))?;

        publish_bootstrap_status(&status_tx, &client.bootstrap_status());

        Ok::<_, Error>((client, listener, endpoint))
    });
    let (client, listener, endpoint) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            finish_failed_start(error, startup_tx, stopped_tx, &status_tx);
            return;
        }
    };

    debug!(?endpoint, "built-in Tor SOCKS listener bound");
    if startup_tx.send(Ok(endpoint)).is_err() {
        status_tx.send_replace(Status::Stopped);
        let _ = stopped_tx.send(Ok(()));
        return;
    }

    let result = runtime.block_on(async {
        let proxy_client = client.isolated_client();
        let proxy = arti::proxy::run_proxy_with_listeners(
            proxy_client,
            vec![listener],
            ListenProtocols::SocksOnly,
            None,
        );

        let mut bootstrap_events = client.bootstrap_events();
        let status_tx_for_events = status_tx.clone();
        let status_watcher = async move {
            while let Some(status) = bootstrap_events.next().await {
                publish_bootstrap_status(&status_tx_for_events, &status);
            }

            Err::<(), Error>(Error::BootstrapStatusStreamClosed)
        };

        let bootstrap_client = client.clone();
        let status_tx_for_bootstrap = status_tx.clone();
        let bootstrap = async move {
            bootstrap_client
                .bootstrap()
                .await
                .map_err(|error| Error::Bootstrap(error.to_string()))?;
            publish_bootstrap_status(
                &status_tx_for_bootstrap,
                &bootstrap_client.bootstrap_status(),
            );

            pending::<Result<(), Error>>().await
        };

        tokio::select! {
            proxy_result = proxy => match proxy_result {
                Ok(()) => Err(Error::ProxyStopped),
                Err(error) => Err(Error::Proxy(error.to_string())),
            },
            bootstrap_result = bootstrap => bootstrap_result,
            status_result = status_watcher => status_result,
            () = wait_for_shutdown(shutdown_rx) => Ok(()),
        }
    });

    match &result {
        Ok(()) => {
            status_tx.send_replace(Status::Stopped);
            debug!("built-in Tor runtime stopped");
        }
        Err(runtime_error) => {
            status_tx.send_replace(Status::Failed(runtime_error.clone()));
            error!("built-in Tor runtime stopped with an error");
        }
    }

    let _ = stopped_tx.send(result);
}

async fn wait_for_shutdown(mut shutdown_rx: watch::Receiver<bool>) {
    loop {
        if *shutdown_rx.borrow() {
            return;
        }

        if shutdown_rx.changed().await.is_err() {
            return;
        }
    }
}

fn finish_failed_start(
    error: Error,
    startup_tx: oneshot::Sender<Result<SocketAddr, Error>>,
    stopped_tx: oneshot::Sender<Result<(), Error>>,
    status_tx: &watch::Sender<Status>,
) {
    status_tx.send_replace(Status::Failed(error.clone()));
    let _ = startup_tx.send(Err(error.clone()));
    let _ = stopped_tx.send(Err(error));
}

fn build_tor_client_config() -> Result<TorClientConfig, Error> {
    let tor_root = ROOT_DATA_DIR.join("tor");
    let state_dir = tor_root.join("state");
    let cache_dir = tor_root.join("cache");

    create_directory(&state_dir)?;
    create_directory(&cache_dir)?;

    TorClientConfigBuilder::from_directories(&state_dir, &cache_dir)
        .build()
        .map_err(|error| Error::Configure(error.to_string()))
}

fn create_directory(path: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(path).map_err(|error| Error::CreateDirectory {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn publish_bootstrap_status(status_tx: &watch::Sender<Status>, status: &BootstrapStatus) {
    if status.ready_for_traffic() {
        status_tx.send_replace(Status::Ready);
        return;
    }

    let percent = (status.as_frac() * 100.0).round().clamp(0.0, 100.0) as u8;
    let blocked = status.blocked().map(|blockage| match blockage.kind() {
        BlockageKind::Disabled => BootstrapBlockage::Disabled,
        BlockageKind::Offline => BootstrapBlockage::Offline,
        BlockageKind::Filtering => BootstrapBlockage::Filtering,
        BlockageKind::CantReachTor => BootstrapBlockage::CannotReachTor,
        BlockageKind::ClockSkewed => BootstrapBlockage::ClockSkewed,
        BlockageKind::CantBootstrap => BootstrapBlockage::CannotBootstrapDirectory,
        _ => BootstrapBlockage::Unknown,
    });

    status_tx.send_replace(Status::Bootstrapping(BootstrapProgress { percent, blocked }));
}
