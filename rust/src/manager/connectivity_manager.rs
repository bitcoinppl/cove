use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use flume::{Receiver, Sender};
use parking_lot::Mutex;
use tracing::warn;

use cove_device::connectivity::Connectivity;

type ReconcilerMessage = ConnectivityManagerReconcileMessage;

pub static CONNECTIVITY_MANAGER: LazyLock<Arc<RustConnectivityManager>> =
    LazyLock::new(RustConnectivityManager::init);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ConnectivityStatus {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ConnectivityState {
    pub status: ConnectivityStatus,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ConnectivityManagerReconcileMessage {
    Status(ConnectivityStatus),
}

#[uniffi::export(callback_interface)]
pub trait ConnectivityManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: ConnectivityManagerReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustConnectivityManager {
    is_connected: Arc<AtomicBool>,
    subscribers: Arc<Mutex<Vec<Sender<bool>>>>,
    reconciler: Sender<ReconcilerMessage>,
    reconcile_receiver: Arc<Receiver<ReconcilerMessage>>,
}

impl RustConnectivityManager {
    fn init() -> Arc<Self> {
        let initial_connected = Connectivity::try_global().is_none_or(Connectivity::is_connected);
        let (sender, receiver) = flume::bounded(1000);

        Arc::new(Self {
            is_connected: Arc::new(AtomicBool::new(initial_connected)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        })
    }

    pub(crate) fn connected(&self) -> bool {
        self.is_connected.load(Ordering::Acquire)
    }

    pub(crate) fn subscribe(&self) -> Receiver<bool> {
        let (sender, receiver) = flume::bounded(16);
        self.subscribers.lock().push(sender);
        receiver
    }

    fn set_connection_state_internal(&self, is_connected: bool) -> bool {
        let previous = self.is_connected.swap(is_connected, Ordering::AcqRel);
        if previous == is_connected {
            return false;
        }

        self.broadcast(is_connected);
        self.send_reconcile(is_connected);
        true
    }

    fn broadcast(&self, is_connected: bool) {
        let mut subscribers = self.subscribers.lock();
        subscribers.retain(|sender| sender.send(is_connected).is_ok());
    }

    fn send_reconcile(&self, is_connected: bool) {
        let message = if is_connected {
            ConnectivityManagerReconcileMessage::Status(ConnectivityStatus::Connected)
        } else {
            ConnectivityManagerReconcileMessage::Status(ConnectivityStatus::Disconnected)
        };

        if let Err(error) = self.reconciler.send(message) {
            warn!("Failed to send connectivity reconcile message: {error}");
        }
    }
}

#[uniffi::export]
impl RustConnectivityManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        CONNECTIVITY_MANAGER.clone()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn ConnectivityManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(message) = reconcile_receiver.recv() {
                reconciler.reconcile(message);
            }
        });
    }

    pub fn state(&self) -> ConnectivityState {
        ConnectivityState {
            status: if self.connected() {
                ConnectivityStatus::Connected
            } else {
                ConnectivityStatus::Disconnected
            },
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected()
    }

    pub fn set_connection_state(&self, is_connected: bool) {
        self.set_connection_state_internal(is_connected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_receives_changes() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();

        manager.set_connection_state(false);

        assert_eq!(receiver.recv().unwrap(), false);
    }

    #[test]
    fn subscribe_ignores_unchanged_value() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();
        let initial = manager.connected();

        manager.set_connection_state(initial);

        assert!(receiver.try_recv().is_err());
    }
}
