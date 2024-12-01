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
    pub fn global() -> &'static Device {
        REF.get().expect("device is not initialized")
    }

    pub fn timezone(&self) -> String {
        self.0.timezone()
    }
}

#[uniffi::export]
impl Device {
    #[uniffi::constructor]
    pub fn new(device: Box<dyn DeviceAccess>) -> Self {
        if let Some(me) = REF.get() {
            tracing::warn!("device is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(device));
        REF.set(me).expect("failed to set keychain");

        Device::global().clone()
    }
}
