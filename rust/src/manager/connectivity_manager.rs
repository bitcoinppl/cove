use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use flume::{Receiver, Sender, TrySendError};
use parking_lot::Mutex;

use cove_device::connectivity::Connectivity;

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

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustConnectivityManager {
    is_connected: Arc<AtomicBool>,
    subscribers: Arc<Mutex<Vec<Sender<bool>>>>,
}

impl RustConnectivityManager {
    fn init() -> Arc<Self> {
        let initial_connected = if let Some(connectivity) = Connectivity::try_global() {
            connectivity.is_connected()
        } else {
            false
        };

        Arc::new(Self {
            is_connected: Arc::new(AtomicBool::new(initial_connected)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
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
        true
    }

    fn broadcast(&self, is_connected: bool) {
        let mut subscribers = self.subscribers.lock();
        subscribers.retain(|sender| match sender.try_send(is_connected) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        });
    }
}

#[uniffi::export]
impl RustConnectivityManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        CONNECTIVITY_MANAGER.clone()
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
        let next = !manager.connected();

        manager.set_connection_state(next);

        assert_eq!(receiver.recv().unwrap(), next);
    }

    #[test]
    fn subscribe_ignores_unchanged_value() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();
        let initial = manager.connected();

        manager.set_connection_state(initial);

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn broadcast_drops_full_and_disconnected_subscribers() {
        let manager = RustConnectivityManager::init();
        let (healthy_sender, healthy_receiver) = flume::bounded(1);
        let (full_sender, _full_receiver) = flume::bounded(1);
        let (disconnected_sender, disconnected_receiver) = flume::bounded(1);

        full_sender.send(false).expect("fill subscriber channel");
        drop(disconnected_receiver);

        {
            let mut subscribers = manager.subscribers.lock();
            subscribers.push(healthy_sender);
            subscribers.push(full_sender);
            subscribers.push(disconnected_sender);
        }

        manager.broadcast(true);

        assert_eq!(healthy_receiver.recv().unwrap(), true);
        assert_eq!(manager.subscribers.lock().len(), 1);
    }
}
