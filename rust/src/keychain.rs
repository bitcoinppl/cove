//! Module for interacting with the secure element

use std::sync::Arc;

use log::warn;
use once_cell::sync::OnceCell;

#[uniffi::export(callback_interface)]
pub trait Keychain: Send + Sync + std::fmt::Debug + 'static {
    fn encrypt(&self, data: Vec<u8>) -> Result<Vec<u8>, String>;
}

static REF: OnceCell<Authenticator> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct Authenticator(Arc<Box<dyn Keychain>>);

#[uniffi::export]
impl Authenticator {
    #[uniffi::constructor]
    pub fn new(keychain: Box<dyn Keychain>) -> Self {
        if let Some(me) = REF.get() {
            warn!("keychain is already");
            return me.clone();
        }

        Self(Arc::new(keychain))
    }
}

impl Authenticator {
    pub fn global() -> &'static Self {
        REF.get().expect("keychain is not initialized")
    }

    pub fn encrypt(&self, data: Vec<u8>) -> Result<Vec<u8>, String> {
        self.0.encrypt(data)
    }
}
