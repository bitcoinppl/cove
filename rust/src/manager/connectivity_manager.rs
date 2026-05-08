use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicU8, Ordering},
};

use flume::{Receiver, Sender, TrySendError};
use parking_lot::Mutex;

pub static CONNECTIVITY_MANAGER: LazyLock<Arc<RustConnectivityManager>> =
    LazyLock::new(RustConnectivityManager::init);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ConnectivityStatus {
    Unknown,
    Connected,
    Disconnected,
}

impl ConnectivityStatus {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Connected => 1,
            Self::Disconnected => 2,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Connected,
            2 => Self::Disconnected,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ConnectivityState {
    pub status: ConnectivityStatus,
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustConnectivityManager {
    status: Arc<AtomicU8>,
    subscribers: Arc<Mutex<Vec<Sender<()>>>>,
}

impl RustConnectivityManager {
    fn init() -> Arc<Self> {
        Arc::new(Self {
            status: Arc::new(AtomicU8::new(ConnectivityStatus::Unknown.as_u8())),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub(crate) fn connection_status(&self) -> ConnectivityStatus {
        ConnectivityStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    pub(crate) fn connected(&self) -> bool {
        self.connection_status() == ConnectivityStatus::Connected
    }

    pub(crate) fn known_disconnected(&self) -> bool {
        self.connection_status() == ConnectivityStatus::Disconnected
    }

    pub(crate) fn subscribe(&self) -> Receiver<()> {
        let (sender, receiver) = flume::bounded(1);
        self.subscribers.lock().push(sender);
        receiver
    }

    fn set_connection_status_internal(&self, status: ConnectivityStatus) -> bool {
        let previous = self.status.swap(status.as_u8(), Ordering::AcqRel);
        if previous == status.as_u8() {
            return false;
        }

        self.broadcast();
        true
    }

    fn broadcast(&self) {
        let mut subscribers = self.subscribers.lock();
        subscribers.retain(|sender| match sender.try_send(()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
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
        ConnectivityState { status: self.connection_status() }
    }

    pub fn is_connected(&self) -> bool {
        self.connected()
    }

    pub fn set_connection_status(&self, status: ConnectivityStatus) {
        self.set_connection_status_internal(status);
    }

    pub fn set_connection_state(&self, is_connected: bool) {
        let status = if is_connected {
            ConnectivityStatus::Connected
        } else {
            ConnectivityStatus::Disconnected
        };
        self.set_connection_status(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_receives_changes() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();

        manager.set_connection_status(ConnectivityStatus::Connected);

        receiver.recv().unwrap();
        assert_eq!(manager.connection_status(), ConnectivityStatus::Connected);
    }

    #[test]
    fn subscribe_ignores_unchanged_value() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();

        manager.set_connection_status(ConnectivityStatus::Unknown);

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn initial_status_is_unknown() {
        let manager = RustConnectivityManager::init();

        assert_eq!(manager.connection_status(), ConnectivityStatus::Unknown);
        assert!(!manager.connected());
        assert!(!manager.known_disconnected());
    }

    #[test]
    fn broadcast_keeps_full_subscribers_registered() {
        let manager = RustConnectivityManager::init();
        let (full_sender, full_receiver) = flume::bounded(1);

        full_sender.send(()).expect("fill subscriber channel");
        manager.subscribers.lock().push(full_sender);

        manager.broadcast();

        assert_eq!(manager.subscribers.lock().len(), 1);
        full_receiver.recv().unwrap();

        manager.broadcast();

        full_receiver.recv().unwrap();
    }

    #[test]
    fn subscribe_coalesces_multiple_changes_and_uses_latest_state() {
        let manager = RustConnectivityManager::init();
        let receiver = manager.subscribe();

        manager.status.store(ConnectivityStatus::Disconnected.as_u8(), Ordering::Release);
        manager.set_connection_status_internal(ConnectivityStatus::Connected);
        manager.set_connection_status_internal(ConnectivityStatus::Disconnected);

        receiver.recv().unwrap();

        assert!(receiver.try_recv().is_err());
        assert_eq!(manager.connection_status(), ConnectivityStatus::Disconnected);
    }

    #[test]
    fn broadcast_drops_disconnected_subscribers() {
        let manager = RustConnectivityManager::init();
        let (healthy_sender, healthy_receiver) = flume::bounded(1);
        let (disconnected_sender, disconnected_receiver) = flume::bounded(1);

        drop(disconnected_receiver);

        {
            let mut subscribers = manager.subscribers.lock();
            subscribers.push(healthy_sender);
            subscribers.push(disconnected_sender);
        }

        manager.broadcast();

        healthy_receiver.recv().unwrap();
        assert_eq!(manager.subscribers.lock().len(), 1);
    }
}
