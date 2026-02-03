use std::sync::Arc;

use once_cell::sync::OnceCell;

#[uniffi::export(callback_interface)]
pub trait DeviceAccess: Send + Sync + std::fmt::Debug + 'static {
    fn timezone(&self) -> String;
}

static REF: OnceCell<Device> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct Device(Arc<Box<dyn DeviceAccess>>);

impl Device {
    /// Returns the global device instance
    ///
    /// # Panics
    ///
    /// Panics if the device has not been initialized
    pub fn global() -> &'static Self {
        REF.get().expect("device is not initialized")
    }

    #[must_use]
    pub fn timezone(&self) -> String {
        self.0.timezone()
    }
}

#[uniffi::export]
impl Device {
    /// Creates a new global device instance
    ///
    /// # Panics
    ///
    /// Panics if the device has already been initialized
    #[uniffi::constructor]
    pub fn new(device: Box<dyn DeviceAccess>) -> Self {
        if let Some(me) = REF.get() {
            tracing::warn!("device is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(device));
        REF.set(me).expect("failed to set keychain");

        Self::global().clone()
    }
}
