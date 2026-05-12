use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use futures::{FutureExt as _, StreamExt as _};
use tokio::{
    net::TcpStream,
    sync::watch,
    time::{Duration, sleep},
};

use arti::proxy::ListenProtocols;
use arti_client::{
    BootstrapBehavior, TorClient, TorClientConfig, config::TorClientConfigBuilder,
    status::BootstrapStatus,
};
use once_cell::sync::Lazy;
use parking_lot::{Condvar, Mutex};
use tor_config::Listen;
use tor_rtcompat::{NetStreamListener, NetStreamProvider, PreferredRuntime, ToplevelBlockOn};
use tracing::{debug, error, info, warn};

use crate::BuiltInTorBootstrapStatus;
use cove_common::consts::ROOT_DATA_DIR;

const BUILT_IN_TOR_SOCKS_PORT: u16 = 39050;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize built-in tor proxy: {0}")]
    Proxy(String),
}

#[derive(Debug, Clone)]
struct BuiltInTorState {
    endpoint: Option<SocketAddr>,
    launched: bool,
    last_error: Option<String>,
    shutdown_tx: Option<watch::Sender<bool>>,
    bootstrap_status: BuiltInTorBootstrapStatus,
}

#[derive(Debug, Clone)]
struct BuiltInTorPaths {
    state_dir: PathBuf,
    cache_dir: PathBuf,
}

static BUILT_IN_TOR_STATE: Lazy<Mutex<BuiltInTorState>> = Lazy::new(|| {
    Mutex::new(BuiltInTorState {
        endpoint: None,
        launched: false,
        last_error: None,
        shutdown_tx: None,
        bootstrap_status: default_bootstrap_status(false, None),
    })
});
static BUILT_IN_TOR_STOPPED: Lazy<Condvar> = Lazy::new(Condvar::new);

fn clear_built_in_state(reason: &str) {
    let mut state = BUILT_IN_TOR_STATE.lock();
    state.endpoint = None;
    state.launched = false;
    state.shutdown_tx = None;
    state.bootstrap_status = default_bootstrap_status(false, state.last_error.clone());
    warn!(%reason, "cleared built-in tor state");
    BUILT_IN_TOR_STOPPED.notify_all();
}

fn set_built_in_error(error: String) {
    let mut state = BUILT_IN_TOR_STATE.lock();
    state.last_error = Some(error.clone());
    state.bootstrap_status = default_bootstrap_status(state.launched, Some(error.clone()));
    warn!(%error, "recorded built-in tor runtime error");
}

fn take_built_in_error() -> Option<String> {
    let mut state = BUILT_IN_TOR_STATE.lock();
    state.last_error.take()
}

pub(crate) fn built_in_status_summary() -> String {
    let state = BUILT_IN_TOR_STATE.lock();
    let last_error = if state.last_error.is_some() { "present" } else { "none" };

    format!("launched={}, last_error={last_error}", state.launched)
}

pub(crate) fn built_in_bootstrap_status() -> BuiltInTorBootstrapStatus {
    let state = BUILT_IN_TOR_STATE.lock();
    let mut status = state.bootstrap_status.clone();
    status.launched = state.launched;
    status.last_error = state.last_error.clone();
    status
}

fn default_bootstrap_status(
    launched: bool,
    last_error: Option<String>,
) -> BuiltInTorBootstrapStatus {
    BuiltInTorBootstrapStatus {
        percent: if launched { 1 } else { 0 },
        ready: false,
        blocked: last_error.clone(),
        message: if launched {
            "Starting Tor".to_string()
        } else {
            "Built-in Tor is not running".to_string()
        },
        launched,
        last_error,
    }
}

fn set_bootstrap_status(status: &BootstrapStatus) {
    let percent = (status.as_frac() * 100.0).round().clamp(0.0, 100.0) as u32;
    let blocked = status.blocked().map(|blockage| blockage.to_string());
    let mut state = BUILT_IN_TOR_STATE.lock();
    state.bootstrap_status = BuiltInTorBootstrapStatus {
        percent,
        ready: status.ready_for_traffic(),
        blocked,
        message: status.to_string(),
        launched: state.launched,
        last_error: state.last_error.clone(),
    };
}

pub(crate) fn request_stop_built_in_proxy() -> bool {
    let shutdown_tx = {
        let state = BUILT_IN_TOR_STATE.lock();
        state.shutdown_tx.clone()
    };

    match shutdown_tx {
        Some(tx) => {
            if tx.send(true).is_err() {
                warn!("failed to request built-in tor shutdown: runtime channel closed");
                return wait_for_built_in_proxy_stopped();
            }
            info!("requested built-in tor shutdown");
            wait_for_built_in_proxy_stopped()
        }
        None => {
            debug!("built-in tor shutdown requested but runtime is not active");
            true
        }
    }
}

fn wait_for_built_in_proxy_stopped() -> bool {
    const STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let started_at = std::time::Instant::now();
    let mut state = BUILT_IN_TOR_STATE.lock();
    loop {
        if !state.launched && state.endpoint.is_none() {
            info!("built-in tor proxy stopped");
            return true;
        }

        let Some(remaining) = STOP_TIMEOUT.checked_sub(started_at.elapsed()) else {
            warn!("timed out waiting for built-in tor proxy to stop");
            return false;
        };

        if BUILT_IN_TOR_STOPPED.wait_for(&mut state, remaining).timed_out() {
            warn!("timed out waiting for built-in tor proxy to stop");
            return false;
        }
    }
}

fn built_in_tor_paths() -> BuiltInTorPaths {
    let tor_root = ROOT_DATA_DIR.join("tor");
    BuiltInTorPaths { state_dir: tor_root.join("state"), cache_dir: tor_root.join("cache") }
}

fn ensure_built_in_tor_dirs(paths: &BuiltInTorPaths) -> Result<(), Error> {
    std::fs::create_dir_all(&paths.state_dir).map_err(|error| {
        Error::Proxy(format!(
            "failed to create built-in tor state dir {}: {error}",
            paths.state_dir.display()
        ))
    })?;

    std::fs::create_dir_all(&paths.cache_dir).map_err(|error| {
        Error::Proxy(format!(
            "failed to create built-in tor cache dir {}: {error}",
            paths.cache_dir.display()
        ))
    })?;

    Ok(())
}

fn build_tor_client_config() -> Result<TorClientConfig, Error> {
    let paths = built_in_tor_paths();
    ensure_built_in_tor_dirs(&paths)?;

    info!(
        state_dir = %paths.state_dir.display(),
        cache_dir = %paths.cache_dir.display(),
        "configuring built-in tor storage directories"
    );

    TorClientConfigBuilder::from_directories(&paths.state_dir, &paths.cache_dir).build().map_err(
        |error| Error::Proxy(format!("failed to build built-in tor client config: {error}")),
    )
}

async fn wait_for_socks_listener(endpoint: SocketAddr) -> Result<(), Error> {
    const MAX_ATTEMPTS: usize = 40;
    const RETRY_DELAY_MS: u64 = 100;

    for attempt in 1..=MAX_ATTEMPTS {
        if TcpStream::connect(endpoint).await.is_ok() {
            info!(%endpoint, attempt, "built-in tor socks listener is ready");
            return Ok(());
        }

        {
            let state = BUILT_IN_TOR_STATE.lock();
            if !state.launched && state.endpoint.is_none() {
                let startup_error = state.last_error.clone().unwrap_or_else(|| {
                    "built-in tor runtime stopped before socks listener became ready".to_string()
                });
                return Err(Error::Proxy(startup_error));
            }
        }

        sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
    }

    if let Some(error) = take_built_in_error() {
        return Err(Error::Proxy(error));
    }

    Err(Error::Proxy(format!(
        "built-in tor socks listener not ready at {endpoint} after {MAX_ATTEMPTS} attempts"
    )))
}

pub async fn built_in_socks_endpoint() -> Result<SocketAddr, Error> {
    let cached_endpoint = {
        let state = BUILT_IN_TOR_STATE.lock();
        if let Some(endpoint) = state.endpoint {
            debug!(%endpoint, launched = state.launched, "built-in tor endpoint already cached");
            Some(endpoint)
        } else {
            None
        }
    };

    if let Some(endpoint) = cached_endpoint {
        wait_for_socks_listener(endpoint).await?;
        return Ok(endpoint);
    }

    info!("built-in tor endpoint requested without cache; launching proxy");
    launch_built_in_proxy().await
}

async fn launch_built_in_proxy() -> Result<SocketAddr, Error> {
    let endpoint = SocketAddr::from((Ipv4Addr::LOCALHOST, BUILT_IN_TOR_SOCKS_PORT));
    info!(
        %endpoint,
        configured_port = BUILT_IN_TOR_SOCKS_PORT,
        "resolved built-in tor endpoint"
    );

    {
        let state = BUILT_IN_TOR_STATE.lock();
        if let Some(endpoint) = state.endpoint {
            return Ok(endpoint);
        }

        if state.launched {
            return Err(Error::Proxy(
                "built-in tor launch already in progress without a ready endpoint".to_string(),
            ));
        }
    }

    let client_config = build_tor_client_config()?;
    let socks_listen = Listen::new_localhost(BUILT_IN_TOR_SOCKS_PORT);

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    {
        let mut state = BUILT_IN_TOR_STATE.lock();
        if let Some(endpoint) = state.endpoint {
            return Ok(endpoint);
        }
        if state.launched {
            return Err(Error::Proxy(
                "built-in tor launch already in progress without a ready endpoint".to_string(),
            ));
        }
        state.launched = true;
        state.endpoint = Some(endpoint);
        state.last_error = None;
        state.shutdown_tx = Some(shutdown_tx);
        state.bootstrap_status = default_bootstrap_status(true, None);
    }

    std::thread::spawn(move || {
        info!("starting built-in tor runtime thread");

        let proxy_runtime = match PreferredRuntime::create() {
            Ok(runtime) => {
                info!("built-in tor runtime created");
                runtime
            }
            Err(error) => {
                let message = format!("failed to create built-in tor runtime: {error}");
                error!("{message}");
                set_built_in_error(message);
                clear_built_in_state("runtime creation failed");
                return;
            }
        };

        let run_result = proxy_runtime.block_on(async {
            info!("creating built-in Tor client");
            let client = TorClient::with_runtime(proxy_runtime.clone())
                .config(client_config)
                .bootstrap_behavior(BootstrapBehavior::OnDemand)
                .create_unbootstrapped_async()
                .await
                .map_err(|error| format!("failed to create built-in tor client: {error}"))?;
            set_bootstrap_status(&client.bootstrap_status());

            info!("launching Arti SOCKS proxy task");
            let mut listeners = Vec::new();
            for addrs in socks_listen
                .ip_addrs()
                .map_err(|error| format!("invalid built-in tor socks listener: {error}"))?
            {
                for addr in addrs {
                    let listener = proxy_runtime
                        .listen(&addr)
                        .await
                        .map_err(|error| format!("failed to listen on built-in tor socks address {addr}: {error}"))?;
                    info!(address = ?listener.local_addr(), "Listening on built-in Tor SOCKS address");
                    listeners.push(listener);
                }
            }
            if listeners.is_empty() {
                return Err("failed to open built-in tor socks listener".to_string());
            }

            let proxy = arti::proxy::run_proxy_with_listeners(
                client.isolated_client(),
                listeners,
                ListenProtocols::SocksOnly,
                None,
            )
            .map(|result| result.map_err(|error| format!("built-in tor socks proxy exited: {error}")));

            let mut bootstrap_events = client.bootstrap_events();
            let status_watcher = async move {
                while let Some(status) = bootstrap_events.next().await {
                    set_bootstrap_status(&status);
                }
                futures::future::pending::<Result<(), String>>().await
            };

            let bootstrap_client = client.clone();
            let bootstrap = async move {
                bootstrap_client
                    .bootstrap()
                    .await
                    .map_err(|error| format!("built-in tor bootstrap failed: {error}"))?;
                let status = bootstrap_client.bootstrap_status();
                set_bootstrap_status(&status);
                info!("Sufficiently bootstrapped; proxy now functional.");
                futures::future::pending::<Result<(), String>>().await
            };

            tokio::select! {
                run_result = proxy => run_result,
                run_result = bootstrap => run_result,
                run_result = status_watcher => run_result,
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(()) => {
                            info!("built-in tor shutdown signal received");
                            Ok(())
                        }
                        Err(_) => {
                            info!("built-in tor shutdown channel closed");
                            Ok(())
                        }
                    }
                }
            }
        });

        if let Err(error) = run_result {
            let message = format!("built-in tor proxy exited: {error:?}");
            error!("{message}");
            set_built_in_error(message);
            clear_built_in_state("proxy exited with error");
            return;
        }

        warn!("built-in tor proxy task returned without error");
        clear_built_in_state("proxy task returned");
    });

    info!(%endpoint, "built-in tor launch initiated; waiting for socks listener");
    wait_for_socks_listener(endpoint).await?;
    info!(%endpoint, "built-in tor endpoint ready");
    Ok(endpoint)
}
