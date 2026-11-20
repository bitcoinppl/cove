mod app_lock;
mod wipe_data_pin;

use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use tap::TapFallible as _;
use tracing::{debug, error, warn};

use cove_types::WalletId;

use crate::{
    app::reconcile::{AppStateReconcileMessage, Updater},
    auth::{AuthError, AuthPin, AuthType},
    database::{self, Database},
    manager::{cloud_backup_manager::CLOUD_BACKUP_MANAGER, reconcile_channel::ReconcileChannel},
    wallet::metadata::WalletMode,
};

pub use app_lock::{AppLockReason, SendConfirmationLock};

use app_lock::{AppLock, BootTime};
use wipe_data_pin::WipeDataPinBlocker;

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
    /// The trick PINs Security Settings now says were turned off, none in decoy mode
    TrickPinNoticeChanged(Option<DuplicateTrickPin>),
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
    /// The trick PINs an unlock turned off for matching the app PIN, which Security Settings
    /// shows until the user dismisses it; always none in decoy mode
    pub trick_pin_notice: Option<DuplicateTrickPin>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AuthManagerAction {
    UpdateAuthType(AuthType),
    EnableBiometric,
    DisableBiometric,
    DisablePin,
    DisableWipeDataPin,
    DisableDecoyPin,
    /// The user read the notice that trick PINs were turned off
    DismissTrickPinNotice,
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
    /// A wallet the current mode lists has no backup; Go To Wallet opens it
    UnverifiedWallets {
        wallet_id: WalletId,
    },
    /// A decoy wallet has no backup, which main mode can't open
    UnverifiedDecoyWallets,
    /// A wallet list couldn't be read, so the Wipe Data PIN stays off
    UnreadableWallets,
    /// A wallet relies on Cloud Backup, whose master key the provider can't confirm
    CloudBackupKeyUnconfirmed,
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

/// What a PIN typed at the lock screen opens
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum UnlockPin {
    /// The app PIN, which opens the main wallets
    App,
    /// The decoy PIN, which opens the decoy wallets
    Decoy,
    /// The wipe data PIN, which wipes every wallet
    WipeData,
    /// No PIN this app has saved
    Wrong,
}

/// Trick PINs saved equal to the app PIN, which builds before Change PIN refused them allowed
///
/// Unlocking checks the app PIN first, so such a decoy PIN opened the main wallets and such a wipe
/// data PIN never wiped anything. The unlock that finds one turns it off and keeps this as a
/// notice, which Security Settings shows in main mode until the user dismisses it
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum DuplicateTrickPin {
    Decoy,
    WipeData,
    DecoyAndWipeData,
}

impl DuplicateTrickPin {
    pub(crate) fn from_flags(decoy: bool, wipe_data: bool) -> Option<Self> {
        match (decoy, wipe_data) {
            (true, true) => Some(Self::DecoyAndWipeData),
            (true, false) => Some(Self::Decoy),
            (false, true) => Some(Self::WipeData),
            (false, false) => None,
        }
    }

    fn includes_decoy(self) -> bool {
        matches!(self, Self::Decoy | Self::DecoyAndWipeData)
    }

    fn includes_wipe_data(self) -> bool {
        matches!(self, Self::WipeData | Self::DecoyAndWipeData)
    }

    /// Both sets of turned-off PINs, as one notice
    fn and(this: Option<Self>, other: Option<Self>) -> Option<Self> {
        let decoy =
            this.is_some_and(Self::includes_decoy) || other.is_some_and(Self::includes_decoy);
        let wipe_data = this.is_some_and(Self::includes_wipe_data)
            || other.is_some_and(Self::includes_wipe_data);

        Self::from_flags(decoy, wipe_data)
    }
}

#[uniffi::export]
impl DuplicateTrickPin {
    /// What Security Settings tells the user; it names no PIN, so it reads the same to whoever
    /// holds the phone
    pub fn notice(&self) -> String {
        match self {
            Self::Decoy => {
                "Your decoy PIN was turned off because it matched your app PIN. Set a new one to use it again. Your decoy wallets are kept and open with the new decoy PIN."
            }
            Self::WipeData => {
                "Your wipe data PIN was turned off because it matched your app PIN. Set a new one to use it again."
            }
            Self::DecoyAndWipeData => {
                "Your decoy PIN and wipe data PIN were turned off because they matched your app PIN. Set new ones to use them again. Your decoy wallets are kept and open with the new decoy PIN."
            }
        }
        .to_string()
    }
}

/// What a secret export, such as revealing a private key or sending one with KeyTeleport, must
/// ask for before it shows or sends the secret
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecretExportAccess {
    /// No app lock is set up, so the unlocked session is trusted to export
    Trusted,
    /// The user must present the main credential: the app PIN, or biometrics when `lock_type`
    /// allows them. The decoy and wipe data PINs never satisfy it
    MainCredential { lock_type: AuthType },
}

/// Result of validating a security settings action
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SecuritySettingsResult {
    /// Proceed to show a sheet for PIN entry
    ProceedToSheet(SecuritySheetState),
    /// Show an alert dialog
    ShowAlert(SecurityAlertState),
    /// Decoy mode: ask for the decoy PIN, as main mode asks for its PIN, then apply the action to
    /// the decoy-local settings only, never the saved ones
    DecoyPinThenLocalUpdate(SecuritySettingsAction),
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

    /// A wallet the wipe would delete has no backup; the alert says which, as the toggle's does
    #[error(
        "Unable to set the wipe data PIN, because not every wallet it would delete has a backup"
    )]
    WipeDataRefused(SecurityAlertState),

    #[error("Unable to set the decoy PIN, because {0}")]
    DecoySet(TrickPinError),

    #[error("Can't update PIN because {0}")]
    PinSet(NewPinError),

    #[error("{0}")]
    PinSave(AuthError),

    #[error("There was a database error: {0}")]
    Database(#[from] database::Error),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum TrickPinError {
    /// Unable to set trick PIN, because PIN is not enabled
    #[error("PIN is not enabled")]
    PinNotEnabled,

    /// Unable to set trick PIN, because it is the same as the current PIN
    #[error("it is the same as your current PIN")]
    SameAsCurrentPin,

    /// Unable to set trick PIN, because it is the same as another PIN
    #[error("it is the same as another PIN")]
    SameAsAnotherPin,

    /// Unable to set trick PIN, because biometrics is enabled
    #[error("biometrics is enabled")]
    BiometricsEnabled,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum NewPinError {
    /// Unlocking checks the app PIN first, so this PIN would open the main wallets instead of
    /// wiping them
    #[error("it's the same as your wipe data PIN")]
    SameAsWipeDataPin,

    /// Unlocking checks the app PIN first, so this PIN would open the main wallets instead of
    /// the decoy wallets
    #[error("it's the same as your decoy PIN")]
    SameAsDecoyPin,
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
        self.send(Message::TrickPinNoticeChanged(settings.trick_pin_notice));
    }

    pub(crate) fn settings(&self) -> AuthSettings {
        AuthSettings {
            auth_type: self.auth_type(),
            is_wipe_data_pin_enabled: self.is_wipe_data_pin_enabled(),
            is_decoy_pin_enabled: self.is_decoy_pin_enabled(),
            trick_pin_notice: self.trick_pin_notice(),
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

    /// What a secret export must ask for before it shows or sends the secret
    ///
    /// Fails closed: when the auth type can't be read, the export still asks for the app PIN
    pub fn secret_export_access(&self) -> SecretExportAccess {
        match Database::global().global_config.auth_type() {
            Ok(AuthType::None) => SecretExportAccess::Trusted,
            Ok(lock_type) => SecretExportAccess::MainCredential { lock_type },
            Err(error) => {
                error!(
                    "unable to read auth type for a secret export, asking for the PIN: {error:?}"
                );
                SecretExportAccess::MainCredential { lock_type: AuthType::Pin }
            }
        }
    }

    // MARK: APP PIN

    /// Set the app PIN, turning the PIN lock on when it was off
    ///
    /// Refuses a PIN equal to the decoy or wipe data PIN, since unlocking checks the app PIN first
    pub fn set_pin(&self, pin: String) -> Result<()> {
        self.validate_new_pin(&pin)?;

        // a lock type that can't be read fails the call before the PIN changes, so the PIN
        // screen never closes as if the lock were on while the app still opens without one
        let config = &Database::global().global_config;
        let lock_type = match config.auth_type()? {
            AuthType::None => AuthType::Pin,
            AuthType::Biometric => AuthType::Both,
            AuthType::Pin | AuthType::Both => {
                return AuthPin::new().set(pin).map_err(AuthManagerError::PinSave);
            }
        };

        // the PIN and the lock that turns it on commit together, so a failed write never leaves
        // a PIN the export prompt would accept while the lock is still off
        let hashed = AuthPin::new().hash(pin).map_err(AuthManagerError::PinSave)?;
        config.set_hashed_pin_code_and_auth_type(hashed, lock_type)?;
        self.send(Message::AuthTypeChanged(lock_type));

        Ok(())
    }

    /// Decides what a PIN typed at the lock screen opens, checking the app PIN first
    ///
    /// An app PIN equal to a trick PIN saved by an older build turns that trick PIN off, since it
    /// could never act, and keeps a notice for Security Settings instead of saying so at the
    /// lock, where whoever holds the phone would read it. The decoy wallets stay saved and open
    /// again with a new decoy PIN. The trick PINs are saved as plain text, so checking them costs
    /// this unlock no extra hash
    pub fn resolve_unlock_pin(&self, pin: String) -> UnlockPin {
        if AuthPin::new().check(&pin) {
            self.turn_off_trick_pins_equal_to(&pin);
            return UnlockPin::App;
        }

        if self.check_decoy_pin(&pin) {
            return UnlockPin::Decoy;
        }

        if self.check_wipe_data_pin(&pin) {
            return UnlockPin::WipeData;
        }

        UnlockPin::Wrong
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
        let pin = pin_setting_or_unset("decoy", Database::global().global_config.decoy_pin());

        !pin.is_empty()
    }

    /// The trick PINs an unlock turned off for matching the app PIN, for Security Settings to show
    /// until the user dismisses it
    ///
    /// Always none in decoy mode, which must never hint at the main wallets' settings
    pub fn trick_pin_notice(&self) -> Option<DuplicateTrickPin> {
        let config = &Database::global().global_config;
        if config.is_in_decoy_mode() {
            return None;
        }

        config.trick_pin_notice()
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
        self.drop_from_trick_pin_notice(DuplicateTrickPin::Decoy);

        Ok(())
    }

    /// Switch from main mode to decoy mode
    pub fn switch_to_decoy_mode(&self) {
        Database::global().global_config.set_decoy_mode().expect("failed to set decoy mode");
        self.republish_cloud_backup_screen(WalletMode::Decoy);
        self.send(Message::TrickPinNoticeChanged(None));

        Updater::send_update(AppStateReconcileMessage::WalletModeChanged(WalletMode::Decoy));
    }

    /// Switch from decoy mode to main mode
    pub fn switch_to_main_mode(&self) {
        Database::global().global_config.set_main_mode().expect("failed to set main mode");
        self.republish_cloud_backup_screen(WalletMode::Main);
        self.send(Message::TrickPinNoticeChanged(self.trick_pin_notice()));

        Updater::send_update(AppStateReconcileMessage::WalletModeChanged(WalletMode::Main));
    }

    // MARK: WIPE DATA PIN

    /// Check if the wipe data pin is enabled
    pub fn is_wipe_data_pin_enabled(&self) -> bool {
        let pin =
            pin_setting_or_unset("wipe data", Database::global().global_config.wipe_data_pin());

        !pin.is_empty()
    }

    /// Set the wipe data pin
    pub fn set_wipe_data_pin(&self, pin: String) -> Result<()> {
        self.validate_pin_settings(&pin).map_err(AuthManagerError::WipeDataSet)?;

        // the toggle checked before the PIN sheet opened, and a backup can fail while it is open,
        // so every save checks again
        if let Some(blocker) = WipeDataPinBlocker::find() {
            return Err(AuthManagerError::WipeDataRefused(blocker.into()));
        }

        // set the pin
        Database::global().global_config.set_wipe_data_pin(pin)?;
        self.send(Message::WipeDataPinChanged(true));
        self.drop_from_trick_pin_notice(DuplicateTrickPin::WipeData);

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

            // decoy mode never shows the notice, so it can't dismiss it either
            Action::DismissTrickPinNotice if self.is_in_decoy_mode() => {}
            Action::DismissTrickPinNotice => self.save_trick_pin_notice(None),
        }
    }

    // MARK: Security Settings Validation

    /// Validate a security settings action and return what UI to show
    pub fn validate_security_action(
        &self,
        action: SecuritySettingsAction,
    ) -> SecuritySettingsResult {
        use SecuritySettingsAction::{
            ChangePin, ToggleBiometric, ToggleDecoyPin, TogglePin, ToggleWipeDataPin,
        };
        use SecuritySettingsResult::{DecoyPinThenLocalUpdate, ProceedToSheet, ShowAlert};

        // decoy mode must look like main mode, so every change main mode guards with a PIN asks
        // for the decoy PIN; turning the PIN on asks for a new one in both modes, and enabling a
        // trick PIN takes main mode's path because its sheets only update the decoy-local toggle
        let is_in_decoy_mode = self.is_in_decoy_mode();
        let asks_for_decoy_pin = match &action {
            ToggleBiometric { .. } | TogglePin { enable: false } => true,
            ToggleWipeDataPin { enable } | ToggleDecoyPin { enable } => !enable,
            TogglePin { enable: true } | ChangePin => false,
        };

        if is_in_decoy_mode && asks_for_decoy_pin {
            return DecoyPinThenLocalUpdate(action);
        }

        match action {
            ToggleBiometric { enable } => {
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
                if enable {
                    ProceedToSheet(SecuritySheetState::NewPin)
                } else {
                    ProceedToSheet(SecuritySheetState::RemovePin)
                }
            }

            ToggleWipeDataPin { enable } => {
                if !enable {
                    return ProceedToSheet(SecuritySheetState::RemoveWipeDataPin);
                }

                // the wipe deletes every wallet, so each one needs a backup first
                if let Some(blocker) = WipeDataPinBlocker::find() {
                    return ShowAlert(blocker.into());
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
}

impl RustAuthManager {
    /// Refuses an app PIN equal to a trick PIN
    ///
    /// Reads the trick PINs strictly: an unreadable one could be the same PIN, so the new PIN is
    /// refused instead of treated as clear
    fn validate_new_pin(&self, pin: &str) -> Result<()> {
        let config = &Database::global().global_config;
        let matches = |trick_pin: String| !trick_pin.is_empty() && trick_pin == pin;

        if matches(config.wipe_data_pin()?) {
            return Err(AuthManagerError::PinSet(NewPinError::SameAsWipeDataPin));
        }

        if matches(config.decoy_pin()?) {
            return Err(AuthManagerError::PinSet(NewPinError::SameAsDecoyPin));
        }

        Ok(())
    }

    /// Turns off each trick PIN equal to the app PIN `pin`, returning the ones it turned off
    ///
    /// A delete that fails leaves that PIN for the next unlock to retry, so the alert never says
    /// a PIN was turned off while it is still saved
    fn turn_off_trick_pins_equal_to(&self, pin: &str) {
        let decoy = self.check_decoy_pin(pin);
        let wipe_data = self.check_wipe_data_pin(pin);
        let Some(found) = DuplicateTrickPin::from_flags(decoy, wipe_data) else { return };

        // the notice is saved before any PIN is deleted, so the user always hears about a PIN
        // that is gone; if the app stops between the two, the next unlock repeats both
        let config = &Database::global().global_config;
        let notice = DuplicateTrickPin::and(config.trick_pin_notice(), Some(found));
        if let Err(error) = config.set_trick_pin_notice(notice) {
            error!("unable to save the notice for a trick PIN equal to the app PIN: {error:?}");
            return;
        }

        if decoy {
            self.turned_off("decoy", config.delete_decoy_pin(), Message::DecoyPinChanged(false));
        }

        if wipe_data {
            self.turned_off(
                "wipe data",
                config.delete_wipe_data_pin(),
                Message::WipeDataPinChanged(false),
            );
        }

        self.send(Message::TrickPinNoticeChanged(self.trick_pin_notice()));
    }

    fn turned_off(
        &self,
        kind: &str,
        deleted: Result<(), database::Error>,
        changed: AuthManagerReconcileMessage,
    ) {
        match deleted {
            Ok(()) => {
                warn!("turned off a {kind} PIN saved equal to the app PIN");
                self.send(changed);
            }
            Err(error) => {
                error!("unable to turn off a {kind} PIN saved equal to the app PIN: {error:?}");
            }
        }
    }

    /// Takes `pins` out of the notice once the user sets a new one, since the notice asks for it
    fn drop_from_trick_pin_notice(&self, pins: DuplicateTrickPin) {
        let config = &Database::global().global_config;
        let Some(notice) = config.trick_pin_notice() else { return };

        let remaining = DuplicateTrickPin::from_flags(
            notice.includes_decoy() && !pins.includes_decoy(),
            notice.includes_wipe_data() && !pins.includes_wipe_data(),
        );

        self.save_trick_pin_notice(remaining);
    }

    fn save_trick_pin_notice(&self, notice: Option<DuplicateTrickPin>) {
        let config = &Database::global().global_config;
        if let Err(error) = config.set_trick_pin_notice(notice) {
            error!("unable to update the trick PIN notice: {error:?}");
        }

        self.send(Message::TrickPinNoticeChanged(self.trick_pin_notice()));
    }

    /// Asks Cloud Backup to publish its screen again after the wallet mode changed, since its
    /// root prompts show in main mode only
    fn republish_cloud_backup_screen(&self, wallet_mode: WalletMode) {
        // a manager that was never started has published nothing, and starting one here would
        // begin Cloud Backup work just for a mode switch
        if let Some(cloud_backup) = LazyLock::get(&CLOUD_BACKUP_MANAGER) {
            cloud_backup.wallet_mode_changed(wallet_mode);
        }
    }

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

        let wipe_data_pin =
            pin_setting_or_unset("wipe data", Database::global().global_config.wipe_data_pin());

        pin == wipe_data_pin
    }

    fn check_decoy_pin(&self, pin: &str) -> bool {
        if pin.is_empty() {
            return false;
        }

        let decoy_pin = pin_setting_or_unset("decoy", Database::global().global_config.decoy_pin());

        pin == decoy_pin
    }
}

/// Treats an unreadable PIN setting as unset, logging the read error but never the PIN
fn pin_setting_or_unset(kind: &str, read: Result<String, crate::database::Error>) -> String {
    read.unwrap_or_else(|error| {
        error!("failed to read {kind} PIN setting, treating it as unset: {error}");
        String::new()
    })
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
        manager::deferred_sender::SingleOrMany, network::Network, wallet::Wallet,
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

        // saving a wipe data PIN reads Cloud Backup's keychain namespace
        crate::test_support::init_test_keychain();
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

    /// Restores the app PIN and trick PINs a test replaced
    struct PinGuard {
        hashed_pin: Option<String>,
        decoy_pin: Option<String>,
        wipe_data_pin: Option<String>,
        trick_pin_notice: Option<DuplicateTrickPin>,
    }

    impl PinGuard {
        fn capture() -> Self {
            let config = &Database::global().global_config;
            let hashed_pin = config.hashed_pin_code().ok().filter(|pin| !pin.is_empty());
            let decoy_pin = config.decoy_pin().ok().filter(|pin| !pin.is_empty());
            let wipe_data_pin = config.wipe_data_pin().ok().filter(|pin| !pin.is_empty());
            let trick_pin_notice = config.trick_pin_notice();

            Self { hashed_pin, decoy_pin, wipe_data_pin, trick_pin_notice }
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

            let _ = match self.wipe_data_pin.take() {
                Some(wipe_data_pin) => config.set_wipe_data_pin(wipe_data_pin),
                None => config.delete_wipe_data_pin(),
            };

            let _ = config.set_trick_pin_notice(self.trick_pin_notice.take());
        }
    }

    // unlocking checks the app PIN first, so an app PIN equal to a trick PIN would open the main
    // wallets where the user expects the decoy wallets or a wipe
    #[test]
    fn app_pin_can_never_become_a_trick_pin() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            let _pins = PinGuard::capture();
            let main_pin = AuthPin::new();
            main_pin.set("111111".to_string()).unwrap();
            manager.set_decoy_pin("222222".to_string()).expect("decoy pin saves");
            manager.set_wipe_data_pin("333333".to_string()).expect("wipe data pin saves");

            assert_eq!(
                manager.set_pin("222222".to_string()),
                Err(AuthManagerError::PinSet(NewPinError::SameAsDecoyPin))
            );
            assert_eq!(
                manager.set_pin("333333".to_string()),
                Err(AuthManagerError::PinSet(NewPinError::SameAsWipeDataPin))
            );

            assert!(main_pin.check("111111"), "the app PIN changed to a trick PIN");
            assert!(!main_pin.check("222222"));
            assert!(!main_pin.check("333333"));

            manager.set_pin("444444".to_string()).expect("a distinct PIN saves");
            assert!(main_pin.check("444444"));
        });
    }

    // the decoy PIN must open Settings that act like main mode's, so each toggle main mode guards
    // with a PIN asks for the decoy PIN there, and only the decoy's own copy of the setting changes
    #[test]
    fn decoy_mode_settings_ask_for_the_decoy_pin_like_main_mode() {
        use SecuritySettingsAction::{
            ToggleBiometric, ToggleDecoyPin, TogglePin, ToggleWipeDataPin,
        };

        with_unlocked_manager(AuthType::Pin, |manager| {
            let config = &Database::global().global_config;
            config.set_decoy_mode().expect("enter decoy mode");

            for action in [
                TogglePin { enable: false },
                ToggleBiometric { enable: true },
                ToggleBiometric { enable: false },
                ToggleWipeDataPin { enable: false },
                ToggleDecoyPin { enable: false },
            ] {
                assert_eq!(
                    manager.validate_security_action(action.clone()),
                    SecuritySettingsResult::DecoyPinThenLocalUpdate(action)
                );
            }

            // turning the PIN on asks for a new one, as main mode does
            assert_eq!(
                manager.validate_security_action(TogglePin { enable: true }),
                SecuritySettingsResult::ProceedToSheet(SecuritySheetState::NewPin)
            );

            config.set_main_mode().expect("leave decoy mode");
        });
    }

    // a settings read error must not read as "no app lock", or secrets export with no prompt
    #[test]
    fn unreadable_auth_settings_still_guard_secret_exports() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            let config = &Database::global().global_config;
            config
                .set(database::global_config::GlobalConfigKey::AuthType, "unreadable".into())
                .expect("write raw auth type");

            assert_eq!(
                manager.secret_export_access(),
                SecretExportAccess::MainCredential { lock_type: AuthType::Pin }
            );

            config.set_auth_type(AuthType::None).unwrap();
            assert_eq!(manager.secret_export_access(), SecretExportAccess::Trusted);

            config.set_auth_type(AuthType::Both).unwrap();
            assert_eq!(
                manager.secret_export_access(),
                SecretExportAccess::MainCredential { lock_type: AuthType::Both }
            );
        });
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

    /// An unverified hot wallet on signet in `mode`, saved as that mode's only signet wallet
    fn save_unverified_hot_wallet(mode: WalletMode) -> WalletId {
        let mut wallet = crate::wallet::metadata::WalletMetadata::preview_new();
        wallet.network = Network::Signet;
        wallet.wallet_mode = mode;
        let id = wallet.id.clone();

        Database::global()
            .wallets
            .save_all_wallets(Network::Signet, mode, vec![wallet])
            .expect("save the wallet");

        id
    }

    /// Drops the wallets table so every wallet read fails, and puts every wallet list back when
    /// dropped
    struct UnreadableWallets(
        Vec<(Network, WalletMode, Vec<crate::wallet::metadata::WalletMetadata>)>,
    );

    impl UnreadableWallets {
        fn start() -> Self {
            use strum::IntoEnumIterator as _;

            let wallets = &Database::global().wallets;
            let lists = Network::iter()
                .flat_map(|network| WalletMode::iter().map(move |mode| (network, mode)))
                .map(|(network, mode)| {
                    (network, mode, wallets.get_all(network, mode).expect("wallets read"))
                })
                .collect();

            let write_txn = wallets.redb().begin_write().expect("begin write");
            write_txn.delete_table(database::wallet::TABLE).expect("drop the wallets table");
            write_txn.commit().expect("commit");

            Self(lists)
        }
    }

    impl Drop for UnreadableWallets {
        fn drop(&mut self) {
            let wallets = &Database::global().wallets;
            let write_txn = wallets.redb().begin_write().expect("begin write");
            write_txn.open_table(database::wallet::TABLE).expect("recreate the wallets table");
            write_txn.commit().expect("commit");

            for (network, mode, list) in self.0.drain(..) {
                let _ = wallets.save_all_wallets(network, mode, list);
            }
        }
    }

    // the wipe deletes the decoy wallets too, so main mode must not allow the wipe data PIN while
    // a decoy wallet has no backup
    #[test]
    fn the_wipe_data_pin_check_counts_decoy_wallets() {
        use SecuritySettingsAction::ToggleWipeDataPin;

        with_unlocked_manager(AuthType::Pin, |manager| {
            crate::test_support::init_test_keychain();
            let _wallets = RestoreSignetWallets::empty();
            let check = || manager.validate_security_action(ToggleWipeDataPin { enable: true });

            assert_eq!(
                check(),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::ConfirmEnableWipeMePin),
                "no wallet needs a backup yet"
            );

            let decoy = save_unverified_hot_wallet(WalletMode::Decoy);
            assert_eq!(
                check(),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::UnverifiedDecoyWallets),
                "the wipe data PIN check left out the decoy wallet that has no backup"
            );

            // a main wallet comes first, since Go To Wallet can open it
            let main = save_unverified_hot_wallet(WalletMode::Main);
            assert_eq!(
                check(),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::UnverifiedWallets {
                    wallet_id: main
                })
            );

            // decoy mode looks like main mode and names its own wallet
            Database::global().global_config.set_decoy_mode().expect("enter decoy mode");
            assert_eq!(
                check(),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::UnverifiedWallets {
                    wallet_id: decoy
                })
            );
        });
    }

    // the toggle checks the backups before the PIN sheet opens, so a wallet that has no backup by
    // the time the PIN is saved must still stop the save
    #[test]
    fn saving_the_wipe_data_pin_checks_the_backups_again() {
        use SecuritySettingsAction::ToggleWipeDataPin;

        with_unlocked_manager(AuthType::Pin, |manager| {
            crate::test_support::init_test_keychain();
            let _wallets = RestoreSignetWallets::empty();
            let _pins = PinGuard::capture();
            AuthPin::new().set("111111".to_string()).unwrap();

            assert_eq!(
                manager.validate_security_action(ToggleWipeDataPin { enable: true }),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::ConfirmEnableWipeMePin),
                "no wallet needs a backup when the sheet opens"
            );

            // while the sheet is open, a wallet turns up with no backup
            let wallet = save_unverified_hot_wallet(WalletMode::Main);

            assert_eq!(
                manager.set_wipe_data_pin("222222".to_string()),
                Err(AuthManagerError::WipeDataRefused(SecurityAlertState::UnverifiedWallets {
                    wallet_id: wallet
                })),
                "the save went ahead although a wallet it would wipe has no backup"
            );
            assert!(!manager.is_wipe_data_pin_enabled(), "the refused wipe data PIN was saved");
        });
    }

    // a wallet list that can't be read could hold a wallet with no backup, so the wipe data PIN
    // must stay off and say why
    #[test]
    fn a_failed_wallet_read_refuses_the_wipe_data_pin() {
        use SecuritySettingsAction::ToggleWipeDataPin;

        with_unlocked_manager(AuthType::Pin, |manager| {
            crate::test_support::init_test_keychain();
            let _wallets = RestoreSignetWallets::empty();
            let _unreadable = UnreadableWallets::start();

            assert_eq!(
                manager.validate_security_action(ToggleWipeDataPin { enable: true }),
                SecuritySettingsResult::ShowAlert(SecurityAlertState::UnreadableWallets),
                "the wipe data PIN was allowed although no wallet list could be read"
            );
        });
    }

    // a build before Change PIN refused trick PINs could save the app PIN equal to the decoy PIN,
    // which then opened the main wallets; the unlock that finds it turns the decoy PIN off and
    // keeps a notice for Security Settings, never one at the lock where anyone could read it
    #[test]
    fn an_app_pin_equal_to_the_decoy_pin_turns_it_off_with_a_settings_notice() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            crate::test_support::init_test_keychain();
            let _wallets = RestoreSignetWallets::empty();
            let _pins = PinGuard::capture();
            let config = &Database::global().global_config;
            let _ = config.set_trick_pin_notice(None);

            AuthPin::new().set("111111".to_string()).unwrap();
            let decoy = save_unverified_hot_wallet(WalletMode::Decoy);
            config.set_decoy_pin("111111".to_string()).expect("an older build saves the duplicate");

            drained_messages(&manager);
            assert_eq!(manager.resolve_unlock_pin("111111".to_string()), UnlockPin::App);
            assert!(!manager.is_decoy_pin_enabled(), "the duplicate decoy PIN is still saved");
            assert_eq!(manager.trick_pin_notice(), Some(DuplicateTrickPin::Decoy));
            assert!(
                drained_messages(&manager)
                    .contains(&Message::TrickPinNoticeChanged(Some(DuplicateTrickPin::Decoy))),
                "Security Settings never heard about the notice"
            );

            // the notice is saved, so a restart still shows it, and the next unlock changes nothing
            assert_eq!(config.trick_pin_notice(), Some(DuplicateTrickPin::Decoy));
            assert_eq!(manager.resolve_unlock_pin("111111".to_string()), UnlockPin::App);
            assert_eq!(manager.trick_pin_notice(), Some(DuplicateTrickPin::Decoy));

            // decoy mode never shows it
            manager.switch_to_decoy_mode();
            assert_eq!(manager.trick_pin_notice(), None, "decoy mode showed the notice");
            assert_eq!(manager.settings().trick_pin_notice, None);
            manager.switch_to_main_mode();
            assert_eq!(manager.trick_pin_notice(), Some(DuplicateTrickPin::Decoy));

            // a new decoy PIN answers the notice and opens the decoy wallets again
            manager.set_decoy_pin("222222".to_string()).expect("a new decoy PIN saves");
            assert_eq!(manager.trick_pin_notice(), None, "the notice still asks for a new PIN");
            assert_eq!(manager.resolve_unlock_pin("222222".to_string()), UnlockPin::Decoy);
            manager.switch_to_decoy_mode();
            assert_eq!(visible_wallet_ids(), vec![decoy], "the decoy wallets didn't come back");
            manager.switch_to_main_mode();
        });
    }

    // the same for the wipe data PIN, which never wiped while it equaled the app PIN; with both
    // turned off, the notice names both until the user dismisses it
    #[test]
    fn an_app_pin_equal_to_the_wipe_data_pin_turns_it_off_until_dismissed() {
        with_unlocked_manager(AuthType::Pin, |manager| {
            let _pins = PinGuard::capture();
            let config = &Database::global().global_config;
            let _ = config.set_trick_pin_notice(None);

            AuthPin::new().set("111111".to_string()).unwrap();
            config.set_wipe_data_pin("111111".to_string()).expect("an older build saves it");

            assert_eq!(manager.resolve_unlock_pin("111111".to_string()), UnlockPin::App);
            assert!(!manager.is_wipe_data_pin_enabled(), "the duplicate wipe PIN is still saved");
            assert_eq!(manager.trick_pin_notice(), Some(DuplicateTrickPin::WipeData));

            // a decoy duplicate found later joins the notice that is still up
            config.set_decoy_pin("111111".to_string()).expect("an older build saves it");
            assert_eq!(manager.resolve_unlock_pin("111111".to_string()), UnlockPin::App);
            assert_eq!(manager.trick_pin_notice(), Some(DuplicateTrickPin::DecoyAndWipeData));

            manager.dispatch(AuthManagerAction::DismissTrickPinNotice);
            assert_eq!(manager.trick_pin_notice(), None, "dismissing kept the notice");
            assert_eq!(config.trick_pin_notice(), None, "dismissing kept the saved notice");

            manager.set_wipe_data_pin("333333".to_string()).expect("a new wipe data PIN saves");
            assert_eq!(manager.resolve_unlock_pin("333333".to_string()), UnlockPin::WipeData);
            assert_eq!(manager.resolve_unlock_pin("444444".to_string()), UnlockPin::Wrong);
        });
    }

    /// The reconcile messages the manager sent since the last call
    fn drained_messages(manager: &RustAuthManager) -> Vec<Message> {
        manager
            .reconciler
            .receiver()
            .try_iter()
            .flat_map(|sent| match sent {
                SingleOrMany::Single(message) => vec![message],
                SingleOrMany::Many(messages) => messages,
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
