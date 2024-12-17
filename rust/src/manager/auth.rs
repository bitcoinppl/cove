use std::sync::{Arc, LazyLock};

use crossbeam::channel::{Receiver, Sender};
use macros::impl_default_for;
use parking_lot::RwLock;
use tap::TapFallible as _;
use tracing::{debug, error};

use crate::{
    auth::{AuthPin, AuthType},
    database::{self, Database},
};

type Message = AuthManagerReconcileMessage;

pub static AUTH_MANAGER: LazyLock<Arc<RustAuthManager>> = LazyLock::new(RustAuthManager::init);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AuthManagerReconcileMessage {
    AuthTypeChanged(AuthType),
    WipeDataPinChanged,
}

#[uniffi::export(callback_interface)]
pub trait AuthManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: AuthManagerReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustAuthManager {
    #[allow(dead_code)]
    pub state: Arc<RwLock<AuthManagerState>>,
    pub reconciler: Sender<AuthManagerReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<AuthManagerReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AuthManagerState {}

type Action = AuthManagerAction;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AuthManagerAction {
    UpdateAuthType(AuthType),
    EnableBiometric,
    DisableBiometric,
    DisablePin,
    SetPin(String),
    DisableWipeDataPin,
}

type Result<T, E = Error> = std::result::Result<T, E>;
type Error = AuthManagerError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum AuthManagerError {
    #[error("Unable to set the wipe data PIN, because {0}")]
    WipeDataSet(#[from] WipeDataPinError),

    #[error("There was a database error: {0}")]
    DatabaseError(#[from] database::Error),
}

#[uniffi::export]
fn auth_manager_error_to_string(error: AuthManagerError) -> String {
    error.to_string()
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WipeDataPinError {
    /// Unable to set wipe data pin, because PIN is not enabled
    #[error("PIN is not enabled")]
    PinNotEnabled,

    /// Unable to set wipe data pin, because its the same as the current pin
    #[error("its is the same as the current pin")]
    SameAsCurrentPin,

    /// Unable to set wipe data pin, because biometrics is enabled
    #[error("biometrics is enabled")]
    BiometricsEnabled,
}

impl RustAuthManager {
    fn init() -> Arc<Self> {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(AuthManagerState::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
        .into()
    }
}

#[uniffi::export]
impl RustAuthManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        AUTH_MANAGER.clone()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AuthManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                // call the reconcile method on the frontend
                reconciler.reconcile(field);
            }
        });
    }

    /// Get the auth type for the app
    pub fn auth_type(&self) -> AuthType {
        Database::global()
            .global_config
            .auth_type()
            .tap_err(|error| {
                error!("unable to get auth type: {error:?}");
            })
            .unwrap_or_default()
    }

    /// Check if the wipe data pin is enabled
    pub fn is_wipe_data_pin_enabled(&self) -> bool {
        let pin = Database::global()
            .global_config
            .wipe_data_pin()
            .unwrap_or_default();

        !pin.is_empty()
    }

    /// Set the wipe data pin
    pub fn set_wipe_data_pin(&self, pin: String) -> Result<()> {
        let auth_type = self.auth_type();

        if auth_type == AuthType::None {
            return Err(WipeDataPinError::PinNotEnabled.into());
        }

        if auth_type == AuthType::Biometric || auth_type == AuthType::Both {
            return Err(WipeDataPinError::BiometricsEnabled.into());
        }

        if AuthPin::new().check(&pin) {
            return Err(WipeDataPinError::SameAsCurrentPin.into());
        }

        // set the pin
        Database::global().global_config.set_wipe_data_pin(pin)?;
        self.send(Message::WipeDataPinChanged);

        Ok(())
    }

    /// Check to see if the passed in PIN matches the wipe data PIN
    pub fn check_wipe_data_pin(&self, pin: String) -> bool {
        if pin.is_empty() {
            return false;
        }

        let wipe_data_pin = Database::global()
            .global_config
            .wipe_data_pin()
            .unwrap_or_default();

        pin == wipe_data_pin
    }

    /// Delete the wipe data pin
    pub fn delete_wipe_data_pin(&self) {
        if let Err(error) = Database::global().global_config.delete_wipe_data_pin() {
            error!("unable to delete wipe data pin: {error:?}");
        }

        self.send(Message::WipeDataPinChanged);
    }

    // private
    fn set_auth_type(&self, auth_type: AuthType) {
        match Database::global().global_config.set_auth_type(auth_type) {
            Ok(_) => {
                self.send(Message::AuthTypeChanged(auth_type));
            }
            Err(error) => {
                error!("unable to set auth type: {error:?}");
            }
        }
    }

    fn send(&self, message: Message) {
        if let Err(error) = self.reconciler.send(message) {
            error!("unable to send message: {error:?}");
        }
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: AuthManagerAction) {
        match action {
            Action::UpdateAuthType(auth_type) => {
                debug!("authType changed, new: {auth_type:?}");
                self.set_auth_type(auth_type);
            }

            Action::EnableBiometric => {
                debug!("enable biometric");

                match self.auth_type() {
                    AuthType::None => self.set_auth_type(AuthType::Biometric),
                    AuthType::Pin => self.set_auth_type(AuthType::Both),
                    _ => {}
                };
            }

            Action::DisableBiometric => {
                debug!("disable biometric");

                match self.auth_type() {
                    AuthType::Biometric => self.set_auth_type(AuthType::None),
                    AuthType::Both => self.set_auth_type(AuthType::Pin),
                    _ => {}
                };
            }

            Action::SetPin(pin) => {
                debug!("set pin");

                if let Err(err) = AuthPin::new().set(pin) {
                    return error!("unable to set pin: {err:?}");
                }

                match self.auth_type() {
                    AuthType::None => self.set_auth_type(AuthType::Pin),
                    AuthType::Biometric => self.set_auth_type(AuthType::Both),
                    _ => {}
                }
            }

            Action::DisablePin => {
                debug!("disable pin");

                if let Err(err) = AuthPin::new().delete() {
                    return error!("unable to delete pin: {err:?}");
                }

                match self.auth_type() {
                    AuthType::Pin => self.set_auth_type(AuthType::None),
                    AuthType::Both => self.set_auth_type(AuthType::Biometric),
                    _ => {}
                }
            }

            Action::DisableWipeDataPin => self.delete_wipe_data_pin(),
        }
    }
}

impl_default_for!(AuthManagerState);
impl AuthManagerState {
    pub fn new() -> Self {
        Self {}
    }
}
