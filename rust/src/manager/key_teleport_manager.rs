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
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use tracing::{error, trace};
use zeroize::{Zeroize as _, Zeroizing};

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
    wallet_identity::{PublicWalletIdentity, PublicWalletIdentityError},
    wallet_secret::WalletSecretExt as _,
};

use super::deferred_sender::SingleOrMany;

type Message = KeyTeleportManagerReconcileMessage;
type Action = KeyTeleportManagerAction;
type Reconciler = dyn KeyTeleportManagerReconciler;

const RECEIVE_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
static RECEIVE_SESSION_STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum PacketDirection {
    Receiver,
    Sender,
}

#[uniffi::export(callback_interface)]
pub trait KeyTeleportManagerReconciler: Send + Sync + fmt::Debug + 'static {
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Debug, uniffi::Object)]
pub struct RustKeyTeleportManager {
    action_lock: Mutex<()>,
    model: Arc<Mutex<ManagerModel>>,
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
    Ingest(KeyTeleportInput),
    StartSendFromWallet(WalletId),
    SelectSendWallet(WalletId),
    EnterReceiverCode(String),
    EnterSenderPassword(String),
    /// Imports the received mnemonic or extended private key as a hot wallet
    ImportReceivedWallet,
    RevealXprv,
    HideXprv,
    FinishReview,
    Clear,
}

/// Validated or unparsed input for a KeyTeleport flow
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportInput {
    /// Text or bytes that still need protocol parsing
    MultiFormat(StringOrData),
    /// A receiver request already validated by the shared scanner
    Receiver(Arc<KeyTeleportReceiverPacket>),
    /// A sender response already validated by the shared scanner
    Sender(Arc<KeyTeleportSenderPacket>),
}

#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportManagerState {
    Idle,
    ReceiveReady(KeyTeleportReceiveState),
    /// Receive-session setup failed and can be retried
    ReceiveError,
    ReceiveEnterPassword,
    ReceiveMnemonicReview(KeyTeleportMnemonicReview),
    ReceiveXprvReview(KeyTeleportXprvReview),
    /// Displays received Secure Notes & Passwords content without treating it as a wallet
    ReceiveMessageReview(KeyTeleportMessageReview),
    /// Reports the wallet created from received private key material
    ReceiveImportedWallet(WalletMetadata),
    /// Reports that the received wallet already exists on this device
    ReceiveAlreadyImportedWallet(WalletMetadata),
    /// Waits for the receiver request after a sending wallet has been fixed
    SendAwaitReceiver,
    SendChooseWallet(KeyTeleportSendChooseWallet),
    SendEnterCode(KeyTeleportSendEnterCode),
    SendReady(KeyTeleportSendReady),
}

impl fmt::Debug for KeyTeleportManagerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::ReceiveReady(_) => f.write_str("ReceiveReady(****)"),
            Self::ReceiveError => f.write_str("ReceiveError"),
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
            Self::ReceiveAlreadyImportedWallet(wallet) => {
                f.debug_tuple("ReceiveAlreadyImportedWallet").field(&wallet.id).finish()
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

/// Display-ready Secure Notes & Passwords content received through KeyTeleport
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

/// An encrypted sender response ready to share with the receiver
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendReady {
    /// The wallet whose private key material is in the encrypted response
    pub selected_wallet: WalletMetadata,
    /// The encoded sender response
    pub packet: Arc<KeyTeleportSenderPacket>,
    /// The password needed to decrypt the sender response
    pub password: Arc<KeyTeleportPassword>,
}

impl fmt::Debug for KeyTeleportSendReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyTeleportSendReady")
            .field("selected_wallet", &self.selected_wallet)
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

    #[error("the previous receive request was unreadable and has been replaced")]
    ReceiveSessionReset,

    #[error("the receive request belongs to a different network or wallet mode")]
    ReceiveSessionScopeChanged,

    #[error("this packet conflicts with the active KeyTeleport transfer direction")]
    ConflictingTransferDirection,

    #[error("unable to parse KeyTeleport data")]
    ParseFailed,

    #[error("KeyTeleport PSBT packets are not supported yet")]
    UnsupportedPsbt,

    #[error("this KeyTeleport payload is not supported")]
    /// The payload uses a valid but unsupported protocol type
    UnsupportedPayload,

    #[error("the decrypted KeyTeleport payload is invalid")]
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
            | KeyTeleportError::UnsupportedMnemonicWordCount(_)
            | KeyTeleportError::InvalidXprvPayload
            | KeyTeleportError::NonMasterXprvPayload
            | KeyTeleportError::NonMainnetXprvPayload
            | KeyTeleportError::InvalidNotesPayload => Self::InvalidPayload,
            error => Self::Protocol(error.to_string()),
        }
    }
}

#[derive(Debug, Default)]
struct ManagerModel {
    generation: PhaseGeneration,
    phase: Phase,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct PhaseGeneration(u64);

impl PhaseGeneration {
    fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("KeyTeleport phase generation exhausted"))
    }
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    ReceiveError,
    ReceiveReady {
        session: ActiveReceiveSession,
        state: KeyTeleportReceiveState,
    },
    ReceiveEnterPassword {
        session: ActiveReceiveSession,
        packet: Arc<KeyTeleportSenderPacket>,
    },
    ReceiveMnemonicReview {
        session: ActiveReceiveSession,
        mnemonic: Mnemonic,
    },
    ReceiveXprvReview {
        session: ActiveReceiveSession,
        xprv: XprvPayload,
        revealed: bool,
    },
    ReceiveMessageReview {
        session: ActiveReceiveSession,
        review: KeyTeleportMessageReview,
    },
    ReceiveImported {
        session_id: ReceiveSessionId,
        wallet: WalletMetadata,
    },
    ReceiveAlreadyImported {
        session_id: ReceiveSessionId,
        wallet: WalletMetadata,
    },
    SendAwaitReceiver {
        wallet: WalletMetadata,
    },
    SendChooseWallet {
        packet: Arc<KeyTeleportReceiverPacket>,
        eligible_wallets: Vec<WalletMetadata>,
    },
    SendEnterCode {
        packet: Arc<KeyTeleportReceiverPacket>,
        wallet: WalletMetadata,
    },
    SendReady(KeyTeleportSendReady),
}

impl Phase {
    fn is_receive_flow_active(&self) -> bool {
        matches!(
            self,
            Self::ReceiveReady { .. }
                | Self::ReceiveEnterPassword { .. }
                | Self::ReceiveMnemonicReview { .. }
                | Self::ReceiveXprvReview { .. }
                | Self::ReceiveMessageReview { .. }
                | Self::ReceiveImported { .. }
                | Self::ReceiveAlreadyImported { .. }
        )
    }

    fn can_restore_receive_session(&self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::SendAwaitReceiver { .. }
                | Self::SendChooseWallet { .. }
                | Self::SendEnterCode { .. }
                | Self::SendReady(_)
        )
    }

    fn is_receive_direction(&self) -> bool {
        matches!(self, Self::ReceiveError) || self.is_receive_flow_active()
    }

    fn is_send_direction(&self) -> bool {
        matches!(
            self,
            Self::SendAwaitReceiver { .. }
                | Self::SendChooseWallet { .. }
                | Self::SendEnterCode { .. }
                | Self::SendReady(_)
        )
    }

    fn public_state(&self) -> KeyTeleportManagerState {
        match self {
            Self::Idle => KeyTeleportManagerState::Idle,
            Self::ReceiveError => KeyTeleportManagerState::ReceiveError,
            Self::ReceiveReady { state, .. } => {
                KeyTeleportManagerState::ReceiveReady(state.clone())
            }
            Self::ReceiveEnterPassword { .. } => KeyTeleportManagerState::ReceiveEnterPassword,
            Self::ReceiveMnemonicReview { mnemonic, .. } => {
                KeyTeleportManagerState::ReceiveMnemonicReview(KeyTeleportMnemonicReview {
                    word_count: mnemonic.word_count() as u32,
                })
            }
            Self::ReceiveXprvReview { revealed, .. } => {
                KeyTeleportManagerState::ReceiveXprvReview(KeyTeleportXprvReview {
                    revealed: *revealed,
                })
            }
            Self::ReceiveMessageReview { review, .. } => {
                KeyTeleportManagerState::ReceiveMessageReview(review.clone())
            }
            Self::ReceiveImported { wallet, .. } => {
                KeyTeleportManagerState::ReceiveImportedWallet(wallet.clone())
            }
            Self::ReceiveAlreadyImported { wallet, .. } => {
                KeyTeleportManagerState::ReceiveAlreadyImportedWallet(wallet.clone())
            }
            Self::SendAwaitReceiver { .. } => KeyTeleportManagerState::SendAwaitReceiver,
            Self::SendChooseWallet { eligible_wallets, .. } => {
                KeyTeleportManagerState::SendChooseWallet(KeyTeleportSendChooseWallet {
                    eligible_wallets: eligible_wallets.clone(),
                })
            }
            Self::SendEnterCode { wallet, .. } => {
                KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
                    selected_wallet: wallet.clone(),
                })
            }
            Self::SendReady(state) => KeyTeleportManagerState::SendReady(state.clone()),
        }
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

#[derive(Serialize, Deserialize)]
struct PersistedReceiveSession {
    #[serde(default)]
    session_id: Option<String>,
    private_key_hex: String,
    created_at_secs: u64,
    network: Network,
    wallet_mode: WalletMode,
}

impl fmt::Debug for PersistedReceiveSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedReceiveSession")
            .field("session_id", &self.session_id.as_ref().map(|_| "****"))
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
    id: ReceiveSessionId,
    receiver: ReceiverSession,
    created_at_secs: u64,
    network: Network,
    wallet_mode: WalletMode,
}

#[derive(Clone, PartialEq, Eq)]
struct ReceiveSessionId(String);

impl fmt::Debug for ReceiveSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceiveSessionId(****)")
    }
}

impl ReceiveSessionId {
    fn new() -> Self {
        Self(hex::encode(rand::rng().random::<[u8; 16]>()))
    }

    fn parse(value: String) -> Result<Self, KeyTeleportAlert> {
        let bytes = hex::decode(&value)
            .map_err(|error| KeyTeleportAlert::Keychain(format!("invalid session id: {error}")))?;
        if bytes.len() != 16 {
            return Err(KeyTeleportAlert::Keychain("invalid receive session id length".into()));
        }

        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReceiveScope {
    network: Network,
    wallet_mode: WalletMode,
}

impl ReceiveScope {
    fn current() -> Self {
        let config = &Database::global().global_config;

        Self { network: config.selected_network(), wallet_mode: config.wallet_mode() }
    }

    fn ensure_current(self) -> Result<(), KeyTeleportAlert> {
        if self == Self::current() {
            return Ok(());
        }

        Err(KeyTeleportAlert::ReceiveSessionScopeChanged)
    }
}

#[uniffi::export]
impl RustKeyTeleportManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            action_lock: Mutex::new(()),
            model: Arc::new(Mutex::new(ManagerModel::default())),
            reconciler: ReconcileChannel::new(20),
        })
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen_async(move |field| {
            trace!("KeyTeleport reconcile: {field:?}");
            match field {
                SingleOrMany::Single(message) => reconciler.reconcile(message),
                SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
            }
        });
    }

    #[uniffi::method]
    pub fn state(&self) -> KeyTeleportManagerState {
        self.model.lock().phase.public_state()
    }

    #[uniffi::method]
    pub fn reveal_mnemonic_words(&self) -> Vec<String> {
        let (session, mnemonic) = {
            let model = self.model.lock();
            let Phase::ReceiveMnemonicReview { session, mnemonic } = &model.phase else {
                return Vec::new();
            };

            let Ok(session) = session.try_clone() else {
                return Vec::new();
            };

            (session, mnemonic.clone())
        };
        if self.ensure_authoritative_receive_session(&session).is_err() {
            return Vec::new();
        }

        mnemonic.words().map(ToString::to_string).collect()
    }

    #[uniffi::method]
    pub fn reveal_xprv(&self) -> Option<String> {
        self.set_xprv_revealed(true).ok()?;

        let model = self.model.lock();
        let Phase::ReceiveXprvReview { xprv, .. } = &model.phase else {
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
        let _action_guard = self.action_lock.lock();

        match action {
            Action::StartReceive => self.start_receive(),
            Action::RestartReceive => self.restart_receive(),
            Action::EndReceive => self.end_receive(),
            Action::Ingest(input) => self.ingest(input),
            Action::StartSendFromWallet(wallet_id) => self.start_send_from_wallet(wallet_id),
            Action::SelectSendWallet(wallet_id) => self.select_send_wallet(wallet_id),
            Action::EnterReceiverCode(code) => self.enter_receiver_code(&code),
            Action::EnterSenderPassword(password) => self.enter_sender_password(&password),
            Action::ImportReceivedWallet => self.import_received_wallet(),
            Action::RevealXprv => self.set_xprv_revealed(true),
            Action::HideXprv => self.set_xprv_revealed(false),
            Action::FinishReview => self.end_receive(),
            Action::Clear => {
                self.set_phase(Phase::Idle);
                Ok(())
            }
        }
    }

    fn start_receive(&self) -> Result<(), KeyTeleportAlert> {
        let (receive_flow_active, active_session) = {
            let model = self.model.lock();
            if model.phase.is_send_direction() {
                return Err(KeyTeleportAlert::ConflictingTransferDirection);
            }

            (model.phase.is_receive_flow_active(), receive_session_from_phase(&model.phase)?)
        };
        if receive_flow_active {
            let Some(session) = active_session else {
                return Ok(());
            };
            if let Err(error) = self.ensure_authoritative_receive_session(&session) {
                self.set_phase(Phase::ReceiveError);
                return Err(error);
            }

            return Ok(());
        }

        match self.load_receive_session() {
            Ok(Some(existing)) if !existing.is_expired() => {
                match ActiveReceiveSession::restore(&existing) {
                    Ok(session) => {
                        if let Err(error) = session.ensure_current_scope() {
                            self.set_phase(Phase::ReceiveError);
                            return Err(error);
                        }

                        return self.activate_receive_session(session);
                    }
                    Err(error) => {
                        error!("unable to restore KeyTeleport receive session: {error}");
                        self.replace_receive_session(KeyTeleportAlert::ReceiveSessionReset)?;
                        return Ok(());
                    }
                }
            }
            Ok(Some(existing)) => {
                let session_id = existing.session_id()?;
                self.replace_receive_session_if_matches(
                    &session_id,
                    KeyTeleportAlert::ReceiveSessionExpired,
                )?;
                return Ok(());
            }
            Ok(None) => {}
            Err(error) => {
                error!("unable to load KeyTeleport receive session: {error}");
                self.replace_receive_session(KeyTeleportAlert::ReceiveSessionReset)?;
                return Ok(());
            }
        }

        let generation = self.invalidate_phase();
        self.create_receive_session(generation).inspect_err(|_| {
            self.set_phase_if_current(generation, Phase::ReceiveError);
        })
    }

    fn restart_receive(&self) -> Result<(), KeyTeleportAlert> {
        if self.model.lock().phase.is_send_direction() {
            return Err(KeyTeleportAlert::ConflictingTransferDirection);
        }

        let generation = self.invalidate_phase();
        self.delete_receive_session()?;

        self.create_receive_session(generation).inspect_err(|_| {
            self.set_phase_if_current(generation, Phase::ReceiveError);
        })
    }

    fn replace_receive_session(&self, alert: KeyTeleportAlert) -> Result<(), KeyTeleportAlert> {
        let generation = self.invalidate_phase();
        self.delete_receive_session()?;
        self.create_receive_session(generation).inspect_err(|_| {
            self.set_phase_if_current(generation, Phase::ReceiveError);
        })?;
        self.reconciler.send(Message::SetAlert(alert));

        Ok(())
    }

    fn replace_receive_session_if_matches(
        &self,
        session_id: &ReceiveSessionId,
        alert: KeyTeleportAlert,
    ) -> Result<(), KeyTeleportAlert> {
        let generation = self.invalidate_phase();
        self.delete_receive_session_if_matches(session_id)?;
        self.create_receive_session(generation).inspect_err(|_| {
            self.set_phase_if_current(generation, Phase::ReceiveError);
        })?;
        self.reconciler.send(Message::SetAlert(alert));

        Ok(())
    }

    fn create_receive_session(&self, generation: PhaseGeneration) -> Result<(), KeyTeleportAlert> {
        let session = ActiveReceiveSession::new();
        session.save()?;
        let session_id = session.id.clone();

        if self.activate_receive_session_if_current(generation, session)? {
            return Ok(());
        }

        if let Err(error) = self.delete_receive_session_if_matches(&session_id)
            && error != KeyTeleportAlert::NoActiveReceiveSession
        {
            return Err(error);
        }

        Ok(())
    }

    fn activate_receive_session(
        &self,
        session: ActiveReceiveSession,
    ) -> Result<(), KeyTeleportAlert> {
        let state = receive_state_from_session(&session)?;
        self.set_phase(Phase::ReceiveReady { session, state });

        Ok(())
    }

    fn activate_receive_session_if_current(
        &self,
        generation: PhaseGeneration,
        session: ActiveReceiveSession,
    ) -> Result<bool, KeyTeleportAlert> {
        let state = receive_state_from_session(&session)?;

        Ok(self.set_phase_if_current(generation, Phase::ReceiveReady { session, state }))
    }

    fn end_receive(&self) -> Result<(), KeyTeleportAlert> {
        let session_id = {
            let model = self.model.lock();
            if model.phase.is_send_direction() {
                return Err(KeyTeleportAlert::ConflictingTransferDirection);
            }

            receive_session_id_from_phase(&model.phase)
        }?;
        let generation = self.invalidate_phase();

        match session_id {
            Some(session_id) => {
                if let Err(error) = self.delete_receive_session_if_matches(&session_id)
                    && error != KeyTeleportAlert::NoActiveReceiveSession
                {
                    return Err(error);
                }
            }
            None => self.delete_receive_session()?,
        }
        self.set_phase_if_current(generation, Phase::Idle);

        Ok(())
    }

    fn ingest(&self, input: KeyTeleportInput) -> Result<(), KeyTeleportAlert> {
        let parsed = match input {
            KeyTeleportInput::Receiver(packet) => {
                self.ensure_ingest_direction(PacketDirection::Receiver)?;
                return self.start_send_with_receiver_packet(packet);
            }
            KeyTeleportInput::Sender(packet) => {
                self.ensure_ingest_direction(PacketDirection::Sender)?;
                return self.start_receive_password_entry(packet);
            }
            KeyTeleportInput::MultiFormat(input) => {
                crate::key_teleport::parse_key_teleport_input(input)
            }
        };

        match parsed {
            Ok(crate::key_teleport::ParsedKeyTeleport::Receiver(packet)) => {
                self.ensure_ingest_direction(PacketDirection::Receiver)?;
                self.start_send_with_receiver_packet(packet)
            }
            Ok(crate::key_teleport::ParsedKeyTeleport::Sender(packet)) => {
                self.ensure_ingest_direction(PacketDirection::Sender)?;
                self.start_receive_password_entry(packet)
            }
            Ok(crate::key_teleport::ParsedKeyTeleport::UnsupportedPsbt) => {
                Err(KeyTeleportAlert::UnsupportedPsbt)
            }
            Err(crate::key_teleport::KeyTeleportParseError::Unrecognized) => {
                Err(KeyTeleportAlert::ParseFailed)
            }
        }
    }

    fn ensure_ingest_direction(
        &self,
        packet_direction: PacketDirection,
    ) -> Result<(), KeyTeleportAlert> {
        let phase = &self.model.lock().phase;
        let conflicts = match packet_direction {
            PacketDirection::Receiver => phase.is_receive_direction(),
            PacketDirection::Sender => phase.is_send_direction(),
        };
        if conflicts {
            return Err(KeyTeleportAlert::ConflictingTransferDirection);
        }

        Ok(())
    }

    fn start_send_from_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let (generation, packet) = {
            let model = self.model.lock();
            if model.phase.is_receive_direction() {
                return Err(KeyTeleportAlert::ConflictingTransferDirection);
            }

            let packet = match &model.phase {
                Phase::SendChooseWallet { packet, .. } => Some(packet.clone()),
                _ => None,
            };

            (model.generation, packet)
        };
        let wallet = eligible_wallet_by_id(&wallet_id)?;

        let phase = match packet {
            Some(packet) => Phase::SendEnterCode { packet, wallet },
            None => Phase::SendAwaitReceiver { wallet },
        };
        if !self.set_phase_if_current(generation, phase) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    fn start_send_with_receiver_packet(
        &self,
        packet: Arc<KeyTeleportReceiverPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let generation = self.current_generation();
        let eligible_wallets = eligible_wallets()?;
        if eligible_wallets.is_empty() {
            return Err(KeyTeleportAlert::NoEligibleWallets);
        }

        let selected_wallet = {
            let model = self.model.lock();
            match &model.phase {
                Phase::SendAwaitReceiver { wallet } => Some(wallet.clone()),
                _ => None,
            }
        };
        if let Some(wallet) = selected_wallet {
            if !self.set_phase_if_current(generation, Phase::SendEnterCode { packet, wallet }) {
                return Err(KeyTeleportAlert::NoPendingSend);
            }

            return Ok(());
        }

        if !self
            .set_phase_if_current(generation, Phase::SendChooseWallet { packet, eligible_wallets })
        {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    fn select_send_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let wallet = eligible_wallet_by_id(&wallet_id)?;
        let (generation, packet) = {
            let model = self.model.lock();
            let Phase::SendChooseWallet { packet, .. } = &model.phase else {
                return Err(KeyTeleportAlert::NoPendingSend);
            };

            (model.generation, packet.clone())
        };
        if !self.set_phase_if_current(generation, Phase::SendEnterCode { packet, wallet }) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    fn enter_receiver_code(&self, code: &str) -> Result<(), KeyTeleportAlert> {
        let code = NumericCode::from_str(code).map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        let (generation, packet, wallet) = {
            let model = self.model.lock();
            let Phase::SendEnterCode { packet, wallet } = &model.phase else {
                return Err(KeyTeleportAlert::NoPendingSend);
            };

            (model.generation, packet.clone(), wallet.clone())
        };

        let sender = SenderSession::new(packet.inner(), &code)
            .map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        let wallet = current_send_wallet(&wallet.id)?;
        let keychain = Keychain::global();
        let secret =
            keychain.get_wallet_secret(&wallet.id)?.ok_or(KeyTeleportAlert::IneligibleWallet)?;
        if !is_supported_send_secret(&secret) {
            return Err(KeyTeleportAlert::IneligibleWallet);
        }
        let incoming_descriptors =
            secret.clone().into_descriptors(wallet.network, wallet.address_type);
        let incoming_identity = PublicWalletIdentity::from_descriptors(&incoming_descriptors);
        let existing_identity = PublicWalletIdentity::from_existing_wallet(&wallet, keychain)?
            .ok_or(KeyTeleportAlert::IneligibleWallet)?;
        if incoming_identity != existing_identity {
            return Err(KeyTeleportAlert::IneligibleWallet);
        }

        let payload = match secret {
            WalletSecret::Mnemonic(mnemonic) => Payload::mnemonic(mnemonic),
            WalletSecret::Xpriv(xpriv) => Payload::xprv(xpriv.expose()),
        }
        .map_err(|_| KeyTeleportAlert::InvalidPayload)?;
        let response =
            sender.send(payload).map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;
        let state = KeyTeleportSendReady {
            selected_wallet: wallet,
            packet: Arc::new(KeyTeleportSenderPacket::new(response.packet)),
            password: Arc::new(KeyTeleportPassword::new(response.password)),
        };
        if !self.set_phase_if_current(generation, Phase::SendReady(state)) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    fn start_receive_password_entry(
        &self,
        packet: Arc<KeyTeleportSenderPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let (generation, session) = self.take_receive_ready_session()?;
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
        ensure_authoritative_receive_session_unlocked(&session)?;
        let receiver = session.receiver_session();
        receiver.decode_step1(packet.inner()).map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;

        if !self.set_phase_if_current(generation, Phase::ReceiveEnterPassword { session, packet }) {
            return Err(KeyTeleportAlert::NoActiveReceiveSession);
        }

        Ok(())
    }

    fn enter_sender_password(&self, password: &str) -> Result<(), KeyTeleportAlert> {
        let (generation, session, packet) = self.take_receive_password_phase()?;
        let password = TeleportPassword::from_str(password)
            .map_err(|_| KeyTeleportAlert::WrongTeleportPassword)?;
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
        ensure_authoritative_receive_session_unlocked(&session)?;
        let receiver = session.receiver_session();
        let decoded = receiver
            .decode(packet.inner(), &password)
            .map_err(KeyTeleportAlert::from_receive_decode_error)?;

        let phase = match decoded {
            DecodedPayload::Mnemonic(mnemonic) => {
                Phase::ReceiveMnemonicReview { session, mnemonic }
            }
            DecodedPayload::Xprv(xprv) => {
                Phase::ReceiveXprvReview { session, xprv, revealed: false }
            }
            DecodedPayload::Notes(notes) => {
                Phase::ReceiveMessageReview { session, review: notes.into() }
            }
        };
        if !self.set_phase_if_current(generation, phase) {
            return Err(KeyTeleportAlert::NoPendingReceiveSecret);
        }

        Ok(())
    }

    fn import_received_wallet(&self) -> Result<(), KeyTeleportAlert> {
        let (generation, session, secret) = {
            let model = self.model.lock();
            match &model.phase {
                Phase::ReceiveMnemonicReview { session, mnemonic } => (
                    model.generation,
                    session.try_clone()?,
                    ReceivedSecret::Mnemonic(mnemonic.clone()).to_wallet_secret()?,
                ),
                Phase::ReceiveXprvReview { session, xprv, .. } => (
                    model.generation,
                    session.try_clone()?,
                    ReceivedSecret::Xprv(xprv.clone()).to_wallet_secret()?,
                ),
                _ => return Err(KeyTeleportAlert::NoPendingReceiveSecret),
            }
        };
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
        ensure_authoritative_receive_session_unlocked(&session)?;

        let result = import_key_teleport_wallet_secret_with_target(
            secret,
            session.network,
            session.wallet_mode,
        );

        match result {
            Ok(metadata) => {
                delete_receive_session_if_matches_unlocked(&session.id)?;
                let phase = Phase::ReceiveImported { session_id: session.id, wallet: metadata };
                if !self.set_phase_if_current(generation, phase) {
                    return Err(KeyTeleportAlert::NoPendingReceiveSecret);
                }
            }
            Err(ImportWalletError::WalletAlreadyExists(id)) => {
                let metadata = Database::global()
                    .wallets
                    .get(&id, session.network, session.wallet_mode)?
                    .ok_or_else(|| {
                        KeyTeleportAlert::ImportFailed(
                            ImportWalletError::MissingMetadata(id).to_string(),
                        )
                    })?;
                delete_receive_session_if_matches_unlocked(&session.id)?;
                let phase =
                    Phase::ReceiveAlreadyImported { session_id: session.id, wallet: metadata };
                if !self.set_phase_if_current(generation, phase) {
                    return Err(KeyTeleportAlert::NoPendingReceiveSecret);
                }
            }
            Err(error) => return Err(KeyTeleportAlert::ImportFailed(error.to_string())),
        }

        Ok(())
    }

    fn load_receive_session(&self) -> Result<Option<PersistedReceiveSession>, KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        load_receive_session_unlocked()
    }

    fn set_xprv_revealed(&self, revealed: bool) -> Result<(), KeyTeleportAlert> {
        let session = {
            let model = self.model.lock();
            let Phase::ReceiveXprvReview { session, .. } = &model.phase else {
                return Ok(());
            };

            session.try_clone()?
        };
        self.ensure_authoritative_receive_session(&session)?;

        let state = {
            let mut model = self.model.lock();
            let Phase::ReceiveXprvReview { revealed: current, .. } = &mut model.phase else {
                return Ok(());
            };
            *current = revealed;
            model.phase.public_state()
        };
        self.reconciler.send(Message::UpdateState(state));

        Ok(())
    }

    fn set_phase(&self, phase: Phase) {
        let state = {
            let mut model = self.model.lock();
            model.generation = model.generation.next();
            model.phase = phase;
            model.phase.public_state()
        };
        self.reconciler.send(Message::UpdateState(state));
    }

    fn set_phase_if_current(&self, generation: PhaseGeneration, phase: Phase) -> bool {
        let state = {
            let mut model = self.model.lock();
            if model.generation != generation {
                return false;
            }

            model.generation = model.generation.next();
            model.phase = phase;
            model.phase.public_state()
        };
        self.reconciler.send(Message::UpdateState(state));

        true
    }

    fn invalidate_phase(&self) -> PhaseGeneration {
        let mut model = self.model.lock();
        model.generation = model.generation.next();

        model.generation
    }

    fn current_generation(&self) -> PhaseGeneration {
        self.model.lock().generation
    }

    fn take_receive_ready_session(
        &self,
    ) -> Result<(PhaseGeneration, ActiveReceiveSession), KeyTeleportAlert> {
        let (generation, active_session) = {
            let model = self.model.lock();
            let active_session = match &model.phase {
                Phase::ReceiveReady { session, .. } => Some(session.try_clone()?),
                phase if phase.can_restore_receive_session() => None,
                _ => return Err(KeyTeleportAlert::NoActiveReceiveSession),
            };

            (model.generation, active_session)
        };
        let session = match active_session {
            Some(session) => session,
            None => {
                let persisted =
                    self.load_receive_session()?.ok_or(KeyTeleportAlert::NoActiveReceiveSession)?;
                ActiveReceiveSession::restore(&persisted)?
            }
        };

        self.ensure_receive_session_fresh(session).map(|session| (generation, session))
    }

    fn take_receive_password_phase(
        &self,
    ) -> Result<
        (PhaseGeneration, ActiveReceiveSession, Arc<KeyTeleportSenderPacket>),
        KeyTeleportAlert,
    > {
        let (generation, session, packet) = {
            let model = self.model.lock();
            let Phase::ReceiveEnterPassword { session, packet } = &model.phase else {
                return Err(KeyTeleportAlert::NoPendingReceiveSecret);
            };

            (model.generation, session.try_clone()?, packet.clone())
        };

        self.ensure_receive_session_fresh(session).map(|session| (generation, session, packet))
    }

    fn ensure_receive_session_fresh(
        &self,
        session: ActiveReceiveSession,
    ) -> Result<ActiveReceiveSession, KeyTeleportAlert> {
        session.ensure_current_scope()?;
        self.ensure_authoritative_receive_session(&session)?;
        if session.is_expired() {
            self.replace_receive_session_if_matches(
                &session.id,
                KeyTeleportAlert::ReceiveSessionExpired,
            )?;
            return Err(KeyTeleportAlert::ReceiveSessionExpired);
        }

        Ok(session)
    }

    fn ensure_authoritative_receive_session(
        &self,
        session: &ActiveReceiveSession,
    ) -> Result<(), KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        ensure_authoritative_receive_session_unlocked(session)
    }

    fn delete_receive_session(&self) -> Result<(), KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        delete_receive_session_unlocked()
    }

    fn delete_receive_session_if_matches(
        &self,
        session_id: &ReceiveSessionId,
    ) -> Result<(), KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        delete_receive_session_if_matches_unlocked(session_id)
    }
}

impl ActiveReceiveSession {
    fn new() -> Self {
        let scope = ReceiveScope::current();

        Self {
            id: ReceiveSessionId::new(),
            receiver: ReceiverSession::new(),
            created_at_secs: now_secs(),
            network: scope.network,
            wallet_mode: scope.wallet_mode,
        }
    }

    fn restore(persisted: &PersistedReceiveSession) -> Result<Self, KeyTeleportAlert> {
        let receiver = persisted.receiver_session()?;

        Ok(Self {
            id: persisted.session_id()?,
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
            id: self.id.clone(),
            receiver,
            created_at_secs: self.created_at_secs,
            network: self.network,
            wallet_mode: self.wallet_mode,
        })
    }

    fn save(&self) -> Result<(), KeyTeleportAlert> {
        let mut private_key = self.receiver.private_key_bytes();
        let persisted = PersistedReceiveSession {
            session_id: Some(self.id.0.clone()),
            private_key_hex: hex::encode(private_key),
            created_at_secs: self.created_at_secs,
            network: self.network,
            wallet_mode: self.wallet_mode,
        };
        private_key.zeroize();

        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        persisted.save_unlocked()
    }

    fn receiver_session(&self) -> &ReceiverSession {
        &self.receiver
    }

    fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }

    fn scope(&self) -> ReceiveScope {
        ReceiveScope { network: self.network, wallet_mode: self.wallet_mode }
    }

    fn ensure_current_scope(&self) -> Result<(), KeyTeleportAlert> {
        self.scope().ensure_current()
    }
}

impl PersistedReceiveSession {
    fn save_unlocked(&self) -> Result<(), KeyTeleportAlert> {
        let value = Zeroizing::new(
            serde_json::to_string(self)
                .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?,
        );

        Keychain::global()
            .save_key_teleport_receive_session(&value)
            .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))
    }

    fn session_id(&self) -> Result<ReceiveSessionId, KeyTeleportAlert> {
        let value = self
            .session_id
            .clone()
            .ok_or_else(|| KeyTeleportAlert::Keychain("receive session id is missing".into()))?;

        ReceiveSessionId::parse(value)
    }

    fn receiver_session(&self) -> Result<ReceiverSession, KeyTeleportAlert> {
        let bytes = Zeroizing::new(
            hex::decode(&self.private_key_hex)
                .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?,
        );
        let mut private_key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeyTeleportAlert::Keychain("invalid receive private key length".into()))?;
        let session = ReceiverSession::from_private_key_bytes(private_key)
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()));
        private_key.zeroize();

        session
    }

    fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }

    fn scope(&self) -> ReceiveScope {
        ReceiveScope { network: self.network, wallet_mode: self.wallet_mode }
    }
}

fn load_receive_session_unlocked() -> Result<Option<PersistedReceiveSession>, KeyTeleportAlert> {
    let Some(value) = Keychain::global()
        .get_key_teleport_receive_session()
        .map_err(|error| KeyTeleportAlert::Keychain(error.to_string()))?
    else {
        return Ok(None);
    };

    let mut session: PersistedReceiveSession = serde_json::from_str(&value).map_err(|error| {
        KeyTeleportAlert::Keychain(format!("unable to parse receive session: {error}"))
    })?;
    if session.session_id.is_none() {
        session.session_id = Some(ReceiveSessionId::new().0);
        session.save_unlocked()?;
    } else {
        session.session_id()?;
    }

    Ok(Some(session))
}

fn ensure_authoritative_receive_session_unlocked(
    session: &ActiveReceiveSession,
) -> Result<(), KeyTeleportAlert> {
    session.ensure_current_scope()?;
    let persisted =
        load_receive_session_unlocked()?.ok_or(KeyTeleportAlert::NoActiveReceiveSession)?;
    persisted.scope().ensure_current()?;
    if persisted.session_id()? != session.id {
        return Err(KeyTeleportAlert::NoActiveReceiveSession);
    }

    Ok(())
}

fn delete_receive_session_unlocked() -> Result<(), KeyTeleportAlert> {
    if Keychain::global().delete_key_teleport_receive_session() {
        return Ok(());
    }

    Err(KeyTeleportAlert::Keychain("unable to delete KeyTeleport receive session".into()))
}

fn delete_receive_session_if_matches_unlocked(
    session_id: &ReceiveSessionId,
) -> Result<(), KeyTeleportAlert> {
    let Some(persisted) = load_receive_session_unlocked()? else {
        return Ok(());
    };
    if persisted.session_id()? != *session_id {
        return Err(KeyTeleportAlert::NoActiveReceiveSession);
    }

    delete_receive_session_unlocked()
}

fn receive_session_from_phase(
    phase: &Phase,
) -> Result<Option<ActiveReceiveSession>, KeyTeleportAlert> {
    let session = match phase {
        Phase::ReceiveReady { session, .. }
        | Phase::ReceiveEnterPassword { session, .. }
        | Phase::ReceiveMnemonicReview { session, .. }
        | Phase::ReceiveXprvReview { session, .. }
        | Phase::ReceiveMessageReview { session, .. } => session,
        _ => return Ok(None),
    };

    session.try_clone().map(Some)
}

fn receive_session_id_from_phase(
    phase: &Phase,
) -> Result<Option<ReceiveSessionId>, KeyTeleportAlert> {
    match phase {
        Phase::ReceiveImported { session_id, .. }
        | Phase::ReceiveAlreadyImported { session_id, .. } => Ok(Some(session_id.clone())),
        _ => receive_session_from_phase(phase).map(|session| session.map(|value| value.id)),
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

fn eligible_wallets() -> Result<Vec<WalletMetadata>, KeyTeleportAlert> {
    let database = Database::global();
    let network = database.global_config.selected_network();
    let mode = database.global_config.wallet_mode();

    database.wallets.get_all(network, mode)?.into_iter().try_fold(
        Vec::new(),
        |mut eligible, wallet| {
            if is_send_eligible(&wallet)? {
                eligible.push(wallet);
            }

            Ok(eligible)
        },
    )
}

fn eligible_wallet_by_id(wallet_id: &WalletId) -> Result<WalletMetadata, KeyTeleportAlert> {
    let database = Database::global();
    let network = database.global_config.selected_network();
    let mode = database.global_config.wallet_mode();
    let wallet = database
        .wallets
        .get(wallet_id, network, mode)?
        .ok_or(KeyTeleportAlert::IneligibleWallet)?;
    if !is_send_eligible(&wallet)? {
        return Err(KeyTeleportAlert::IneligibleWallet);
    }

    Ok(wallet)
}

fn current_send_wallet(wallet_id: &WalletId) -> Result<WalletMetadata, KeyTeleportAlert> {
    let database = Database::global();
    let scope = ReceiveScope::current();
    let wallet = database
        .wallets
        .get(wallet_id, scope.network, scope.wallet_mode)?
        .ok_or(KeyTeleportAlert::IneligibleWallet)?;
    if wallet.wallet_type != WalletType::Hot {
        return Err(KeyTeleportAlert::IneligibleWallet);
    }

    Ok(wallet)
}

pub(crate) fn is_send_eligible_wallet_id(wallet_id: &WalletId) -> bool {
    match eligible_wallet_by_id(wallet_id) {
        Ok(_) => true,
        Err(KeyTeleportAlert::IneligibleWallet) => false,
        Err(error) => {
            tracing::warn!("unable to determine KeyTeleport send eligibility: {error}");
            false
        }
    }
}

fn is_send_eligible(wallet: &WalletMetadata) -> Result<bool, KeyTeleportAlert> {
    if wallet.wallet_type != WalletType::Hot {
        return Ok(false);
    }

    let Some(secret) = Keychain::global().get_wallet_secret(&wallet.id)? else {
        return Ok(false);
    };
    Ok(is_supported_send_secret(&secret))
}

fn is_supported_send_secret(secret: &WalletSecret) -> bool {
    match secret {
        WalletSecret::Mnemonic(mnemonic) => matches!(mnemonic.word_count(), 12 | 18 | 24),
        WalletSecret::Xpriv(_) => true,
    }
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

impl From<PublicWalletIdentityError> for KeyTeleportAlert {
    fn from(error: PublicWalletIdentityError) -> Self {
        match error {
            PublicWalletIdentityError::Keychain(error) => error.into(),
            error => Self::Protocol(error.to_string()),
        }
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
        sync::{
            Arc, Once,
            atomic::{AtomicBool, Ordering},
        },
    };

    use crate::wallet_secret::WalletSecretExt as _;
    use cove_device::keychain::{KeychainAccess, KeychainError};

    use super::*;

    #[derive(Debug, Default)]
    struct TestKeychain(parking_lot::Mutex<HashMap<String, String>>);

    static FAIL_KEYCHAIN_DELETES: AtomicBool = AtomicBool::new(false);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            if FAIL_KEYCHAIN_DELETES.load(Ordering::Relaxed) {
                return false;
            }

            self.0.lock().remove(&key).is_some()
        }
    }

    fn init_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            crate::database::test_support::init_test_database();
            let _ = Keychain::new(Box::<TestKeychain>::default());
        });

        FAIL_KEYCHAIN_DELETES.store(false, Ordering::Relaxed);
        Keychain::global().delete_key_teleport_receive_session();
    }

    struct SendWalletFixture {
        wallet: WalletMetadata,
        original_wallets: Vec<WalletMetadata>,
    }

    impl SendWalletFixture {
        fn new() -> Self {
            let mnemonic = Mnemonic::from_str(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .unwrap();

            Self::with_secret(WalletSecret::Mnemonic(mnemonic))
        }

        fn with_secret(secret: WalletSecret) -> Self {
            let database = Database::global();
            let mut wallet = WalletMetadata::preview_new();
            wallet.network = database.global_config.selected_network();
            wallet.wallet_mode = database.global_config.wallet_mode();
            wallet.master_fingerprint =
                Some(Arc::new(secret.xpub(wallet.network).fingerprint().into()));
            let original_wallets =
                database.wallets.get_all(wallet.network, wallet.wallet_mode).unwrap_or_default();

            database
                .wallets
                .save_all_wallets(wallet.network, wallet.wallet_mode, vec![wallet.clone()])
                .unwrap();
            let descriptors = secret.clone().into_descriptors(wallet.network, wallet.address_type);
            Keychain::global().save_wallet_xpub(&wallet.id, secret.xpub(wallet.network)).unwrap();
            Keychain::global()
                .save_public_descriptor(
                    &wallet.id,
                    descriptors.external.extended_descriptor,
                    descriptors.internal.extended_descriptor,
                )
                .unwrap();
            Keychain::global().save_wallet_secret(&wallet.id, secret).unwrap();

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

    fn receive_request(manager: &RustKeyTeleportManager) -> KeyTeleportReceiveState {
        let KeyTeleportManagerState::ReceiveReady(state) = manager.state() else {
            panic!("expected receive ready")
        };

        state
    }

    fn sender_transfer(
        request: &KeyTeleportReceiveState,
    ) -> (Arc<KeyTeleportSenderPacket>, String) {
        let sender = SenderSession::new(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();

        (
            Arc::new(KeyTeleportSenderPacket::new(response.packet)),
            response.password.as_display_text(),
        )
    }

    fn sender_packet(request: &KeyTeleportReceiveState) -> Arc<KeyTeleportSenderPacket> {
        sender_transfer(request).0
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
        assert_eq!(*first, *resumed);

        manager.clone().dispatch(Action::RestartReceive);
        let restarted = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        assert_ne!(*resumed, *restarted);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn legacy_receive_session_json_is_migrated_without_changing_the_request() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let receiver = ReceiverSession::from_private_key_bytes([21; 32]).unwrap();
        let scope = ReceiveScope::current();
        let legacy = serde_json::json!({
            "private_key_hex": hex::encode(receiver.private_key_bytes()),
            "created_at_secs": now_secs(),
            "network": scope.network,
            "wallet_mode": scope.wallet_mode,
        });
        Keychain::global().save_key_teleport_receive_session(&legacy.to_string()).unwrap();
        let expected_packet = receiver.request().unwrap().packet;
        let manager = RustKeyTeleportManager::new();

        manager.start_receive().unwrap();

        let request = receive_request(&manager);
        assert_eq!(request.packet.inner(), &expected_packet);
        let migrated = manager.load_receive_session().unwrap().unwrap();
        assert!(migrated.session_id().is_ok());
        assert_eq!(migrated.private_key_hex, legacy["private_key_hex"]);
    }

    #[test]
    fn another_manager_restart_invalidates_the_displayed_receive_request() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let first_manager = RustKeyTeleportManager::new();
        first_manager.start_receive().unwrap();
        let stale_packet = sender_packet(&receive_request(&first_manager));
        let second_manager = RustKeyTeleportManager::new();

        second_manager.restart_receive().unwrap();
        let error = first_manager.ingest(KeyTeleportInput::Sender(stale_packet));

        assert_eq!(error, Err(KeyTeleportAlert::NoActiveReceiveSession));
        assert!(matches!(second_manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn another_manager_end_invalidates_the_displayed_receive_request() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let first_manager = RustKeyTeleportManager::new();
        first_manager.start_receive().unwrap();
        let stale_packet = sender_packet(&receive_request(&first_manager));
        let second_manager = RustKeyTeleportManager::new();

        second_manager.end_receive().unwrap();
        let error = first_manager.ingest(KeyTeleportInput::Sender(stale_packet));

        assert_eq!(error, Err(KeyTeleportAlert::NoActiveReceiveSession));
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
    }

    #[test]
    fn stale_phase_generation_cannot_commit_after_clear_end_or_restart() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        let before_clear = manager.current_generation();
        manager.clone().handle_action(Action::Clear).unwrap();
        assert!(!manager.set_phase_if_current(before_clear, Phase::ReceiveError));

        manager.start_receive().unwrap();
        let before_end = manager.current_generation();
        manager.end_receive().unwrap();
        assert!(!manager.set_phase_if_current(before_end, Phase::ReceiveError));

        manager.start_receive().unwrap();
        let before_restart = manager.current_generation();
        manager.restart_receive().unwrap();
        assert!(!manager.set_phase_if_current(before_restart, Phase::ReceiveError));
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn corrupt_receive_session_is_replaced_with_a_usable_request() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        Keychain::global().save_key_teleport_receive_session("{").unwrap();
        let corrupt = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);

        let replacement = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        assert_ne!(*replacement, *corrupt);
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
    fn end_receive_propagates_keychain_deletion_failure() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();
        manager.start_receive().unwrap();
        FAIL_KEYCHAIN_DELETES.store(true, Ordering::Relaxed);

        let error = manager.end_receive();

        FAIL_KEYCHAIN_DELETES.store(false, Ordering::Relaxed);
        assert!(matches!(error, Err(KeyTeleportAlert::Keychain(_))));
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());
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

        let sender = SenderSession::new(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let packet = Arc::new(KeyTeleportSenderPacket::new(response.packet));

        manager.clone().dispatch(Action::Ingest(KeyTeleportInput::Sender(packet)));
        manager.clone().dispatch(Action::EnterSenderPassword("AAAAAAAA".to_string()));

        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveEnterPassword));
    }

    #[test]
    fn displayed_receive_request_is_invalidated_if_authoritative_storage_disappears() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        let request = match manager.state() {
            KeyTeleportManagerState::ReceiveReady(state) => state,
            other => panic!("expected receive ready, got {other:?}"),
        };
        let sender = SenderSession::new(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let packet = KeyTeleportSenderPacket::new(response.packet);

        Keychain::global().delete_key_teleport_receive_session();
        let error =
            manager.handle_action(Action::Ingest(KeyTeleportInput::Sender(Arc::new(packet))));

        assert_eq!(error, Err(KeyTeleportAlert::NoActiveReceiveSession));
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn duplicate_receive_import_finishes_and_erases_the_receive_session() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();
        manager.clone().dispatch(Action::StartReceive);
        let request = match manager.state() {
            KeyTeleportManagerState::ReceiveReady(state) => state,
            other => panic!("expected receive ready, got {other:?}"),
        };
        let sender = SenderSession::new(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let password = response.password.clone();

        manager
            .handle_action(Action::Ingest(KeyTeleportInput::Sender(Arc::new(
                KeyTeleportSenderPacket::new(response.packet),
            ))))
            .unwrap();
        manager.enter_sender_password(&password.as_display_text()).unwrap();
        manager.import_received_wallet().unwrap();

        let KeyTeleportManagerState::ReceiveAlreadyImportedWallet(wallet) = manager.state() else {
            panic!("expected already-imported result")
        };
        assert_eq!(wallet.id, fixture.wallet.id);
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
    }

    #[test]
    fn receive_import_refuses_a_changed_network() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let config = &Database::global().global_config;
        let original_network = config.selected_network();
        let changed_network =
            if original_network == Network::Bitcoin { Network::Signet } else { Network::Bitcoin };
        let manager = RustKeyTeleportManager::new();
        manager.start_receive().unwrap();
        let (packet, password) = sender_transfer(&receive_request(&manager));
        manager.ingest(KeyTeleportInput::Sender(packet)).unwrap();
        manager.enter_sender_password(&password).unwrap();
        config.set_selected_network(changed_network).unwrap();

        let error = manager.import_received_wallet();

        config.set_selected_network(original_network).unwrap();
        assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
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
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(
                KeyTeleportError::UnsupportedMnemonicWordCount(15),
            ),
            KeyTeleportAlert::InvalidPayload,
        );
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(KeyTeleportError::NonMasterXprvPayload),
            KeyTeleportAlert::InvalidPayload,
        );
        assert_eq!(
            KeyTeleportAlert::from_receive_decode_error(KeyTeleportError::NonMainnetXprvPayload),
            KeyTeleportAlert::InvalidPayload,
        );
    }

    #[test]
    fn expired_receive_session_is_deleted_and_not_resumed() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let session = ReceiverSession::from_private_key_bytes([3; 32]).unwrap();
        let persisted = PersistedReceiveSession {
            session_id: Some(ReceiveSessionId::new().0),
            private_key_hex: hex::encode(session.private_key_bytes()),
            created_at_secs: now_secs() - RECEIVE_SESSION_TTL.as_secs() - 1,
            network: Database::global().global_config.selected_network(),
            wallet_mode: Database::global().global_config.wallet_mode(),
        };
        {
            let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
            persisted.save_unlocked().unwrap();
        }
        let manager = RustKeyTeleportManager::new();

        let expired = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
        manager.clone().dispatch(Action::StartReceive);
        let replacement = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();

        assert_ne!(replacement, expired);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
    }

    #[test]
    fn receive_resume_and_ingest_refuse_a_changed_network() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let config = &Database::global().global_config;
        let original_network = config.selected_network();
        let changed_network =
            if original_network == Network::Bitcoin { Network::Signet } else { Network::Bitcoin };
        let original_manager = RustKeyTeleportManager::new();
        original_manager.start_receive().unwrap();
        let packet = sender_packet(&receive_request(&original_manager));
        config.set_selected_network(changed_network).unwrap();
        let resumed_manager = RustKeyTeleportManager::new();

        let resume_error = resumed_manager.start_receive();
        let ingest_error = original_manager.ingest(KeyTeleportInput::Sender(packet));

        config.set_selected_network(original_network).unwrap();
        assert_eq!(resume_error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
        assert_eq!(ingest_error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
    }

    #[test]
    fn receive_ingest_refuses_a_changed_wallet_mode() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let config = &Database::global().global_config;
        let original_mode = config.wallet_mode();
        let manager = RustKeyTeleportManager::new();
        manager.start_receive().unwrap();
        let packet = sender_packet(&receive_request(&manager));
        match original_mode {
            WalletMode::Main => config.set_decoy_mode().unwrap(),
            WalletMode::Decoy => config.set_main_mode().unwrap(),
        }

        let error = manager.ingest(KeyTeleportInput::Sender(packet));

        match original_mode {
            WalletMode::Main => config.set_main_mode().unwrap(),
            WalletMode::Decoy => config.set_decoy_mode().unwrap(),
        }
        assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
    }

    #[test]
    fn ingest_rejects_packets_that_conflict_with_the_active_direction() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let receive_manager = RustKeyTeleportManager::new();
        receive_manager.start_receive().unwrap();
        let other_receiver = ReceiverSession::new().request().unwrap();

        let receive_error = receive_manager.ingest(KeyTeleportInput::Receiver(Arc::new(
            KeyTeleportReceiverPacket::new(other_receiver.packet),
        )));

        let fixture = SendWalletFixture::new();
        let send_manager = RustKeyTeleportManager::new();
        send_manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        let receiver = ReceiverSession::new();
        let request = receiver.request().unwrap();
        let sender = SenderSession::new(&request.packet, &request.numeric_code).unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let send_error = send_manager.ingest(KeyTeleportInput::Sender(Arc::new(
            KeyTeleportSenderPacket::new(response.packet),
        )));

        assert_eq!(receive_error, Err(KeyTeleportAlert::ConflictingTransferDirection));
        assert_eq!(send_error, Err(KeyTeleportAlert::ConflictingTransferDirection));
    }

    #[test]
    fn entry_actions_reject_the_opposite_active_direction() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let receive_manager = RustKeyTeleportManager::new();
        receive_manager.start_receive().unwrap();
        let wallet_id = WalletMetadata::preview_new().id;

        let send_error = receive_manager.start_send_from_wallet(wallet_id);

        assert_eq!(send_error, Err(KeyTeleportAlert::ConflictingTransferDirection));
        assert!(matches!(receive_manager.state(), KeyTeleportManagerState::ReceiveReady(_)));

        receive_manager.end_receive().unwrap();
        let fixture = SendWalletFixture::new();
        let send_manager = RustKeyTeleportManager::new();
        send_manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();

        let receive_error = send_manager.start_receive();
        let restart_error = send_manager.restart_receive();

        assert_eq!(receive_error, Err(KeyTeleportAlert::ConflictingTransferDirection));
        assert_eq!(restart_error, Err(KeyTeleportAlert::ConflictingTransferDirection));
        assert!(matches!(send_manager.state(), KeyTeleportManagerState::SendAwaitReceiver));
    }

    #[test]
    fn sender_packet_without_active_receive_session_returns_clear_alert() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([4; 32]).unwrap();
        let request = receiver.request().unwrap();
        let sender = SenderSession::new(&request.packet, &request.numeric_code).unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let packet = KeyTeleportSenderPacket::new(response.packet);

        let error =
            manager.handle_action(Action::Ingest(KeyTeleportInput::Sender(Arc::new(packet))));

        assert_eq!(error, Err(KeyTeleportAlert::NoActiveReceiveSession));
    }

    #[test]
    fn sender_packet_resumes_persisted_receive_session_before_route_appears() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let original_manager = RustKeyTeleportManager::new();
        original_manager.clone().dispatch(Action::StartReceive);
        let request = match original_manager.state() {
            KeyTeleportManagerState::ReceiveReady(state) => state,
            other => panic!("expected receive ready, got {other:?}"),
        };
        let sender = SenderSession::new(
            request.packet.inner(),
            &NumericCode::from_str(&request.numeric_code).unwrap(),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let manager = RustKeyTeleportManager::new();

        manager
            .handle_action(Action::Ingest(KeyTeleportInput::Sender(Arc::new(
                KeyTeleportSenderPacket::new(response.packet),
            ))))
            .unwrap();
        manager.handle_action(Action::StartReceive).unwrap();

        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveEnterPassword));
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

        let model = manager.model.lock();
        let Phase::SendAwaitReceiver { wallet } = &model.phase else {
            panic!("expected wallet-fixed private phase")
        };
        assert_eq!(wallet, &fixture.wallet);
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
    fn receiver_code_reaches_send_ready_for_mnemonic() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([11; 32]).unwrap();
        let request = receiver.request().unwrap();

        manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();
        manager.enter_receiver_code(request.numeric_code.as_str()).unwrap();

        let model = manager.model.lock();
        let Phase::SendReady(ready) = &model.phase else { panic!("expected send ready") };
        assert_eq!(ready.selected_wallet, fixture.wallet);
        assert!(matches!(
            receiver.decode(ready.packet.inner(), &ready.password.0).unwrap(),
            DecodedPayload::Mnemonic(_)
        ));
    }

    #[test]
    fn eligible_wallet_by_id_ignores_other_wallets_with_unreadable_secrets() {
        use cove_cspp::CsppStore as _;

        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let database = Database::global();
        let network = fixture.wallet.network;
        let mode = fixture.wallet.wallet_mode;

        let mut corrupt = WalletMetadata::preview_new();
        corrupt.network = network;
        corrupt.wallet_mode = mode;
        database
            .wallets
            .save_all_wallets(network, mode, vec![fixture.wallet.clone(), corrupt.clone()])
            .unwrap();
        Keychain::global()
            .save(format!("{}::wallet_mnemonic", corrupt.id), "garbage-secret".to_string())
            .unwrap();
        Keychain::global()
            .save(
                format!("{}::wallet_mnemonic_encryption_key_and_nonce", corrupt.id),
                "garbage-cryptor".to_string(),
            )
            .unwrap();

        assert_eq!(eligible_wallet_by_id(&fixture.wallet.id).unwrap(), fixture.wallet);
        assert_eq!(
            eligible_wallet_by_id(&WalletMetadata::preview_new().id),
            Err(KeyTeleportAlert::IneligibleWallet)
        );

        assert!(Keychain::global().delete_wallet_items(&corrupt.id));
    }

    #[test]
    fn end_receive_during_send_ready_preserves_receive_session_and_send_state() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let receive_manager = RustKeyTeleportManager::new();
        receive_manager.start_receive().unwrap();
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());

        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([14; 32]).unwrap();
        let request = receiver.request().unwrap();
        manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();
        manager.enter_receiver_code(request.numeric_code.as_str()).unwrap();
        assert!(matches!(manager.state(), KeyTeleportManagerState::SendReady(_)));

        assert_eq!(
            manager.handle_action(Action::EndReceive),
            Err(KeyTeleportAlert::ConflictingTransferDirection)
        );
        assert_eq!(
            manager.handle_action(Action::FinishReview),
            Err(KeyTeleportAlert::ConflictingTransferDirection)
        );
        assert!(matches!(manager.state(), KeyTeleportManagerState::SendReady(_)));
        assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_some());
    }

    #[test]
    fn receiver_code_reaches_send_ready_for_xprv_stash() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let xprv = bdk_wallet::bitcoin::bip32::Xpriv::new_master(
            bdk_wallet::bitcoin::Network::Bitcoin,
            &[13; 32],
        )
        .unwrap();
        let fixture = SendWalletFixture::with_secret(WalletSecret::try_from(xprv).unwrap());
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([12; 32]).unwrap();
        let request = receiver.request().unwrap();

        manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();
        manager.enter_receiver_code(request.numeric_code.as_str()).unwrap();

        let model = manager.model.lock();
        let Phase::SendReady(ready) = &model.phase else { panic!("expected send ready") };
        assert_eq!(ready.selected_wallet, fixture.wallet);
        let DecodedPayload::Xprv(decoded) =
            receiver.decode(ready.packet.inner(), &ready.password.0).unwrap()
        else {
            panic!("expected xprv")
        };
        assert_eq!(decoded.expose_string(), xprv.to_string());
    }

    #[test]
    fn receiver_code_revalidates_the_wallet_before_reading_its_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([22; 32]).unwrap();
        let request = receiver.request().unwrap();
        manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();
        Database::global()
            .wallets
            .save_all_wallets(fixture.wallet.network, fixture.wallet.wallet_mode, Vec::new())
            .unwrap();

        let error = manager.enter_receiver_code(request.numeric_code.as_str());

        assert_eq!(error, Err(KeyTeleportAlert::IneligibleWallet));
        assert!(matches!(manager.state(), KeyTeleportManagerState::SendEnterCode(_)));
    }

    #[test]
    fn receiver_code_rejects_a_replaced_wallet_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let fixture = SendWalletFixture::new();
        let manager = RustKeyTeleportManager::new();
        let receiver = ReceiverSession::from_private_key_bytes([23; 32]).unwrap();
        let request = receiver.request().unwrap();
        manager.start_send_from_wallet(fixture.wallet.id.clone()).unwrap();
        manager
            .start_send_with_receiver_packet(Arc::new(KeyTeleportReceiverPacket::new(
                request.packet,
            )))
            .unwrap();
        let before = manager.state();
        let replacement = Mnemonic::from_entropy(&[1_u8; 16]).unwrap();
        Keychain::global().delete_wallet_secret(&fixture.wallet.id);
        Keychain::global()
            .save_wallet_secret(&fixture.wallet.id, WalletSecret::Mnemonic(replacement))
            .unwrap();

        let error = manager.enter_receiver_code(request.numeric_code.as_str());

        assert_eq!(error, Err(KeyTeleportAlert::IneligibleWallet));
        assert_eq!(manager.state(), before);
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
        let packet = Arc::new(KeyTeleportReceiverPacket::new(request.packet));
        manager.set_phase(Phase::SendEnterCode { packet, wallet });

        let error = manager.enter_receiver_code(&wrong_code);

        assert_eq!(error, Err(KeyTeleportAlert::WrongReceiverCode));
        assert!(matches!(&manager.model.lock().phase, Phase::SendEnterCode { .. }));
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

        assert!(!is_send_eligible(&hot_wallet).unwrap());

        Keychain::global().save_wallet_key(&hot_wallet.id, mnemonic.clone()).unwrap();
        assert!(is_send_eligible(&hot_wallet).unwrap());

        let unsupported_mnemonic = Mnemonic::from_entropy(&[0_u8; 20]).unwrap();
        Keychain::global().delete_wallet_secret(&hot_wallet.id);
        Keychain::global().save_wallet_key(&hot_wallet.id, unsupported_mnemonic).unwrap();
        assert!(!is_send_eligible(&hot_wallet).unwrap());

        let xpriv = bdk_wallet::bitcoin::bip32::Xpriv::new_master(
            bdk_wallet::bitcoin::Network::Bitcoin,
            &[9; 32],
        )
        .unwrap();
        Keychain::global().delete_wallet_secret(&hot_wallet.id);
        Keychain::global()
            .save_wallet_secret(&hot_wallet.id, WalletSecret::try_from(xpriv).unwrap())
            .unwrap();
        assert!(is_send_eligible(&hot_wallet).unwrap());

        hot_wallet.wallet_type = WalletType::Cold;
        assert!(!is_send_eligible(&hot_wallet).unwrap());

        Keychain::global().delete_wallet_items(&hot_wallet.id);
    }
}
