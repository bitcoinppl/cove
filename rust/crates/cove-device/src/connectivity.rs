use std::sync::Arc;

use once_cell::sync::OnceCell;

static REF: OnceCell<Connectivity> = OnceCell::new();

#[uniffi::export(callback_interface)]
pub trait ConnectivityAccess: Send + Sync + std::fmt::Debug + 'static {
    fn is_connected(&self) -> bool;
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct Connectivity(Arc<Box<dyn ConnectivityAccess>>);

impl Connectivity {
    pub fn try_global() -> Option<&'static Self> {
        REF.get()
    }

    pub fn global() -> &'static Self {
        Self::try_global().expect("connectivity is not initialized")
    }

    pub fn is_connected(&self) -> bool {
        self.0.is_connected()
    }
}

#[uniffi::export]
impl Connectivity {
    #[uniffi::constructor]
    pub fn new(connectivity: Box<dyn ConnectivityAccess>) -> Self {
        REF.get_or_init(|| Self(Arc::new(connectivity))).clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[derive(Debug)]
    struct TestConnectivity(bool);

    impl ConnectivityAccess for TestConnectivity {
        fn is_connected(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn concurrent_initialization_returns_the_same_singleton() {
        let barrier = Arc::new(Barrier::new(3));
        let connected_barrier = Arc::clone(&barrier);
        let disconnected_barrier = Arc::clone(&barrier);

        let connected = std::thread::spawn(move || {
            connected_barrier.wait();
            Connectivity::new(Box::new(TestConnectivity(true)))
        });
        let disconnected = std::thread::spawn(move || {
            disconnected_barrier.wait();
            Connectivity::new(Box::new(TestConnectivity(false)))
        });

        barrier.wait();

        let connected = connected.join().expect("join connected initializer");
        let disconnected = disconnected.join().expect("join disconnected initializer");
        let global = Connectivity::global();

        assert_eq!(connected.is_connected(), global.is_connected());
        assert_eq!(disconnected.is_connected(), global.is_connected());
    }
}
