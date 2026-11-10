mod app_lock;

use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use tap::TapFallible as _;
use tracing::{debug, error};

use cove_types::WalletId;

use crate::{
    app::reconcile::{AppStateReconcileMessage, Updater},
    auth::{AuthPin, AuthType},
    database::{self, Database},
    manager::reconcile_channel::ReconcileChannel,
    wallet::metadata::WalletMode,
};

pub use app_lock::{AppLockReason, SendConfirmationLock};

use app_lock::{AppLock, BootTime};

type Message = AuthManagerReconcileMessage;

pub static AUTH_MANAGER: LazyLock<Arc<RustAuthManager>> = LazyLock::new(RustAuthManager::init);

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
#[allow(clippy::enum_variant_names)] // all variants describe state changes, postfix is intentional
pub enum AuthManagerReconcileMessage {
    AuthTypeChanged(AuthType),
    /// Whether a wipe data PIN is now set
    WipeDataPinChanged(bool),
    /// Whether a decoy PIN is now set
    DecoyPinChanged(bool),
}

/// Authentication settings a frontend mirrors
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct AuthSettings {
    /// How the app is unlocked
    pub auth_type: AuthType,
    /// Whether a wipe data PIN is set
    pub is_wipe_data_pin_enabled: bool,
    /// Whether a decoy PIN is set
    pub is_decoy_pin_enabled: bool,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AuthManagerAction {
    UpdateAuthType(AuthType),
    EnableBiometric,
    DisableBiometric,
    DisablePin,
    SetPin(String),
    DisableWipeDataPin,
    DisableDecoyPin,
}

// MARK: Security Settings Validation

/// Action the user wants to take on security settings
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecuritySettingsAction {
    ToggleBiometric { enable: bool },
    TogglePin { enable: bool },
    ToggleWipeDataPin { enable: bool },
    ToggleDecoyPin { enable: bool },
    ChangePin,
}

/// What sheet to show for PIN entry flows
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecuritySheetState {
    None,
    NewPin,
    RemovePin,
    ChangePin,
    EnableBiometric,
    DisableBiometric,
    EnableWipeDataPin,
    RemoveWipeDataPin,
    /// Remove wipe data PIN, then enable biometric
    RemoveWipeDataPinThenEnableBiometric,
    EnableDecoyPin,
    RemoveDecoyPin,
    /// Remove decoy PIN, then enable biometric
    RemoveDecoyPinThenEnableBiometric,
    RemoveAllTrickPins,
}

/// What alert to show for validation messages
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecurityAlertState {
    UnverifiedWallets {
        wallet_id: WalletId,
    },
    ConfirmEnableWipeMePin,
    ConfirmDecoyPin,
    NoteNoFaceIdWhenTrickPins,
    NoteNoFaceIdWhenWipeMePin,
    NoteNoFaceIdWhenDecoyPin,
    NotePinRequired,
    /// Disabling biometric, then show confirm for wipe me PIN
    NoteFaceIdDisablingForWipeMePin,
    /// Disabling biometric, then show confirm for decoy PIN
    NoteFaceIdDisablingForDecoyPin,
    ExtraSetPinError {
        message: String,
    },
}

/// Result of validating a security settings action
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecuritySettingsResult {
    /// Proceed to show a sheet for PIN entry
    ProceedToSheet(SecuritySheetState),
    /// Show an alert dialog
    ShowAlert(SecurityAlertState),
    /// Decoy mode - just update local UI, don't persist
    DecoyModeLocalUpdate,
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustAuthManager {
    pub reconciler: ReconcileChannel<AuthManagerReconcileMessage>,
    app_lock: Arc<Mutex<AppLock>>,
}

type Action = AuthManagerAction;

type Result<T, E = Error> = std::result::Result<T, E>;
type Error = AuthManagerError;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum AuthManagerError {
    #[error("Unable to set the wipe data PIN, because {0}")]
    WipeDataSet(TrickPinError),

    #[error("Unable to set the decoy PIN, because {0}")]
    DecoySet(TrickPinError),

    #[error("There was a database error: {0}")]
    Database(#[from] database::Error),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum TrickPinError {
    /// Unable to set trick PIN, because PIN is not enabled
    #[error("PIN is not enabled")]
    PinNotEnabled,

    /// Unable to set trick PIN, because its the same as the current pin
    #[error("its is the same as the current pin")]
    SameAsCurrentPin,

    /// Unable to set trick PIN, its the same as another PIN
    #[error("its is the same as another PIN")]
    SameAsAnotherPin,

    /// Unable to set trick PIN, because biometrics is enabled
    #[error("biometrics is enabled")]
    BiometricsEnabled,
}

impl RustAuthManager {
    fn init() -> Arc<Self> {
        Self {
            reconciler: ReconcileChannel::new(1000),
            app_lock: Arc::new(Mutex::new(AppLock::locked_at_startup())),
        }
        .into()
    }

    /// Sends every setting in `settings` so frontends end on those values
    pub(crate) fn publish(&self, settings: &AuthSettings) {
        self.send(Message::AuthTypeChanged(settings.auth_type));
        self.send(Message::WipeDataPinChanged(settings.is_wipe_data_pin_enabled));
        self.send(Message::DecoyPinChanged(settings.is_decoy_pin_enabled));
    }

    pub(crate) fn settings(&self) -> AuthSettings {
        AuthSettings {
            auth_type: self.auth_type(),
            is_wipe_data_pin_enabled: self.is_wipe_data_pin_enabled(),
            is_decoy_pin_enabled: self.is_decoy_pin_enabled(),
        }
    }
}

#[uniffi::export]
impl RustAuthManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        AUTH_MANAGER.clone()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AuthManagerReconciler>) {
        self.reconciler.listen_sink(reconciler);
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

    // MARK: APP LOCK

    /// Whether any lock is held on the app
    pub fn is_locked(&self) -> bool {
        self.app_lock.lock().is_locked()
    }

    /// Lock the app until the user authenticates, returns whether the app is locked
    ///
    /// Does nothing when no authentication is set up
    pub fn lock(&self, reason: AppLockReason) -> bool {
        let is_auth_enabled = !self.is_auth_known_disabled();
        let mut app_lock = self.app_lock.lock();

        if is_auth_enabled {
            app_lock.lock(reason, BootTime::now());
        }

        app_lock.is_locked()
    }

    /// Lock the app for a send confirmation screen, returns the token that releases only
    /// this lock, or nothing when no authentication is set up
    pub fn lock_for_send_confirmation(&self) -> Option<SendConfirmationLock> {
        if self.is_auth_known_disabled() {
            return None;
        }

        Some(self.app_lock.lock().lock_for_send_confirmation(BootTime::now()))
    }

    /// Release the lock a send confirmation screen took, returns whether the app is still
    /// locked for any other reason
    pub fn release_send_confirmation_lock(&self, lock: SendConfirmationLock) -> bool {
        let mut app_lock = self.app_lock.lock();
        app_lock.release_send_confirmation(lock, BootTime::now());

        app_lock.is_locked()
    }

    /// Skip authentication when the app returns to the foreground right after leaving it,
    /// returns whether the app is still locked
    pub fn release_background_lock_on_foreground(&self) -> bool {
        let config = &Database::global().global_config;
        let settings = config.auth_type().and_then(|auth_type| {
            let decoy_pin = config.decoy_pin()?;
            Ok((auth_type, !decoy_pin.is_empty()))
        });

        let mut app_lock = self.app_lock.lock();

        // without readable settings there is no grace period to pick, so the lock stays
        // until the user authenticates
        match settings {
            Ok((auth_type, is_decoy_pin_enabled)) => app_lock.release_background_on_foreground(
                auth_type,
                is_decoy_pin_enabled,
                BootTime::now(),
            ),
            Err(error) => error!("unable to read auth settings, keeping app lock: {error:?}"),
        }

        app_lock.is_locked()
    }

    /// Clear every lock after the user authenticated with a PIN, biometrics, or a trick PIN
    pub fn unlock_authenticated(&self) {
        self.app_lock.lock().unlock_authenticated();
    }

    /// Clear every lock when no authentication is set up, returns whether the app is locked
    pub fn unlock_if_auth_disabled(&self) -> bool {
        let is_auth_disabled = self.is_auth_known_disabled();
        let mut app_lock = self.app_lock.lock();

        if is_auth_disabled {
            app_lock.unlock_authenticated();
        }

        app_lock.is_locked()
    }

    // MARK: DECOY PIN

    /// Check if decoy pin is enabled, not if the user is in decoy mode
    pub fn is_decoy_pin_enabled(&self) -> bool {
        let pin = Database::global().global_config.decoy_pin().unwrap_or_default();

        !pin.is_empty()
    }

    /// Actually check if the user is in decoy mode
    pub fn is_in_decoy_mode(&self) -> bool {
        Database::global().global_config.is_in_decoy_mode()
    }

    /// Check to see if the passed in PIN matches the decoy pin
    #[uniffi::method(name = "checkDecoyPin")]
    pub fn _check_decoy_pin(&self, pin: String) -> bool {
        self.check_decoy_pin(&pin)
    }

    /// Delete the decoy pin
    pub fn delete_decoy_pin(&self) {
        debug!("deleting decoy pin");

        if let Err(error) = Database::global().global_config.delete_decoy_pin() {
            error!("unable to delete decoy pin: {error:?}");
        }

        self.send(Message::DecoyPinChanged(self.is_decoy_pin_enabled()));
    }

    /// Set the decoy pin
    pub fn set_decoy_pin(&self, pin: String) -> Result<()> {
        self.validate_pin_settings(&pin).map_err(AuthManagerError::DecoySet)?;

        // set the pin
        Database::global().global_config.set_decoy_pin(pin)?;
        self.send(Message::DecoyPinChanged(true));

        Ok(())
    }

    /// Switch from main mode to decoy mode
    pub fn switch_to_decoy_mode(&self) {
        Database::global().global_config.set_decoy_mode().expect("failed to set decoy mode");

        Updater::send_update(AppStateReconcileMessage::WalletModeChanged(WalletMode::Decoy));
    }

    /// Switch from decoy mode to main mode
    pub fn switch_to_main_mode(&self) {
        Database::global().global_config.set_main_mode().expect("failed to set main mode");

        Updater::send_update(AppStateReconcileMessage::WalletModeChanged(WalletMode::Main));
    }

    // MARK: WIPE DATA PIN

    /// Check if the wipe data pin is enabled
    pub fn is_wipe_data_pin_enabled(&self) -> bool {
        let pin = Database::global().global_config.wipe_data_pin().unwrap_or_default();

        !pin.is_empty()
    }

    /// Set the wipe data pin
    pub fn set_wipe_data_pin(&self, pin: String) -> Result<()> {
        self.validate_pin_settings(&pin).map_err(AuthManagerError::WipeDataSet)?;

        // set the pin
        Database::global().global_config.set_wipe_data_pin(pin)?;
        self.send(Message::WipeDataPinChanged(true));

        Ok(())
    }
    /// Check to see if the passed in PIN matches the wipe data PIN
    #[uniffi::method(name = "checkWipeDataPin")]
    pub fn _check_wipe_data_pin(&self, pin: String) -> bool {
        self.check_wipe_data_pin(&pin)
    }

    /// Delete the wipe data pin
    pub fn delete_wipe_data_pin(&self) {
        debug!("deleting wipe data pin");

        if let Err(error) = Database::global().global_config.delete_wipe_data_pin() {
            error!("unable to delete wipe data pin: {error:?}");
        }

        self.send(Message::WipeDataPinChanged(self.is_wipe_data_pin_enabled()));
    }

    // private

    /// Validate if we have the correct settings to be able to set a decoy or wipe data pin
    fn validate_pin_settings(&self, pin: &str) -> Result<(), TrickPinError> {
        let auth_type = self.auth_type();

        if auth_type == AuthType::None {
            return Err(TrickPinError::PinNotEnabled);
        }

        if auth_type == AuthType::Biometric || auth_type == AuthType::Both {
            return Err(TrickPinError::BiometricsEnabled);
        }

        if AuthPin::new().check(pin) {
            return Err(TrickPinError::SameAsCurrentPin);
        }

        if self.check_decoy_pin(pin) || self.check_wipe_data_pin(pin) {
            return Err(TrickPinError::SameAsAnotherPin);
        }

        // valid
        Ok(())
    }

    fn set_auth_type(&self, auth_type: AuthType) {
        match Database::global().global_config.set_auth_type(auth_type) {
            Ok(()) => {
                self.send(Message::AuthTypeChanged(auth_type));
            }
            Err(error) => {
                error!("unable to set auth type: {error:?}");
            }
        }
    }

    fn send(&self, message: Message) {
        self.reconciler.send_sync(message);
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
                }
            }

            Action::DisableBiometric => {
                debug!("disable biometric");

                match self.auth_type() {
                    AuthType::Biometric => self.set_auth_type(AuthType::None),
                    AuthType::Both => self.set_auth_type(AuthType::Pin),
                    _ => {}
                }
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
            Action::DisableDecoyPin => self.delete_decoy_pin(),
        }
    }

    // MARK: Security Settings Validation

    /// Validate a security settings action and return what UI to show
    pub fn validate_security_action(
        &self,
        action: SecuritySettingsAction,
        unverified_wallet_ids: Vec<WalletId>,
    ) -> SecuritySettingsResult {
        use SecuritySettingsAction::{
            ChangePin, ToggleBiometric, ToggleDecoyPin, TogglePin, ToggleWipeDataPin,
        };
        use SecuritySettingsResult::{DecoyModeLocalUpdate, ProceedToSheet, ShowAlert};

        match action {
            ToggleBiometric { enable } => {
                if self.is_in_decoy_mode() {
                    return DecoyModeLocalUpdate;
                }

                if !enable {
                    return ProceedToSheet(SecuritySheetState::DisableBiometric);
                }

                // check trick PINs before enabling biometrics
                if self.is_decoy_pin_enabled() && self.is_wipe_data_pin_enabled() {
                    return ShowAlert(SecurityAlertState::NoteNoFaceIdWhenTrickPins);
                }

                if self.is_wipe_data_pin_enabled() {
                    return ShowAlert(SecurityAlertState::NoteNoFaceIdWhenWipeMePin);
                }

                if self.is_decoy_pin_enabled() {
                    return ShowAlert(SecurityAlertState::NoteNoFaceIdWhenDecoyPin);
                }

                ProceedToSheet(SecuritySheetState::EnableBiometric)
            }

            TogglePin { enable } => {
                if self.is_in_decoy_mode() {
                    return DecoyModeLocalUpdate;
                }

                if enable {
                    ProceedToSheet(SecuritySheetState::NewPin)
                } else {
                    ProceedToSheet(SecuritySheetState::RemovePin)
                }
            }

            ToggleWipeDataPin { enable } => {
                if !enable {
                    if self.is_in_decoy_mode() {
                        return DecoyModeLocalUpdate;
                    }
                    return ProceedToSheet(SecuritySheetState::RemoveWipeDataPin);
                }

                // check unverified wallets
                if let Some(wallet_id) = unverified_wallet_ids.first() {
                    return ShowAlert(SecurityAlertState::UnverifiedWallets {
                        wallet_id: wallet_id.clone(),
                    });
                }

                // PIN is required (biometric only = no PIN)
                if self.auth_type() == AuthType::Biometric {
                    return ShowAlert(SecurityAlertState::NotePinRequired);
                }

                // must disable biometric first
                if self.auth_type() == AuthType::Both {
                    return ShowAlert(SecurityAlertState::NoteFaceIdDisablingForWipeMePin);
                }

                ShowAlert(SecurityAlertState::ConfirmEnableWipeMePin)
            }

            ToggleDecoyPin { enable } => {
                if !enable {
                    if self.is_in_decoy_mode() {
                        return DecoyModeLocalUpdate;
                    }
                    return ProceedToSheet(SecuritySheetState::RemoveDecoyPin);
                }

                // PIN is required
                if self.auth_type() == AuthType::Biometric {
                    return ShowAlert(SecurityAlertState::NotePinRequired);
                }

                // must disable biometric first
                if self.auth_type() == AuthType::Both {
                    return ShowAlert(SecurityAlertState::NoteFaceIdDisablingForDecoyPin);
                }

                ShowAlert(SecurityAlertState::ConfirmDecoyPin)
            }

            ChangePin => ProceedToSheet(SecuritySheetState::ChangePin),
        }
    }

    /// Validate a new PIN doesn't conflict with existing PINs
    pub fn validate_new_pin(&self, new_pin: String) -> Option<String> {
        if self.check_wipe_data_pin(&new_pin) {
            return Some(
                "Can't update PIN because it's the same as your wipe data PIN".to_string(),
            );
        }

        if self.check_decoy_pin(&new_pin) {
            return Some("Can't update PIN because it's the same as your decoy PIN".to_string());
        }

        None
    }
}

impl RustAuthManager {
    /// Whether the saved settings say no authentication is set up
    ///
    /// Lock decisions fail closed, so a settings read error counts as authentication
    /// being set up instead of the `AuthType::None` default `auth_type` falls back to
    fn is_auth_known_disabled(&self) -> bool {
        Database::global()
            .global_config
            .auth_type()
            .tap_err(|error| error!("unable to read auth type for app lock: {error:?}"))
            .is_ok_and(|auth_type| auth_type == AuthType::None)
    }

    fn check_wipe_data_pin(&self, pin: &str) -> bool {
        if pin.is_empty() {
            return false;
        }

        let wipe_data_pin = Database::global().global_config.wipe_data_pin().unwrap_or_default();

        pin == wipe_data_pin
    }

    fn check_decoy_pin(&self, pin: &str) -> bool {
        if pin.is_empty() {
            return false;
        }

        let decoy_pin = Database::global().global_config.decoy_pin().unwrap_or_default();

        pin == decoy_pin
    }
}

#[uniffi::export(callback_interface)]
pub trait AuthManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: AuthManagerReconcileMessage);
}

crate::manager::reconcile_channel::impl_reconcile_sink!(
    dyn AuthManagerReconciler,
    AuthManagerReconcileMessage
);

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use crate::{
        app::reconcile::test_support::AppUpdateTap, database::test_support::RestoreSignetWallets,
        network::Network, wallet::Wallet,
    };

    /// Sets the global auth type for one test and restores the previous one afterward
    struct AuthTypeGuard(AuthType);

    impl AuthTypeGuard {
        fn set(auth_type: AuthType) -> Self {
            let config = &Database::global().global_config;
            let previous = config.auth_type().unwrap_or_default();
            config.set_auth_type(auth_type).expect("set auth type");

            Self(previous)
        }
    }

    impl Drop for AuthTypeGuard {
        fn drop(&mut self) {
            let _ = Database::global().global_config.set_auth_type(self.0);
        }
    }

    /// Runs `test` with a fresh auth manager whose app is unlocked under `auth_type`
    fn with_unlocked_manager(auth_type: AuthType, test: impl FnOnce(Arc<RustAuthManager>)) {
        let _lock = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::init_test_database();
        let _auth_type = AuthTypeGuard::set(auth_type);

        let manager = RustAuthManager::init();
        manager.unlock_authenticated();
        assert!(!manager.is_locked());

        test(manager);
    }

    #[test]
    fn send_confirmation_cannot_clear_a_lock_it_did_not_take() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            // the confirmation screen asks for authentication before sending
            let send = manager.lock_for_send_confirmation().expect("auth is enabled");
            assert!(manager.is_locked());
            manager.unlock_authenticated();

            // the app leaves the foreground while the send is still in flight
            assert!(manager.lock(AppLockReason::Background));

            // the confirmation screen finishes and gives back only its own lock
            assert!(manager.release_send_confirmation_lock(send));
            assert!(manager.is_locked());
        });
    }

    #[test]
    fn send_confirmation_cannot_clear_a_reauthentication_lock() {
        with_unlocked_manager(AuthType::Biometric, |manager| {
            let send = manager.lock_for_send_confirmation().expect("auth is enabled");
            manager.lock(AppLockReason::Reauthentication);

            assert!(manager.release_send_confirmation_lock(send));
        });
    }

    #[test]
    fn quick_foreground_keeps_send_confirmation_lock() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            let send = manager.lock_for_send_confirmation().expect("auth is enabled");
            manager.lock(AppLockReason::Background);

            // returning right away only clears the lock leaving the foreground took
            assert!(manager.release_background_lock_on_foreground());

            // leaving the confirmation screen right away clears the last lock
            assert!(!manager.release_send_confirmation_lock(send));
        });
    }

    #[test]
    fn quick_foreground_releases_background_lock() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            manager.lock(AppLockReason::Background);

            assert!(!manager.release_background_lock_on_foreground());
        });
    }

    #[test]
    fn unreadable_auth_settings_still_lock() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            let config = &Database::global().global_config;
            config
                .set(database::global_config::GlobalConfigKey::AuthType, "unreadable".into())
                .expect("write raw auth type");

            assert!(manager.lock(AppLockReason::Background));
            assert!(manager.release_background_lock_on_foreground());
            assert!(manager.lock_for_send_confirmation().is_some());
            assert!(manager.unlock_if_auth_disabled());
        });
    }

    #[test]
    fn no_lock_is_taken_without_authentication() {
        with_unlocked_manager(AuthType::None, |manager| {
            assert!(!manager.lock(AppLockReason::Background));
            assert!(manager.lock_for_send_confirmation().is_none());
        });
    }

    /// Restores the app PIN and decoy PIN a test replaced
    struct PinGuard {
        hashed_pin: Option<String>,
        decoy_pin: Option<String>,
    }

    impl PinGuard {
        fn capture() -> Self {
            let config = &Database::global().global_config;
            let hashed_pin = config.hashed_pin_code().ok().filter(|pin| !pin.is_empty());
            let decoy_pin = config.decoy_pin().ok().filter(|pin| !pin.is_empty());

            Self { hashed_pin, decoy_pin }
        }
    }

    impl Drop for PinGuard {
        fn drop(&mut self) {
            let config = &Database::global().global_config;
            let _ = match self.hashed_pin.take() {
                Some(hashed_pin) => config.set_hashed_pin_code(hashed_pin),
                None => config.delete_hashed_pin_code(),
            };

            let _ = match self.decoy_pin.take() {
                Some(decoy_pin) => config.set_decoy_pin(decoy_pin),
                None => config.delete_decoy_pin(),
            };
        }
    }

    /// The wallets the sidebar and Settings → All Wallets list for the current mode
    fn visible_wallet_ids() -> Vec<WalletId> {
        let wallets = Database::global().wallets().all().expect("wallets load");
        wallets.into_iter().map(|wallet| wallet.id).collect()
    }

    fn wallet_mode_changes(tap: &AppUpdateTap) -> Vec<WalletMode> {
        tap.drain()
            .into_iter()
            .filter_map(|update| match update {
                AppStateReconcileMessage::WalletModeChanged(mode) => Some(mode),
                _ => None,
            })
            .collect()
    }

    /// Pastes a watch-only export for `words` into the current mode and selects it
    fn import_and_select_watch(words: &str) -> WalletId {
        let mnemonic = bip39::Mnemonic::from_str(words).expect("valid test mnemonic");
        let export =
            crate::wallet::test_support::public_descriptor_export(&mnemonic, Network::Signet);

        let id = Wallet::try_new_persisted_from_xpub(export).expect("watch import").id;
        Database::global().global_config.select_wallet(id.clone()).expect("select wallet");

        id
    }

    // the platform unlock screens route a PIN by checking the app PIN, then the decoy PIN, and
    // switch modes through these calls, so this follows the same order
    #[test]
    fn decoy_pin_enters_and_main_pin_leaves_a_separate_wallet_scope() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            crate::test_support::ensure_tokio_runtime();
            crate::test_support::init_test_keychain();
            let _wallets = RestoreSignetWallets::empty();
            let _pins = PinGuard::capture();
            let config = &Database::global().global_config;

            let main_pin = AuthPin::new();
            main_pin.set("111111".to_string()).unwrap();
            assert_eq!(
                manager.set_decoy_pin("111111".to_string()),
                Err(AuthManagerError::DecoySet(TrickPinError::SameAsCurrentPin))
            );

            manager.set_decoy_pin("222222".to_string()).expect("a distinct decoy pin saves");
            assert!(manager.is_decoy_pin_enabled());

            let main = import_and_select_watch(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            );

            let main_before = visible_wallet_ids();
            assert_eq!(main_before, vec![main.clone()]);

            // the decoy pin is not the app pin, so the unlock screen enters decoy mode
            assert!(!main_pin.check("222222"));
            assert!(manager._check_decoy_pin("222222".to_string()));
            assert!(!manager._check_decoy_pin("111111".to_string()));
            assert!(!manager._check_decoy_pin(String::new()));

            let tap = AppUpdateTap::open();
            manager.switch_to_decoy_mode();
            assert_eq!(wallet_mode_changes(&tap), vec![WalletMode::Decoy]);
            assert!(manager.is_in_decoy_mode());
            assert!(visible_wallet_ids().is_empty(), "decoy mode lists no main wallet");
            assert_eq!(config.selected_wallet(), None);

            let decoy =
                import_and_select_watch("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong");

            assert_eq!(visible_wallet_ids(), vec![decoy.clone()]);

            // the app pin in decoy mode switches back to main
            assert!(main_pin.check("111111"));
            manager.switch_to_main_mode();
            assert_eq!(wallet_mode_changes(&tap), vec![WalletMode::Main]);
            assert!(!manager.is_in_decoy_mode());
            assert_eq!(visible_wallet_ids(), main_before);
            assert_eq!(config.selected_wallet(), Some(main));

            // re-entering decoy mode restores its own wallet and selection
            manager.switch_to_decoy_mode();
            assert_eq!(wallet_mode_changes(&tap), vec![WalletMode::Decoy]);
            assert_eq!(visible_wallet_ids(), vec![decoy.clone()]);
            assert_eq!(config.selected_wallet(), Some(decoy));
        });
    }
}
