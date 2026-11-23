//! The Tor settings screen's manager

use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use cove_http::ProxyAddress;
use serde::Deserialize;
use tracing::warn;

use crate::{
    manager::reconcile_channel::ReconcileChannel,
    tor::{SetTorError, TorMode, TorService, TorSettings, TorSettingsError, TorStatus},
};

/// Tor's standard SOCKS port, which Orbot on Android also uses
const DEFAULT_PROXY_ADDRESS: &str = "127.0.0.1:9050";

/// Asked through Tor to confirm a proxy really leads into Tor
const TOR_CHECK_URL: &str = "https://check.torproject.org/api/ip";

pub static TOR_MANAGER: LazyLock<Arc<RustTorManager>> = LazyLock::new(RustTorManager::init);

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TorManagerState {
    /// The saved mode, `None` when the saved setting can't be read
    pub mode: Option<TorMode>,
    pub status: TorStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TorManagerAction {
    /// Save and apply a mode
    SelectMode(TorMode),
    /// Start Tor again with the saved mode, such as after the built-in client failed
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TorManagerReconcileMessage {
    Mode(Option<TorMode>),
    Status(TorStatus),
    /// A mode change was refused, and the saved mode is unchanged
    ModeRejected(TorManagerError),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum TorManagerError {
    #[error("{0}")]
    InvalidProxy(String),

    #[error("{node_name} is a .onion node, which only works over Tor. Choose another node first")]
    OnionNodeSelected { node_name: String },

    #[error("could not save the Tor setting: {0}")]
    Save(String),

    #[error("Tor is off")]
    TorOff,

    #[error("could not reach the Tor check service: {0}")]
    CheckFailed(String),
}

/// The answer from the Tor Project's check service
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TorConnectionCheck {
    /// Whether the request left the network from a Tor exit relay
    pub is_tor: bool,
}

#[uniffi::export(callback_interface)]
pub trait TorManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: TorManagerReconcileMessage);
}

crate::manager::reconcile_channel::impl_reconcile_sink!(
    dyn TorManagerReconciler,
    TorManagerReconcileMessage
);

#[derive(Debug, Clone, uniffi::Object)]
pub struct RustTorManager {
    reconciler: ReconcileChannel<TorManagerReconcileMessage>,
    forwarding: Arc<AtomicBool>,
    /// Changes the user asked for, applied one at a time in the order they were dispatched
    changes: flume::Sender<TorChange>,
}

/// A change waiting for the Tor service
#[derive(Debug)]
enum TorChange {
    Select(TorSettings),
    Restart,
}

impl RustTorManager {
    fn init() -> Arc<Self> {
        let reconciler = ReconcileChannel::new(100);
        let (changes, queued) = flume::unbounded();

        // saving writes the database and starting built-in Tor opens its files, so changes run
        // off the caller's thread; one thread runs them in order, so Off then Built-in never
        // lands as Off
        let rejections = reconciler.clone();
        std::thread::Builder::new()
            .name("tor-settings".into())
            .spawn(move || apply_changes(&queued, &rejections))
            .expect("the Tor settings thread starts");

        Arc::new(Self { reconciler, forwarding: Arc::new(AtomicBool::new(false)), changes })
    }

    /// Sends every mode and status change to the reconciler, from the first listener on
    ///
    /// The mode comes from what the service applied, so a change it makes on its own, such as
    /// turning Tor off after a full wipe, reaches the screen too
    fn forward_changes(&self) {
        if self.forwarding.swap(true, Ordering::AcqRel) {
            return;
        }

        let mut state = TorService::global().subscribe();
        let reconciler = self.reconciler.clone();

        // the baseline is read here, before the listener's own snapshot, so a change between
        // the two is still sent
        let (mut mode, mut status) = {
            let current = state.borrow_and_update();
            (current.settings.as_ref().map(TorMode::from), current.status.clone())
        };

        cove_tokio::task::spawn(async move {
            while state.changed().await.is_ok() {
                let (next_mode, next_status) = {
                    let current = state.borrow_and_update();
                    (current.settings.as_ref().map(TorMode::from), current.status.clone())
                };

                if next_mode != mode {
                    mode = next_mode;
                    reconciler.send(TorManagerReconcileMessage::Mode(mode.clone()));
                }

                if next_status != status {
                    status = next_status;
                    reconciler.send(TorManagerReconcileMessage::Status(status.clone()));
                }
            }
        });
    }

    fn select_mode(&self, mode: TorMode) {
        let settings = match TorSettings::try_from(mode) {
            Ok(settings) => settings,
            Err(error) => {
                self.reconciler.send(TorManagerReconcileMessage::ModeRejected(error.into()));
                return;
            }
        };

        self.queue(TorChange::Select(settings));
    }

    fn queue(&self, change: TorChange) {
        // the receiver lives on its own thread for the whole process
        if self.changes.send(change).is_err() {
            warn!("the Tor settings thread stopped, dropping a change");
        }
    }
}

/// Applies queued changes in order; the saved mode reaches the screen through the service's
/// published state, and only a refusal is sent from here
fn apply_changes(
    queued: &flume::Receiver<TorChange>,
    reconciler: &ReconcileChannel<TorManagerReconcileMessage>,
) {
    for change in queued.iter() {
        match change {
            TorChange::Select(settings) => {
                if let Err(error) = TorService::global().set_settings(settings) {
                    reconciler.send(TorManagerReconcileMessage::ModeRejected(error.into()));
                }
            }

            TorChange::Restart => TorService::global().restart(),
        }
    }
}

#[uniffi::export]
impl RustTorManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        TOR_MANAGER.clone()
    }

    /// The initial snapshot; later changes arrive as reconcile messages
    pub fn state(&self) -> TorManagerState {
        let state = TorService::global().subscribe().borrow().clone();
        TorManagerState { mode: state.settings.as_ref().map(TorMode::from), status: state.status }
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn TorManagerReconciler>) {
        self.reconciler.listen_sink(reconciler);
        self.forward_changes();
    }

    pub fn dispatch(&self, action: TorManagerAction) {
        match action {
            TorManagerAction::SelectMode(mode) => self.select_mode(mode),
            TorManagerAction::Restart => self.queue(TorChange::Restart),
        }
    }

    /// The proxy address the form starts with
    pub fn default_proxy_address(&self) -> String {
        DEFAULT_PROXY_ADDRESS.to_string()
    }

    /// Checks a typed proxy address, returning it in the form the app saves
    pub fn validate_proxy_address(&self, input: String) -> Result<String, TorManagerError> {
        let address = ProxyAddress::parse(&input)
            .map_err(|error| TorManagerError::InvalidProxy(error.to_string()))?;
        Ok(address.to_string())
    }

    /// Asks the Tor Project's check service, over Tor, whether traffic leaves through Tor
    ///
    /// Useful for a proxy, which Cove can't otherwise tell apart from a SOCKS proxy that isn't
    /// Tor. The request only ever goes over the Tor route
    pub async fn check_connection(&self) -> Result<TorConnectionCheck, TorManagerError> {
        let route = crate::tor::current_route()
            .map_err(|error| TorManagerError::CheckFailed(error.to_string()))?;
        if !route.is_tor() {
            return Err(TorManagerError::TorOff);
        }

        let client = cove_http::new_client(&route)
            .map_err(|error| TorManagerError::CheckFailed(error.to_string()))?;
        let response = client
            .get(TOR_CHECK_URL)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| {
                warn!("Tor check failed: {error}");
                TorManagerError::CheckFailed(error.without_url().to_string())
            })?;

        #[derive(Deserialize)]
        struct CheckResponse {
            #[serde(rename = "IsTor")]
            is_tor: bool,
        }

        let check: CheckResponse = response
            .json()
            .await
            .map_err(|error| TorManagerError::CheckFailed(error.without_url().to_string()))?;

        Ok(TorConnectionCheck { is_tor: check.is_tor })
    }
}

impl From<TorSettingsError> for TorManagerError {
    fn from(error: TorSettingsError) -> Self {
        Self::InvalidProxy(error.to_string())
    }
}

impl From<SetTorError> for TorManagerError {
    fn from(error: SetTorError) -> Self {
        match error {
            SetTorError::OnionNodeSelected { node_name } => Self::OnionNodeSelected { node_name },
            SetTorError::Save(message) => Self::Save(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use parking_lot::Mutex;

    use super::{RustTorManager, TorManagerReconcileMessage, TorManagerReconciler, TorMode};
    use crate::{
        database::Database,
        test_support::socks_proxy::{TorTest, closed_proxy_address},
        tor::{TorService, TorSettings},
    };

    #[derive(Debug, Default)]
    struct Recorder(Arc<Mutex<Vec<TorManagerReconcileMessage>>>);

    impl TorManagerReconciler for Recorder {
        fn reconcile(&self, message: TorManagerReconcileMessage) {
            self.0.lock().push(message);
        }
    }

    async fn wait_for(
        messages: &Mutex<Vec<TorManagerReconcileMessage>>,
        expected: &TorManagerReconcileMessage,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !messages.lock().contains(expected) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "never received {expected:?}, got {:?}",
                messages.lock()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // a full wipe turns Tor off by reloading the wiped settings, not through the screen, so the
    // screen has to hear the new mode from the service
    #[tokio::test]
    async fn the_screen_hears_a_mode_the_service_reloads() {
        let tor = TorTest::start().await;
        let messages = Arc::new(Mutex::new(Vec::new()));
        RustTorManager::new().listen_for_updates(Box::new(Recorder(messages.clone())));

        let proxy = closed_proxy_address();
        tor.use_proxy(proxy.clone());
        let proxy_mode = TorMode::Proxy { address: proxy.to_string() };
        wait_for(&messages, &TorManagerReconcileMessage::Mode(Some(proxy_mode))).await;

        Database::global().global_config.set_tor_settings(&TorSettings::Off).unwrap();
        TorService::global().restart();
        wait_for(&messages, &TorManagerReconcileMessage::Mode(Some(TorMode::Off))).await;
    }
}
