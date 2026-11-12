use std::sync::Arc;

use once_cell::sync::OnceCell;

#[uniffi::export(callback_interface)]
pub trait DeviceAccess: Send + Sync + std::fmt::Debug + 'static {
    fn timezone(&self) -> String;

    /// Which kind of build of the app is running
    fn build_channel(&self) -> BuildChannel;
}

/// Who a build of the app was made for
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum BuildChannel {
    /// A debug build from a developer's machine
    Debug,

    /// A release build for testers, such as one installed from TestFlight
    Beta,

    /// A release build from an app store; Android reports every release build as this, since
    /// it can't tell its testing tracks apart
    Release,
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

    #[must_use]
    pub fn build_channel(&self) -> BuildChannel {
        self.0.build_channel()
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
