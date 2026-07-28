mod controller;
mod model;
mod receive;
mod receive_session;
mod send;

use std::{fmt, sync::Arc};

use cove_device::keychain::KeychainError;
use cove_keyteleport::{Error as KeyTeleportError, NotesPayload, NotesRecord, TeleportPassword};
use parking_lot::Mutex;
use tracing::trace;

use crate::{
    database,
    key_teleport::{KeyTeleportReceiverPacket, KeyTeleportSenderPacket},
    manager::{import_wallet_manager::ImportWalletError, reconcile_channel::ReconcileChannel},
    multi_format::StringOrData,
    network::Network,
    wallet::metadata::{WalletId, WalletMetadata, WalletMode},
    wallet_identity::PublicWalletIdentityError,
};

use super::deferred_sender::SingleOrMany;
use controller::ManagerController;
use model::ManagerModel;
use receive::ReceiveWorkflow;
use receive_session::ReceiveSessionStore;
pub(crate) use send::is_send_eligible_wallet_id;

type Message = KeyTeleportManagerReconcileMessage;
type Action = KeyTeleportManagerAction;
type Reconciler = dyn KeyTeleportManagerReconciler;

#[uniffi::export(callback_interface)]
pub trait KeyTeleportManagerReconciler: Send + Sync + fmt::Debug + 'static {
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

#[derive(Debug, uniffi::Object)]
pub struct RustKeyTeleportManager {
    action_lock: Mutex<()>,
    model: Arc<Mutex<ManagerModel>>,
    receive_sessions: ReceiveSessionStore,
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

#[uniffi::export]
impl RustKeyTeleportManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            action_lock: Mutex::new(()),
            model: Arc::new(Mutex::new(ManagerModel::default())),
            receive_sessions: ReceiveSessionStore,
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
        ReceiveWorkflow::new(self).reveal_mnemonic_words()
    }

    #[uniffi::method]
    pub fn reveal_xprv(&self) -> Option<String> {
        ReceiveWorkflow::new(self).reveal_xprv()
    }

    #[uniffi::method]
    pub fn is_send_eligible(&self, wallet_id: WalletId) -> bool {
        is_send_eligible_wallet_id(&wallet_id)
    }

    #[uniffi::method]
    pub fn dispatch(self: Arc<Self>, action: Action) {
        if let Err(alert) = ManagerController::new(&self).handle(action) {
            self.reconciler.send(Message::SetAlert(alert));
        }
    }
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
mod tests;
