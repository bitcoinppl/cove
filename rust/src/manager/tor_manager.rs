//! The Tor settings screen's manager

use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use cove_http::ProxyAddress;
use cove_util::ResultExt as _;
use serde::Deserialize;
use tracing::warn;

use crate::{
    manager::reconcile_channel::ReconcileChannel,
    tor::{SetTorError, TorMode, TorService, TorSettings, TorSettingsError, TorState, TorStatus},
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
    forwarding_status: Arc<AtomicBool>,
}

impl RustTorManager {
    fn init() -> Arc<Self> {
        Arc::new(Self {
            reconciler: ReconcileChannel::new(100),
            forwarding_status: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Sends every mode and status change to the reconciler, from the first listener on
    ///
    /// The mode comes from the service rather than from this manager's own requests, so a reset
    /// the screen didn't ask for, such as a full wipe, still reaches it
    fn forward_status(&self) {
        if self.forwarding_status.swap(true, Ordering::AcqRel) {
            return;
        }

        let mut state = TorService::global().subscribe_state();
        let mut last = state.borrow_and_update().clone();
        let reconciler = self.reconciler.clone();

        cove_tokio::task::spawn(async move {
            while state.changed().await.is_ok() {
                let next = state.borrow_and_update().clone();
                if next.mode != last.mode {
                    reconciler.send(TorManagerReconcileMessage::Mode(next.mode.clone()));
                }

                if next.status != last.status {
                    reconciler.send(TorManagerReconcileMessage::Status(next.status.clone()));
                }

                last = next;
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

        // saving writes the database and starting built-in Tor opens its files, so this stays
        // off the caller's thread
        // the saved mode reaches the screen through the service's state, so only a refusal is
        // sent from here
        let reconciler = self.reconciler.clone();
        cove_tokio::task::spawn_blocking(move || {
            if let Err(error) = TorService::global().set_settings(settings) {
                reconciler.send(TorManagerReconcileMessage::ModeRejected(error.into()));
            }
        });
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
        TorService::global().state().into()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn TorManagerReconciler>) {
        self.reconciler.listen_sink(reconciler);
        self.forward_status();
    }

    pub fn dispatch(&self, action: TorManagerAction) {
        match action {
            TorManagerAction::SelectMode(mode) => self.select_mode(mode),
            TorManagerAction::Restart => {
                cove_tokio::task::spawn_blocking(|| TorService::global().restart());
            }
        }
    }

    /// The proxy address the form starts with
    pub fn default_proxy_address(&self) -> String {
        DEFAULT_PROXY_ADDRESS.to_string()
    }

    /// Checks a typed proxy address, returning it in the form the app saves
    pub fn validate_proxy_address(&self, input: String) -> Result<String, TorManagerError> {
        let address = ProxyAddress::parse(&input).map_err_str(TorManagerError::InvalidProxy)?;
        Ok(address.to_string())
    }

    /// Asks the Tor Project's check service, over Tor, whether traffic leaves through Tor
    ///
    /// Useful for a proxy, which Cove can't otherwise tell apart from a SOCKS proxy that isn't
    /// Tor. The request only ever goes over the Tor route
    pub async fn check_connection(&self) -> Result<TorConnectionCheck, TorManagerError> {
        let route = crate::tor::current_route().map_err_str(TorManagerError::CheckFailed)?;
        if !route.is_tor() {
            return Err(TorManagerError::TorOff);
        }

        let client = cove_http::new_client(&route).map_err_str(TorManagerError::CheckFailed)?;
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

impl From<TorState> for TorManagerState {
    fn from(TorState { mode, status }: TorState) -> Self {
        Self { mode, status }
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
    use std::time::Duration;

    use super::*;
    use crate::{
        database::{Database, global_config::GlobalConfigKey},
        manager::deferred_sender::SingleOrMany,
        test_support::socks_proxy::{TorTest, closed_proxy_address},
    };

    // a full wipe resets Tor underneath the manager, and the settings screen must still learn the
    // new mode, or it keeps showing the old one and ignores a tap on it
    #[tokio::test]
    async fn a_reset_mode_reaches_the_screen() {
        let tor = TorTest::start().await;
        tor.use_proxy(closed_proxy_address());

        let manager = TOR_MANAGER.clone();
        manager.forward_status();
        let receiver = manager.reconciler.receiver();
        while receiver.try_recv().is_ok() {}

        Database::global()
            .global_config
            .set(GlobalConfigKey::TorSettings, TorSettings::Off.to_json())
            .unwrap();
        TorService::global().restart();

        let expected = TorManagerReconcileMessage::Mode(Some(TorMode::Off));
        let wait = crate::test_support::loaded_host_wait(Duration::from_secs(1));
        let arrived = tokio::time::timeout(wait, async {
            while let Ok(messages) = receiver.recv_async().await {
                let messages = match messages {
                    SingleOrMany::Single(message) => vec![message],
                    SingleOrMany::Many(messages) => messages,
                };

                if messages.contains(&expected) {
                    return;
                }
            }
        })
        .await;

        assert!(arrived.is_ok(), "the reset mode never reached the screen");
    }
}
