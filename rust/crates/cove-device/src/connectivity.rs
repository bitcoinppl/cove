use std::sync::Arc;

use once_cell::sync::OnceCell;
use tracing::warn;

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
        if let Some(me) = REF.get() {
            warn!("connectivity is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(connectivity));
        REF.set(me).expect("failed to set connectivity");

        Self::global().clone()
    }
}
