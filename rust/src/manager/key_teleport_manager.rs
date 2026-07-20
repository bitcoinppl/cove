use std::{
    fmt,
    str::FromStr as _,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use bip39::Mnemonic;
use cove_device::keychain::{Keychain, KeychainError, WalletSecret, WalletXprv};
use cove_keyteleport::{
    DecodedPayload, Error as KeyTeleportError, NotesPayload, NotesRecord, NumericCode, Payload,
    ReceiverSession, SenderSession, TeleportPassword, XprvPayload,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{error, trace};
use zeroize::Zeroize as _;

use crate::{
    database::{self, Database},
    key_teleport::{KeyTeleportReceiverPacket, KeyTeleportSenderPacket},
    manager::{
        import_wallet_manager::{ImportWalletError, import_key_teleport_wallet_secret_with_target},
        reconcile_channel::ReconcileChannel,
    },
    multi_format::StringOrData,
    network::Network,
    wallet::metadata::{WalletId, WalletMetadata, WalletMode, WalletType},
};

use super::deferred_sender::SingleOrMany;

type Message = KeyTeleportManagerReconcileMessage;
type Action = KeyTeleportManagerAction;
type Reconciler = dyn KeyTeleportManagerReconciler;

const RECEIVE_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[uniffi::export(callback_interface)]
pub trait KeyTeleportManagerReconciler: Send + Sync + fmt::Debug + 'static {
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Debug, uniffi::Object)]
pub struct RustKeyTeleportManager {
    state: Arc<Mutex<KeyTeleportManagerState>>,
    private: Arc<Mutex<PrivateState>>,
    reconciler: ReconcileChannel<Message>,
}

#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportManagerReconcileMessage {
    UpdateState(KeyTeleportManagerState),
    SetAlert(KeyTeleportAlert),
    ClearAlert,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportManagerAction {
    StartReceive,
    /// Invalidates the active receive request and creates a new one
    RestartReceive,
    /// Deletes the active receive request
    EndReceive,
    Ingest(StringOrData),
    StartSendFromWallet(WalletId),
    SelectSendWallet(WalletId),
    EnterReceiverCode(String),
    /// Confirms sending the selected wallet's private key material
    ConfirmSendWallet,
    EnterSenderPassword(String),
    /// Imports the received mnemonic or extended private key as a hot wallet
    ImportReceivedWallet,
    RevealXprv,
    HideXprv,
    FinishReview,
    Clear,
}

#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportManagerState {
    Idle,
    ReceiveReady(KeyTeleportReceiveState),
    ReceiveEnterPassword,
    ReceiveMnemonicReview(KeyTeleportMnemonicReview),
    ReceiveXprvReview(KeyTeleportXprvReview),
    /// Displays received Secure Notes & Passwords content without treating it as a wallet
    ReceiveMessageReview(KeyTeleportMessageReview),
    /// Reports the wallet created from received private key material
    ReceiveImportedWallet(WalletMetadata),
    /// Waits for the receiver request after a sending wallet has been fixed
    SendAwaitReceiver,
    SendChooseWallet(KeyTeleportSendChooseWallet),
    SendEnterCode(KeyTeleportSendEnterCode),
    SendConfirm(KeyTeleportSendConfirm),
    SendReady(KeyTeleportSendReady),
}

impl fmt::Debug for KeyTeleportManagerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::ReceiveReady(_) => f.write_str("ReceiveReady(****)"),
            Self::ReceiveEnterPassword => f.write_str("ReceiveEnterPassword"),
            Self::ReceiveMnemonicReview(_) => f.write_str("ReceiveMnemonicReview(****)"),
            Self::ReceiveXprvReview(review) => f
                .debug_tuple("ReceiveXprvReview")
                .field(&format_args!("revealed={}", review.revealed))
                .finish(),
            Self::ReceiveMessageReview(review) => f
                .debug_tuple("ReceiveMessageReview")
                .field(&format_args!("item_count={}", review.items.len()))
                .finish(),
            Self::ReceiveImportedWallet(wallet) => {
                f.debug_tuple("ReceiveImportedWallet").field(&wallet.id).finish()
            }
            Self::SendAwaitReceiver => f.write_str("SendAwaitReceiver"),
            Self::SendChooseWallet(state) => f
                .debug_struct("SendChooseWallet")
                .field("eligible_wallets", &state.eligible_wallets)
                .finish(),
            Self::SendEnterCode(state) => f
                .debug_struct("SendEnterCode")
                .field("selected_wallet", &state.selected_wallet)
                .finish(),
            Self::SendConfirm(state) => f
                .debug_struct("SendConfirm")
                .field("selected_wallet", &state.selected_wallet)
                .field("warns_passphrase_not_included", &state.warns_passphrase_not_included)
                .finish(),
            Self::SendReady(_) => f.write_str("SendReady(****)"),
        }
    }
}

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportReceiveState {
    pub packet: Arc<KeyTeleportReceiverPacket>,
    pub numeric_code: String,
    pub grouped_numeric_code: String,
    pub created_at_secs: u64,
    pub network: Network,
    pub wallet_mode: WalletMode,
}

impl fmt::Debug for KeyTeleportReceiveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyTeleportReceiveState")
            .field("packet", &self.packet)
            .field("numeric_code", &"****")
            .field("created_at_secs", &self.created_at_secs)
            .field("network", &self.network)
            .field("wallet_mode", &self.wallet_mode)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportMnemonicReview {
    pub word_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportXprvReview {
    pub revealed: bool,
}

/// Display-ready Secure Notes & Passwords content received through Key Teleport
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportMessageReview {
    /// Records in their transmitted order
    pub items: Vec<KeyTeleportMessageItem>,
}

impl fmt::Debug for KeyTeleportMessageReview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyTeleportMessageReview").field("item_count", &self.items.len()).finish()
    }
}

/// Display-ready content for one received secure note or password record
#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportMessageItem {
    /// A free-form note
    Note { title: String, text: String, group: String },
    /// A structured password record
    Password {
        title: String,
        username: String,
        password: String,
        site: String,
        notes: String,
        group: String,
    },
}

impl fmt::Debug for KeyTeleportMessageItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Note { .. } => f.write_str("KeyTeleportMessageItem::Note(****)"),
            Self::Password { .. } => f.write_str("KeyTeleportMessageItem::Password(****)"),
        }
    }
}

impl From<NotesPayload> for KeyTeleportMessageReview {
    fn from(notes: NotesPayload) -> Self {
        let items = notes
            .records()
            .iter()
            .map(|record| match record {
                NotesRecord::Note(note) => KeyTeleportMessageItem::Note {
                    title: note.title().to_string(),
                    text: note.text().to_string(),
                    group: note.group().to_string(),
                },
                NotesRecord::Password(password) => KeyTeleportMessageItem::Password {
                    title: password.title().to_string(),
                    username: password.username().to_string(),
                    password: password.password().to_string(),
                    site: password.site().to_string(),
                    notes: password.notes().to_string(),
                    group: password.group().to_string(),
                },
            })
            .collect();

        Self { items }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendChooseWallet {
    /// Wallets available for the pending receiver request
    pub eligible_wallets: Vec<WalletMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendEnterCode {
    pub selected_wallet: WalletMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendConfirm {
    pub selected_wallet: WalletMetadata,
    pub warns_passphrase_not_included: bool,
}

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendReady {
    pub packet: Arc<KeyTeleportSenderPacket>,
    pub password: Arc<KeyTeleportPassword>,
}

impl fmt::Debug for KeyTeleportSendReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyTeleportSendReady")
            .field("packet", &self.packet)
            .field("password", &"****")
            .finish()
    }
}

#[derive(Clone, uniffi::Object)]
pub struct KeyTeleportPassword(TeleportPassword);

impl KeyTeleportPassword {
    fn new(password: TeleportPassword) -> Self {
        Self(password)
    }
}

impl PartialEq for KeyTeleportPassword {
    fn eq(&self, other: &Self) -> bool {
        self.0.expose_bytes() == other.0.expose_bytes()
    }
}

impl Eq for KeyTeleportPassword {}

impl fmt::Debug for KeyTeleportPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyTeleportPassword(****)")
    }
}

#[uniffi::export]
impl KeyTeleportPassword {
    pub fn display_text(&self) -> String {
        self.0.as_display_text()
    }

    pub fn grouped_text(&self) -> String {
        self.0.grouped()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum KeyTeleportAlert {
    #[error("start a receive session before accepting sender data")]
    NoActiveReceiveSession,

    #[error("receive session expired")]
    ReceiveSessionExpired,

    #[error("unable to parse Key Teleport data")]
    ParseFailed,

    #[error("Key Teleport PSBT packets are not supported yet")]
    UnsupportedPsbt,

    #[error("this Key Teleport payload is not supported")]
    /// The payload uses a valid but unsupported protocol type
    UnsupportedPayload,

    #[error("the decrypted Key Teleport payload is invalid")]
    /// The password was valid but the decrypted typed payload was malformed
    InvalidPayload,

    #[error("wrong receiver code")]
    WrongReceiverCode,

    #[error("wrong Teleport Password")]
    WrongTeleportPassword,

    #[error("no eligible hot wallets with saved private keys")]
    NoEligibleWallets,

    #[error("selected wallet is not eligible")]
    IneligibleWallet,

    #[error("no pending send")]
    NoPendingSend,

    #[error("no pending receive secret")]
    NoPendingReceiveSecret,

    #[error("import failed: {0}")]
    ImportFailed(String),

    #[error("keychain error: {0}")]
    Keychain(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("database error: {0}")]
    Database(String),
}

impl KeyTeleportAlert {
    fn from_receive_decode_error(error: KeyTeleportError) -> Self {
        match error {
            KeyTeleportError::Checksum => Self::WrongTeleportPassword,
            KeyTeleportError::UnsupportedPayload(_) => Self::UnsupportedPayload,
            KeyTeleportError::InvalidMnemonicPayload
            | KeyTeleportError::InvalidXprvPayload
            | KeyTeleportError::InvalidNotesPayload => Self::InvalidPayload,
            error => Self::Protocol(error.to_string()),
        }
    }
}

#[derive(Default)]
struct PrivateState {
    active_receive_session: Option<ActiveReceiveSession>,
    pending_receiver_packet: Option<Arc<KeyTeleportReceiverPacket>>,
    selected_send_wallet: Option<WalletMetadata>,
    receiver_code: Option<NumericCode>,
    pending_sender_packet: Option<Arc<KeyTeleportSenderPacket>>,
    received_secret: Option<ReceivedSecret>,
}

impl fmt::Debug for PrivateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateState")
            .field("active_receive_session", &self.active_receive_session.is_some())
            .field("pending_receiver_packet", &self.pending_receiver_packet.is_some())
            .field("selected_send_wallet", &self.selected_send_wallet.as_ref().map(|w| &w.id))
            .field("receiver_code", &self.receiver_code.as_ref().map(|_| "****"))
            .field("pending_sender_packet", &self.pending_sender_packet.is_some())
            .field("received_secret", &self.received_secret.as_ref().map(|_| "****"))
            .finish()
    }
}

enum ReceivedSecret {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

impl ReceivedSecret {
    fn to_wallet_secret(&self) -> Result<WalletSecret, KeyTeleportAlert> {
        match self {
            Self::Mnemonic(mnemonic) => Ok(WalletSecret::Mnemonic(mnemonic.clone())),
            Self::Xprv(xprv) => WalletXprv::parse(xprv.expose_string())
                .map(WalletSecret::Xpriv)
                .map_err(|_| KeyTeleportAlert::InvalidPayload),
        }
    }
}

impl Drop for PrivateState {
    fn drop(&mut self) {
        self.received_secret = None;
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedReceiveSession {
    private_key_hex: String,
    created_at_secs: u64,
    network: Network,
    wallet_mode: WalletMode,
}

impl fmt::Debug for PersistedReceiveSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedReceiveSession")
            .field("private_key_hex", &"****")
            .field("created_at_secs", &self.created_at_secs)
            .field("network", &self.network)
            .field("wallet_mode", &self.wallet_mode)
            .finish()
    }
}

impl Drop for PersistedReceiveSession {
    fn drop(&mut self) {
        self.private_key_hex.zeroize();
    }
}

#[derive(Debug)]
struct ActiveReceiveSession {
    receiver: ReceiverSession,
    created_at_secs: u64,
    network: Network,
    wallet_mode: WalletMode,
}

#[uniffi::export]
impl RustKeyTeleportManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(KeyTeleportManagerState::Idle)),
            private: Arc::new(Mutex::new(PrivateState::default())),
            reconciler: ReconcileChannel::new(20),
        })
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen_async(move |field| {
            trace!("key teleport reconcile: {field:?}");
            match field {
                SingleOrMany::Single(message) => reconciler.reconcile(message),
                SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
            }
        });
    }

    #[uniffi::method]
    pub fn state(&self) -> KeyTeleportManagerState {
        let state = self.state.lock().clone();
        if !matches!(state, KeyTeleportManagerState::Idle) {
            return state;
        }

        if let Some(resumed_state) = self.resume_receive_state() {
            self.set_state(resumed_state.clone());
            return resumed_state;
        }

        state
    }

    #[uniffi::method]
    pub fn reveal_mnemonic_words(&self) -> Vec<String> {
        let private = self.private.lock();
        let Some(ReceivedSecret::Mnemonic(mnemonic)) = private.received_secret.as_ref() else {
            return Vec::new();
        };

        mnemonic.words().map(ToString::to_string).collect()
    }

    #[uniffi::method]
    pub fn reveal_xprv(&self) -> Option<String> {
        self.set_xprv_revealed(true);

        let private = self.private.lock();
        let Some(ReceivedSecret::Xprv(xprv)) = private.received_secret.as_ref() else {
            return None;
        };

        Some(xprv.expose_string().to_string())
    }

    #[uniffi::method]
    pub fn is_send_eligible(&self, wallet_id: WalletId) -> bool {
        is_send_eligible_wallet_id(&wallet_id)
    }

    #[uniffi::method]
    pub fn dispatch(self: Arc<Self>, action: Action) {
        if let Err(alert) = self.handle_action(action) {
            self.reconciler.send(Message::SetAlert(alert));
        }
    }
}

impl RustKeyTeleportManager {
    fn handle_action(self: &Arc<Self>, action: Action) -> Result<(), KeyTeleportAlert> {
        self.reconciler.send(Message::ClearAlert);

        match action {
            Action::StartReceive => self.start_receive(),
            Action::RestartReceive => self.restart_receive(),
            Action::EndReceive => self.end_receive(),
            Action::Ingest(input) => self.ingest(input),
            Action::StartSendFromWallet(wallet_id) => self.start_send_from_wallet(wallet_id),
            Action::SelectSendWallet(wallet_id) => self.select_send_wallet(wallet_id),
            Action::EnterReceiverCode(code) => self.enter_receiver_code(&code),
            Action::ConfirmSendWallet => self.confirm_send_wallet(),
            Action::EnterSenderPassword(password) => self.enter_sender_password(&password),
            Action::ImportReceivedWallet => self.import_received_wallet(),
            Action::RevealXprv => {
                self.set_xprv_revealed(true);
                Ok(())
            }
            Action::HideXprv => {
                self.set_xprv_revealed(false);
                Ok(())
            }
            Action::FinishReview => self.end_receive(),
            Action::Clear => {
                self.clear_private();
                self.set_state(KeyTeleportManagerState::Idle);
                Ok(())
            }
        }
    }

    fn start_receive(&self) -> Result<(), KeyTeleportAlert> {
        if let Some(existing) = self.load_receive_session()? {
            if !existing.is_expired() {
                return self.activate_receive_session(ActiveReceiveSession::restore(&existing)?);
            }

            Keychain::global().delete_key_teleport_receive_session();
        }

        self.create_receive_session()
    }

    fn restart_receive(&self) -> Result<(), KeyTeleportAlert> {
        Keychain::global().delete_key_teleport_receive_session();

        self.create_receive_session()
    }

    fn create_receive_session(&self) -> Result<(), KeyTeleportAlert> {
        let session = ActiveReceiveSession::new();
        session.save()?;

        self.activate_receive_session(session)
    }

    fn activate_receive_session(
        &self,
        session: ActiveReceiveSession,
    ) -> Result<(), KeyTeleportAlert> {
        let state = receive_state_from_session(&session)?;
        let mut private = PrivateState::default();
        private.active_receive_session = Some(session);
        *self.private.lock() = private;
        self.set_state(KeyTeleportManagerState::ReceiveReady(state));

        Ok(())
    }

    fn end_receive(&self) -> Result<(), KeyTeleportAlert> {
        Keychain::global().delete_key_teleport_receive_session();
        self.clear_private();
        self.set_state(KeyTeleportManagerState::Idle);

        Ok(())
    }

    fn ingest(&self, input: StringOrData) -> Result<(), KeyTeleportAlert> {
        match crate::key_teleport::parse_key_teleport_input(input) {
            Ok(crate::key_teleport::ParsedKeyTeleport::Receiver(packet)) => {
                self.start_send_with_receiver_packet(packet)
            }
            Ok(crate::key_teleport::ParsedKeyTeleport::Sender(packet)) => {
                self.start_receive_password_entry(packet)
            }
            Ok(crate::key_teleport::ParsedKeyTeleport::UnsupportedPsbt)
            | Err(crate::key_teleport::KeyTeleportParseError::UnsupportedPsbt) => {
                Err(KeyTeleportAlert::UnsupportedPsbt)
            }
            Err(crate::key_teleport::KeyTeleportParseError::Unrecognized) => {
                Err(KeyTeleportAlert::ParseFailed)
            }
        }
    }

    fn start_send_from_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let wallet = eligible_wallet_by_id(&wallet_id)?;
        self.private.lock().selected_send_wallet = Some(wallet.clone());

        if self.private.lock().pending_receiver_packet.is_some() {
            self.set_state(KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
                selected_wallet: wallet,
            }));
            return Ok(());
        }

        self.set_state(KeyTeleportManagerState::SendAwaitReceiver);

        Ok(())
    }

    fn start_send_with_receiver_packet(
        &self,
        packet: Arc<KeyTeleportReceiverPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let eligible_wallets = eligible_wallets();
        if eligible_wallets.is_empty() {
            return Err(KeyTeleportAlert::NoEligibleWallets);
        }

        self.private.lock().pending_receiver_packet = Some(packet);
        if let Some(wallet) = self.private.lock().selected_send_wallet.clone() {
            self.set_state(KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
                selected_wallet: wallet,
            }));
            return Ok(());
        }

        self.set_state(KeyTeleportManagerState::SendChooseWallet(KeyTeleportSendChooseWallet {
            eligible_wallets,
        }));

        Ok(())
    }

    fn select_send_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let wallet = eligible_wallet_by_id(&wallet_id)?;
        self.private.lock().selected_send_wallet = Some(wallet.clone());
        self.set_state(KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
            selected_wallet: wallet,
        }));

        Ok(())
    }

    fn enter_receiver_code(&self, code: &str) -> Result<(), KeyTeleportAlert> {
        let code = NumericCode::from_str(code).map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        let (packet, wallet) = {
            let private = self.private.lock();
            let packet =
                private.pending_receiver_packet.clone().ok_or(KeyTeleportAlert::NoPendingSend)?;
            let wallet =
                private.selected_send_wallet.clone().ok_or(KeyTeleportAlert::IneligibleWallet)?;

            (packet, wallet)
        };

        SenderSession::new(packet.inner(), &code)
            .map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        self.private.lock().receiver_code = Some(code);

        let warns_passphrase_not_included = Keychain::global()
            .get_wallet_secret(&wallet.id)?
            .is_some_and(|secret| matches!(secret, WalletSecret::Mnemonic(_)));

        self.set_state(KeyTeleportManagerState::SendConfirm(KeyTeleportSendConfirm {
            selected_wallet: wallet,
            warns_passphrase_not_included,
        }));

        Ok(())
    }

    fn confirm_send_wallet(&self) -> Result<(), KeyTeleportAlert> {
        let (packet, wallet, code) = {
            let private = self.private.lock();
            let packet =
                private.pending_receiver_packet.clone().ok_or(KeyTeleportAlert::NoPendingSend)?;
            let wallet =
                private.selected_send_wallet.clone().ok_or(KeyTeleportAlert::IneligibleWallet)?;
            let code = private.receiver_code.clone().ok_or(KeyTeleportAlert::WrongReceiverCode)?;

            (packet, wallet, code)
        };

        let secret = Keychain::global()
            .get_wallet_secret(&wallet.id)?
            .ok_or(KeyTeleportAlert::IneligibleWallet)?;
        let sender = SenderSession::new(packet.inner(), &code)
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;
        let payload = match secret {
            WalletSecret::Mnemonic(mnemonic) => Payload::mnemonic(mnemonic),
            WalletSecret::Xpriv(xpriv) => {
                Payload::xprv(xpriv.expose()).map_err(|_| KeyTeleportAlert::InvalidPayload)?
            }
        };
        let response =
            sender.send(payload).map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;
        let state = KeyTeleportManagerState::SendReady(KeyTeleportSendReady {
            packet: Arc::new(KeyTeleportSenderPacket::new(response.packet)),
            password: Arc::new(KeyTeleportPassword::new(response.password)),
        });
        self.set_state(state);

        Ok(())
    }

    fn start_receive_password_entry(
        &self,
        packet: Arc<KeyTeleportSenderPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let session = self.active_receive_session()?;
        let receiver = session.receiver_session();
        receiver.decode_step1(packet.inner()).map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;

        self.private.lock().pending_sender_packet = Some(packet);
        self.set_state(KeyTeleportManagerState::ReceiveEnterPassword);

        Ok(())
    }

    fn enter_sender_password(&self, password: &str) -> Result<(), KeyTeleportAlert> {
        let session = self.active_receive_session()?;
        let packet = self
            .private
            .lock()
            .pending_sender_packet
            .clone()
            .ok_or(KeyTeleportAlert::NoPendingReceiveSecret)?;
        let password = TeleportPassword::from_str(password)
            .map_err(|_| KeyTeleportAlert::WrongTeleportPassword)?;
        let receiver = session.receiver_session();
        let decoded = receiver
            .decode(packet.inner(), &password)
            .map_err(KeyTeleportAlert::from_receive_decode_error)?;

        match decoded {
            DecodedPayload::Mnemonic(mnemonic) => {
                let word_count = mnemonic.words().count() as u32;
                self.private.lock().received_secret = Some(ReceivedSecret::Mnemonic(mnemonic));
                self.set_state(KeyTeleportManagerState::ReceiveMnemonicReview(
                    KeyTeleportMnemonicReview { word_count },
                ));
            }
            DecodedPayload::Xprv(xprv) => {
                self.private.lock().received_secret = Some(ReceivedSecret::Xprv(xprv));
                self.set_state(KeyTeleportManagerState::ReceiveXprvReview(KeyTeleportXprvReview {
                    revealed: false,
                }));
            }
            DecodedPayload::Notes(notes) => {
                self.private.lock().received_secret = None;
                self.set_state(KeyTeleportManagerState::ReceiveMessageReview(notes.into()));
            }
        }

        Ok(())
    }

    fn import_received_wallet(&self) -> Result<(), KeyTeleportAlert> {
        let session = self.active_receive_session()?;
        let secret = {
            let private = self.private.lock();
            let Some(secret) = private.received_secret.as_ref() else {
                return Err(KeyTeleportAlert::NoPendingReceiveSecret);
            };

            secret.to_wallet_secret()?
        };

        let metadata = import_key_teleport_wallet_secret_with_target(
            secret,
            session.network,
            session.wallet_mode,
        )
        .map_err(|error| KeyTeleportAlert::ImportFailed(error.to_string()))?;

        Keychain::global().delete_key_teleport_receive_session();
        self.clear_private();
        self.set_state(KeyTeleportManagerState::ReceiveImportedWallet(metadata));

        Ok(())
    }

    fn resume_receive_state(&self) -> Option<KeyTeleportManagerState> {
        let session = match self.load_receive_session() {
            Ok(Some(session)) => session,
            Ok(None) => return None,
            Err(error) => {
                error!("unable to load key teleport receive session: {error}");
                return None;
            }
        };

        if session.is_expired() {
            Keychain::global().delete_key_teleport_receive_session();
            return Some(KeyTeleportManagerState::Idle);
        }

        let active = ActiveReceiveSession::restore(&session).ok()?;
        let state = receive_state_from_session(&active).ok()?;
        self.private.lock().active_receive_session = Some(active);

        Some(KeyTeleportManagerState::ReceiveReady(state))
    }

    fn active_receive_session(&self) -> Result<ActiveReceiveSession, KeyTeleportAlert> {
        let session = {
            let private = self.private.lock();
            let Some(session) = private.active_receive_session.as_ref() else {
                return Err(KeyTeleportAlert::NoActiveReceiveSession);
            };

            session.try_clone()?
        };

        if session.is_expired() {
            Keychain::global().delete_key_teleport_receive_session();
            self.clear_private();
            self.set_state(KeyTeleportManagerState::Idle);
            return Err(KeyTeleportAlert::ReceiveSessionExpired);
        }

        Ok(session)
    }

    fn load_receive_session(&self) -> Result<Option<PersistedReceiveSession>, KeyTeleportAlert> {
        let Some(value) = Keychain::global()
            .get_key_teleport_receive_session()
            .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?
        else {
            return Ok(None);
        };

        let session = serde_json::from_str(&value).map_err(|error| {
            KeyTeleportAlert::Keychain(format!("unable to parse receive session: {error}"))
        })?;

        Ok(Some(session))
    }

    fn set_xprv_revealed(&self, revealed: bool) {
        let has_xprv = matches!(self.private.lock().received_secret, Some(ReceivedSecret::Xprv(_)));
        if has_xprv {
            self.set_state(KeyTeleportManagerState::ReceiveXprvReview(KeyTeleportXprvReview {
                revealed,
            }));
        }
    }

    fn set_state(&self, state: KeyTeleportManagerState) {
        *self.state.lock() = state.clone();
        self.reconciler.send(Message::UpdateState(state));
    }

    fn clear_private(&self) {
        *self.private.lock() = PrivateState::default();
    }
}

impl ActiveReceiveSession {
    fn new() -> Self {
        let database = Database::global();

        Self {
            receiver: ReceiverSession::new(),
            created_at_secs: now_secs(),
            network: database.global_config.selected_network(),
            wallet_mode: database.global_config.wallet_mode(),
        }
    }

    fn restore(persisted: &PersistedReceiveSession) -> Result<Self, KeyTeleportAlert> {
        let receiver = persisted.receiver_session()?;

        Ok(Self {
            receiver,
            created_at_secs: persisted.created_at_secs,
            network: persisted.network,
            wallet_mode: persisted.wallet_mode,
        })
    }

    fn try_clone(&self) -> Result<Self, KeyTeleportAlert> {
        let receiver = ReceiverSession::from_private_key_bytes(self.receiver.private_key_bytes())
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;

        Ok(Self {
            receiver,
            created_at_secs: self.created_at_secs,
            network: self.network,
            wallet_mode: self.wallet_mode,
        })
    }

    fn save(&self) -> Result<(), KeyTeleportAlert> {
        let mut private_key = self.receiver.private_key_bytes();
        let persisted = PersistedReceiveSession {
            private_key_hex: hex::encode(private_key),
            created_at_secs: self.created_at_secs,
            network: self.network,
            wallet_mode: self.wallet_mode,
        };
        private_key.zeroize();

        persisted.save()
    }

    fn receiver_session(&self) -> &ReceiverSession {
        &self.receiver
    }

    fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }
}

impl PersistedReceiveSession {
    fn save(&self) -> Result<(), KeyTeleportAlert> {
        let value = serde_json::to_string(self)
            .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?;

        Keychain::global()
            .save_key_teleport_receive_session(&value)
            .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))
    }

    fn receiver_session(&self) -> Result<ReceiverSession, KeyTeleportAlert> {
        let mut bytes = hex::decode(&self.private_key_hex)
            .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?;
        let mut private_key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeyTeleportAlert::Keychain("invalid receive private key length".into()))?;
        bytes.zeroize();

        let session = ReceiverSession::from_private_key_bytes(private_key)
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()));
        private_key.zeroize();

        session
    }

    fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }
}

fn receive_state_from_session(
    session: &ActiveReceiveSession,
) -> Result<KeyTeleportReceiveState, KeyTeleportAlert> {
    let request = session
        .receiver_session()
        .request()
        .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;

    Ok(KeyTeleportReceiveState {
        packet: Arc::new(KeyTeleportReceiverPacket::new(request.packet)),
        numeric_code: request.numeric_code.as_str().to_string(),
        grouped_numeric_code: request.numeric_code.grouped(),
        created_at_secs: session.created_at_secs,
        network: session.network,
        wallet_mode: session.wallet_mode,
    })
}

fn eligible_wallets() -> Vec<WalletMetadata> {
    let database = Database::global();
    let network = database.global_config.selected_network();
    let mode = database.global_config.wallet_mode();

    database
        .wallets
        .get_all(network, mode)
        .unwrap_or_default()
        .into_iter()
        .filter(is_send_eligible)
        .collect()
}

fn eligible_wallet_by_id(wallet_id: &WalletId) -> Result<WalletMetadata, KeyTeleportAlert> {
    eligible_wallets()
        .into_iter()
        .find(|wallet| wallet.id == *wallet_id)
        .ok_or(KeyTeleportAlert::IneligibleWallet)
}

pub(crate) fn is_send_eligible_wallet_id(wallet_id: &WalletId) -> bool {
    eligible_wallet_by_id(wallet_id).is_ok()
}

fn is_send_eligible(wallet: &WalletMetadata) -> bool {
    wallet.wallet_type == WalletType::Hot
        && Keychain::global().get_wallet_secret(&wallet.id).is_ok_and(|secret| secret.is_some())
}

fn now_secs() -> u64 {
    UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}

impl From<KeychainError> for KeyTeleportAlert {
    fn from(error: KeychainError) -> Self {
        Self::Keychain(error.to_string())
    }
}

impl From<database::Error> for KeyTeleportAlert {
    fn from(error: database::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<ImportWalletError> for KeyTeleportAlert {
    fn from(error: ImportWalletError) -> Self {
        Self::ImportFailed(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        str::FromStr as _,
        sync::{Arc, Once},
    };

    use cove_device::keychain::{KeychainAccess, KeychainError};

    use super::*;

    #[derive(Debug, Default)]
    struct TestKeychain(parking_lot::Mutex<HashMap<String, String>>);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.0.lock().remove(&key).is_some()
        }
    }

    fn init_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            crate::database::test_support::init_test_database();
            let _ = Keychain::new(Box::<TestKeychain>::default());
        });

        Keychain::global().delete_key_teleport_receive_session();
    }

    struct SendWalletFixture {
        wallet: WalletMetadata,
        original_wallets: Vec<WalletMetadata>,
    }

    impl SendWalletFixture {
        fn new() -> Self {
            let database = Database::global();
            let mut wallet = WalletMetadata::preview_new();
            wallet.network = database.global_config.selected_network();
            wallet.wallet_mode = database.global_config.wallet_mode();
            let original_wallets =
                database.wallets.get_all(wallet.network, wallet.wallet_mode).unwrap_or_default();
            let mnemonic = Mnemonic::from_str(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .unwrap();

            database
                .wallets
                .save_all_wallets(wallet.network, wallet.wallet_mode, vec![wallet.clone()])
                .unwrap();
            Keychain::global().save_wallet_key(&wallet.id, mnemonic).unwrap();

            Self { wallet, original_wallets }
        }
    }

    impl Drop for SendWalletFixture {
        fn drop(&mut self) {
            Keychain::global().delete_wallet_items(&self.wallet.id);
            Database::global()
                .wallets
                .save_all_wallets(
                    self.wallet.network,
                    self.wallet.wallet_mode,
                    self.original_wallets.clone(),
                )
                .unwrap();
        }
    }

    #[test]
    fn start_receive_resumes_session_and_restart_replaces_it() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        let first = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();

        manager.clone().dispatch(Action::StartReceive);
        let resumed = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        assert_eq!(first, resumed);

        manager.clone().dispatch(Action::RestartReceive);
        let restarted = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        assert_ne!(resumed, restarted);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn end_receive_deletes_session_and_returns_to_idle() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        manager.clone().dispatch(Action::EndReceive);

        assert!(matches!(manager.state(), KeyTeleportManagerState::Idle));
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
    }

    #[test]
    fn wrong_sender_password_keeps_receive_session_for_retry() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        let request = match manager.state() {
            KeyTeleportManagerState::ReceiveReady(state) => state,
            other => panic!("expected receive ready, got {other:?}"),
        };

        let sender = SenderSession::with_private_key_and_password(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
            [7; 32],
            TeleportPassword::from_bytes([1, 2, 3, 4, 5]),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic)).unwrap();
        let packet = Arc::new(KeyTeleportSenderPacket::new(response.packet));

        manager.clone().dispatch(Action::Ingest(StringOrData::String(packet.bbqr_part())));
        manager.clone().dispatch(Action::EnterSenderPassword("AAAAAAAA".to_string()));

        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveEnterPassword));
    }

    #[test]
    fn displayed_receive_request_remains_usable_if_resume_storage_disappears() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        let request = match manager.state() {
            KeyTeleportManagerState::ReceiveReady(state) => state,
            other => panic!("expected receive ready, got {other:?}"),
        };
        let sender = SenderSession::with_private_key_and_password(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
            [8; 32],
            TeleportPassword::from_bytes([1, 2, 3, 4, 5]),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic)).unwrap();
        let password = response.password.as_display_text();
        let packet = KeyTeleportSenderPacket::new(response.packet);

        Keychain::global().delete_key_teleport_receive_session();
        manager.handle_action(Action::Ingest(StringOrData::String(packet.bbqr_part()))).unwrap();
        manager.handle_action(Action::EnterSenderPassword(password)).unwrap();

        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
    }

    #[test]
    fn receive_decode_errors_preserve_failure_kind() {
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(KeyTeleportError::Checksum),
            KeyTeleportAlert::WrongTeleportPassword,
        );
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(KeyTeleportError::UnsupportedPayload(
                cove_keyteleport::UnsupportedPayloadKind::Vault,
            )),
            KeyTeleportAlert::UnsupportedPayload,
        );
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(KeyTeleportError::InvalidNotesPayload),
            KeyTeleportAlert::InvalidPayload,
        );
    }

    #[test]
    fn expired_receive_session_is_deleted_and_not_resumed() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let session = ReceiverSession::from_private_key_bytes([3; 32]).unwrap();
        let persisted = PersistedReceiveSession {
            private_key_hex: hex::encode(session.private_key_bytes()),
            created_at_secs: now_secs() - RECEIVE_SESSION_TTL.as_secs() - 1,
            network: Database::global().global_config.selected_network(),
            wallet_mode: Database::global().global_config.wallet_mode(),
        };
        persisted.save().unwrap();
        let manager = RustKeyTeleportManager::new();

        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());
        assert!(matches!(manager.state(), KeyTeleportManagerState::Idle));
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
    }

    #[test]
    fn sender_packet_without_active_receive_session_returns_clear_alert() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([4; 32]).unwrap();
        let request = receiver.request().unwrap();
        let sender = SenderSession::with_private_key_and_password(
            &request.packet,
            &request.numeric_code,
            [5; 32],
            TeleportPassword::from_bytes([1, 2, 3, 4, 5]),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic)).unwrap();
        let packet = KeyTeleportSenderPacket::new(response.packet);

        let error = manager.handle_action(Action::Ingest(StringOrData::String(packet.bbqr_part())));

        assert_eq!(error, Err(KeyTeleportAlert::NoActiveReceiveSession));
    }

    #[test]
    fn wallet_started_send_keeps_wallet_fixed_while_awaiting_receiver() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();

        manager.handle_action(Action::StartSendFromWallet(fixture.wallet.id.clone())).unwrap();

        let KeyTeleportManagerState::SendAwaitReceiver = manager.state() else {
            panic!("expected wallet-fixed send state")
        };

        assert_eq!(manager.private.lock().selected_send_wallet, Some(fixture.wallet.clone()));
    }

    #[test]
    fn receiver_started_send_requires_wallet_choice() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([10; 32]).unwrap();
        let request = receiver.request().unwrap();

        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();

        let KeyTeleportManagerState::SendChooseWallet(state) = manager.state() else {
            panic!("expected wallet choice state")
        };
        assert_eq!(state.eligible_wallets, vec![fixture.wallet.clone()]);
    }

    #[test]
    fn wrong_receiver_code_keeps_pending_send_for_retry() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([6; 32]).unwrap();
        let request = receiver.request().unwrap();
        let wrong_code = (0..100)
            .map(|value| format!("{value:08}"))
            .find(|code| {
                code != request.numeric_code.as_str()
                    && SenderSession::new(&request.packet, &NumericCode::from_str(code).unwrap())
                        .is_err()
            })
            .expect("test fixture should have at least one invalid wrong code");
        let wallet = WalletMetadata::preview_new();
        {
            let mut private = manager.private.lock();
            private.pending_receiver_packet =
                Some(Arc::new(KeyTeleportReceiverPacket::new(request.packet)));
            private.selected_send_wallet = Some(wallet);
        }

        let error = manager.enter_receiver_code(&wrong_code);
        let private = manager.private.lock();

        assert_eq!(error, Err(KeyTeleportAlert::WrongReceiverCode));
        assert!(private.pending_receiver_packet.is_some());
        assert!(private.selected_send_wallet.is_some());
        assert!(private.receiver_code.is_none());
    }

    #[test]
    fn send_eligibility_requires_hot_wallet_with_keychain_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let mut hot_wallet = WalletMetadata::preview_new();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();

        assert!(!is_send_eligible(&hot_wallet));

        Keychain::global().save_wallet_key(&hot_wallet.id, mnemonic.clone()).unwrap();
        assert!(is_send_eligible(&hot_wallet));

        let xpriv = bdk_wallet::bitcoin::bip32::Xpriv::new_master(
            bdk_wallet::bitcoin::Network::Bitcoin,
            &[9; 32],
        )
        .unwrap();
        Keychain::global().save_wallet_secret(&hot_wallet.id, xpriv.into()).unwrap();
        assert!(is_send_eligible(&hot_wallet));

        hot_wallet.wallet_type = WalletType::Cold;
        assert!(!is_send_eligible(&hot_wallet));

        Keychain::global().delete_wallet_items(&hot_wallet.id);
    }
}
