use std::{
    fmt,
    str::FromStr as _,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use bip39::Mnemonic;
use cove_device::keychain::{Keychain, KeychainError};
use cove_keyteleport::{
    DecodedPayload, NumericCode, Payload, ReceiveRequest, ReceiverSession, SenderSession,
    TeleportPassword, XprvPayload,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{error, trace};
use zeroize::Zeroize as _;

use crate::{
    database::{self, Database},
    key_teleport::{KeyTeleportReceiverPacket, KeyTeleportSenderPacket},
    manager::{
        import_wallet_manager::{ImportWalletError, import_mnemonic_with_target},
        reconcile_channel::ReconcileChannel,
    },
    mnemonic::Mnemonic as StoredMnemonic,
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
    ConfirmReplaceReceive,
    CancelReceive,
    Ingest(StringOrData),
    StartSendFromWallet(WalletId),
    SelectSendWallet(WalletId),
    EnterReceiverCode(String),
    ConfirmSendMnemonic,
    EnterSenderPassword(String),
    ImportReceivedMnemonic,
    RevealXprv,
    HideXprv,
    FinishReview,
    Clear,
}

#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyTeleportManagerState {
    Idle,
    ReceiveReplacementRequired(KeyTeleportReceiveState),
    ReceiveReady(KeyTeleportReceiveState),
    ReceiveEnterPassword,
    ReceiveMnemonicReview(KeyTeleportMnemonicReview),
    ReceiveXprvReview(KeyTeleportXprvReview),
    SendChooseWallet(KeyTeleportSendChooseWallet),
    SendEnterCode(KeyTeleportSendEnterCode),
    SendConfirm(KeyTeleportSendConfirm),
    SendReady(KeyTeleportSendReady),
}

impl fmt::Debug for KeyTeleportManagerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::ReceiveReplacementRequired(_) => f.write_str("ReceiveReplacementRequired(****)"),
            Self::ReceiveReady(_) => f.write_str("ReceiveReady(****)"),
            Self::ReceiveEnterPassword => f.write_str("ReceiveEnterPassword"),
            Self::ReceiveMnemonicReview(_) => f.write_str("ReceiveMnemonicReview(****)"),
            Self::ReceiveXprvReview(review) => f
                .debug_tuple("ReceiveXprvReview")
                .field(&format_args!("revealed={}", review.revealed))
                .finish(),
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
    pub imported_wallet: Option<WalletMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportXprvReview {
    pub revealed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyTeleportSendChooseWallet {
    pub eligible_wallets: Vec<WalletMetadata>,
    pub selected_wallet: Option<WalletId>,
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

    #[error("wrong receiver code")]
    WrongReceiverCode,

    #[error("wrong Teleport Password")]
    WrongTeleportPassword,

    #[error("no eligible hot wallets with saved secret words")]
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

#[derive(Default)]
struct PrivateState {
    pending_receiver_packet: Option<Arc<KeyTeleportReceiverPacket>>,
    selected_send_wallet: Option<WalletMetadata>,
    receiver_code: Option<NumericCode>,
    pending_sender_packet: Option<Arc<KeyTeleportSenderPacket>>,
    received_secret: Option<ReceivedSecret>,
}

impl fmt::Debug for PrivateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateState")
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

impl Drop for PrivateState {
    fn drop(&mut self) {
        self.received_secret = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedReceiveSession {
    private_key_hex: String,
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
            Action::StartReceive => self.start_receive(false),
            Action::ConfirmReplaceReceive => self.start_receive(true),
            Action::CancelReceive => {
                Keychain::global().delete_key_teleport_receive_session();
                self.clear_private();
                self.set_state(KeyTeleportManagerState::Idle);
                Ok(())
            }
            Action::Ingest(input) => self.ingest(input),
            Action::StartSendFromWallet(wallet_id) => self.start_send_from_wallet(wallet_id),
            Action::SelectSendWallet(wallet_id) => self.select_send_wallet(wallet_id),
            Action::EnterReceiverCode(code) => self.enter_receiver_code(&code),
            Action::ConfirmSendMnemonic => self.confirm_send_mnemonic(),
            Action::EnterSenderPassword(password) => self.enter_sender_password(&password),
            Action::ImportReceivedMnemonic => self.import_received_mnemonic(),
            Action::RevealXprv => {
                self.set_xprv_revealed(true);
                Ok(())
            }
            Action::HideXprv => {
                self.set_xprv_revealed(false);
                Ok(())
            }
            Action::FinishReview => {
                Keychain::global().delete_key_teleport_receive_session();
                self.clear_private();
                self.set_state(KeyTeleportManagerState::Idle);
                Ok(())
            }
            Action::Clear => {
                self.clear_private();
                self.set_state(KeyTeleportManagerState::Idle);
                Ok(())
            }
        }
    }

    fn start_receive(&self, replace: bool) -> Result<(), KeyTeleportAlert> {
        if let Some(existing) = self.load_receive_session()? {
            if !existing.is_expired() && !replace {
                let state = receive_state_from_session(&existing)?;
                self.set_state(KeyTeleportManagerState::ReceiveReplacementRequired(state));
                return Ok(());
            }

            Keychain::global().delete_key_teleport_receive_session();
        }

        let session = PersistedReceiveSession::new();
        session.save()?;
        let state = receive_state_from_session(&session)?;
        self.set_state(KeyTeleportManagerState::ReceiveReady(state));

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

        self.set_state(KeyTeleportManagerState::SendChooseWallet(KeyTeleportSendChooseWallet {
            eligible_wallets: eligible_wallets(),
            selected_wallet: Some(wallet.id),
        }));

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

        let selected_wallet =
            Database::global().global_config.selected_wallet().and_then(|selected_id| {
                eligible_wallets
                    .iter()
                    .find(|wallet| wallet.id == selected_id)
                    .map(|wallet| wallet.id.clone())
            });

        self.private.lock().pending_receiver_packet = Some(packet);
        if let Some(wallet) = self.private.lock().selected_send_wallet.clone() {
            self.set_state(KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
                selected_wallet: wallet,
            }));
            return Ok(());
        }

        self.set_state(KeyTeleportManagerState::SendChooseWallet(KeyTeleportSendChooseWallet {
            eligible_wallets,
            selected_wallet,
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
        let mut private = self.private.lock();
        let packet =
            private.pending_receiver_packet.clone().ok_or(KeyTeleportAlert::NoPendingSend)?;
        let wallet =
            private.selected_send_wallet.clone().ok_or(KeyTeleportAlert::IneligibleWallet)?;

        SenderSession::new(packet.inner(), &code)
            .map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        private.receiver_code = Some(code);

        self.set_state(KeyTeleportManagerState::SendConfirm(KeyTeleportSendConfirm {
            selected_wallet: wallet,
            warns_passphrase_not_included: true,
        }));

        Ok(())
    }

    fn confirm_send_mnemonic(&self) -> Result<(), KeyTeleportAlert> {
        let private = self.private.lock();
        let packet =
            private.pending_receiver_packet.clone().ok_or(KeyTeleportAlert::NoPendingSend)?;
        let wallet =
            private.selected_send_wallet.clone().ok_or(KeyTeleportAlert::IneligibleWallet)?;
        let code = private.receiver_code.clone().ok_or(KeyTeleportAlert::WrongReceiverCode)?;
        drop(private);

        let mnemonic: Mnemonic = StoredMnemonic::try_from_id(&wallet.id)
            .map_err(|_| KeyTeleportAlert::IneligibleWallet)?
            .into();
        let sender = SenderSession::new(packet.inner(), &code)
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;
        let response = sender
            .send(Payload::mnemonic(mnemonic))
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;
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
        let receiver = session.receiver_session()?;
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
        let receiver = session.receiver_session()?;
        let decoded = receiver
            .decode(packet.inner(), &password)
            .map_err(|_| KeyTeleportAlert::WrongTeleportPassword)?;

        match decoded {
            DecodedPayload::Mnemonic(mnemonic) => {
                let word_count = mnemonic.words().count() as u32;
                self.private.lock().received_secret = Some(ReceivedSecret::Mnemonic(mnemonic));
                self.set_state(KeyTeleportManagerState::ReceiveMnemonicReview(
                    KeyTeleportMnemonicReview { word_count, imported_wallet: None },
                ));
            }
            DecodedPayload::Xprv(xprv) => {
                self.private.lock().received_secret = Some(ReceivedSecret::Xprv(xprv));
                self.set_state(KeyTeleportManagerState::ReceiveXprvReview(KeyTeleportXprvReview {
                    revealed: false,
                }));
            }
        }

        Ok(())
    }

    fn import_received_mnemonic(&self) -> Result<(), KeyTeleportAlert> {
        let session = self.active_receive_session()?;
        let mnemonic = {
            let private = self.private.lock();
            let Some(ReceivedSecret::Mnemonic(mnemonic)) = private.received_secret.as_ref() else {
                return Err(KeyTeleportAlert::NoPendingReceiveSecret);
            };
            mnemonic.clone()
        };

        let metadata = import_mnemonic_with_target(mnemonic, session.network, session.wallet_mode)
            .map_err(|error| KeyTeleportAlert::ImportFailed(error.to_string()))?;

        Keychain::global().delete_key_teleport_receive_session();
        self.clear_private();
        self.set_state(KeyTeleportManagerState::ReceiveMnemonicReview(KeyTeleportMnemonicReview {
            word_count: 0,
            imported_wallet: Some(metadata),
        }));

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

        receive_state_from_session(&session).map(KeyTeleportManagerState::ReceiveReady).ok()
    }

    fn active_receive_session(&self) -> Result<PersistedReceiveSession, KeyTeleportAlert> {
        let Some(session) = self.load_receive_session()? else {
            return Err(KeyTeleportAlert::NoActiveReceiveSession);
        };

        if session.is_expired() {
            Keychain::global().delete_key_teleport_receive_session();
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

impl PersistedReceiveSession {
    fn new() -> Self {
        let database = Database::global();
        let session = ReceiverSession::new();
        let mut private_key = session.private_key_bytes();
        let private_key_hex = hex::encode(private_key);
        private_key.zeroize();

        Self {
            private_key_hex,
            created_at_secs: now_secs(),
            network: database.global_config.selected_network(),
            wallet_mode: database.global_config.wallet_mode(),
        }
    }

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

    fn receive_request(&self) -> Result<ReceiveRequest, KeyTeleportAlert> {
        self.receiver_session()?
            .request()
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))
    }

    fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }
}

fn receive_state_from_session(
    session: &PersistedReceiveSession,
) -> Result<KeyTeleportReceiveState, KeyTeleportAlert> {
    let request = session.receive_request()?;

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
    wallet.wallet_type == WalletType::Hot && StoredMnemonic::try_from_id(&wallet.id).is_ok()
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

    #[test]
    fn start_receive_requires_confirmation_before_replacing_unexpired_session() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        let manager = RustKeyTeleportManager::new();

        manager.clone().dispatch(Action::StartReceive);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));

        manager.clone().dispatch(Action::StartReceive);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReplacementRequired(_)));

        manager.clone().dispatch(Action::ConfirmReplaceReceive);
        assert!(matches!(manager.state(), KeyTeleportManagerState::ReceiveReady(_)));
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
    fn send_eligibility_requires_hot_wallet_with_keychain_mnemonic() {
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

        hot_wallet.wallet_type = WalletType::Cold;
        assert!(!is_send_eligible(&hot_wallet));

        Keychain::global().delete_wallet_items(&hot_wallet.id);
    }
}
