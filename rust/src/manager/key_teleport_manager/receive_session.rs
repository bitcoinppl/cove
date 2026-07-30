use std::{
    fmt,
    time::{Duration, UNIX_EPOCH},
};

use cove_device::keychain::Keychain;
use keyteleport::ReceiverSession;
use parking_lot::{Mutex, MutexGuard};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    database::Database, key_teleport::KeyTeleportReceiverPacket, network::Network,
    wallet::metadata::WalletMode,
};

use super::{KeyTeleportAlert, KeyTeleportReceiveState};

const RECEIVE_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
static RECEIVE_SESSION_STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
pub(crate) struct ReceiveSessionStore;

#[derive(Debug)]
pub(crate) enum LoadedReceiveSession {
    Active(ActiveReceiveSession),
    Expired(ReceiveSessionId),
}

#[derive(Debug)]
pub(crate) struct ActiveReceiveSession {
    pub(crate) id: ReceiveSessionId,
    receiver: ReceiverSession,
    pub(crate) created_at_secs: u64,
    pub(crate) network: Network,
    pub(crate) wallet_mode: WalletMode,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ReceiveSessionId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReceiveScope {
    pub(crate) network: Network,
    pub(crate) wallet_mode: WalletMode,
}

pub(crate) struct AuthoritativeReceiveSession {
    session_id: ReceiveSessionId,
    _storage_guard: MutexGuard<'static, ()>,
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

impl ReceiveSessionStore {
    pub(crate) fn create(&self) -> Result<ActiveReceiveSession, KeyTeleportAlert> {
        let session = ActiveReceiveSession::new();
        self.save(&session)?;

        Ok(session)
    }

    pub(crate) fn load(&self) -> Result<Option<LoadedReceiveSession>, KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
        let Some(persisted) = load_receive_session_unlocked()? else {
            return Ok(None);
        };

        let session_id = persisted.session_id()?;
        if persisted.is_expired() {
            return Ok(Some(LoadedReceiveSession::Expired(session_id)));
        }

        ActiveReceiveSession::restore(&persisted).map(LoadedReceiveSession::Active).map(Some)
    }

    pub(crate) fn save(&self, session: &ActiveReceiveSession) -> Result<(), KeyTeleportAlert> {
        let mut private_key = session.receiver.private_key_bytes();
        let persisted = PersistedReceiveSession {
            session_id: Some(session.id.0.clone()),
            private_key_hex: hex::encode(private_key),
            created_at_secs: session.created_at_secs,
            network: session.network,
            wallet_mode: session.wallet_mode,
        };

        private_key.zeroize();

        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        persisted.save_unlocked()
    }

    pub(crate) fn authoritative(
        &self,
        session: &ActiveReceiveSession,
    ) -> Result<AuthoritativeReceiveSession, KeyTeleportAlert> {
        let storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();
        ensure_authoritative_receive_session_unlocked(session)?;

        Ok(AuthoritativeReceiveSession {
            session_id: session.id.clone(),
            _storage_guard: storage_guard,
        })
    }

    pub(crate) fn delete(&self) -> Result<(), KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        delete_receive_session_unlocked()
    }

    pub(crate) fn delete_if_matches(
        &self,
        session_id: &ReceiveSessionId,
    ) -> Result<(), KeyTeleportAlert> {
        let _storage_guard = RECEIVE_SESSION_STORAGE_LOCK.lock();

        delete_receive_session_if_matches_unlocked(session_id)
    }
}

impl AuthoritativeReceiveSession {
    pub(crate) fn delete(&self) -> Result<(), KeyTeleportAlert> {
        delete_receive_session_if_matches_unlocked(&self.session_id)
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

    pub(crate) fn try_clone(&self) -> Result<Self, KeyTeleportAlert> {
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

    pub(crate) fn receiver_session(&self) -> &ReceiverSession {
        &self.receiver
    }

    pub(crate) fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.created_at_secs) >= RECEIVE_SESSION_TTL.as_secs()
    }

    pub(crate) fn ensure_current_scope(&self) -> Result<(), KeyTeleportAlert> {
        self.scope().ensure_current()
    }

    pub(crate) fn receive_state(&self) -> Result<KeyTeleportReceiveState, KeyTeleportAlert> {
        let request = self
            .receiver_session()
            .request()
            .map_err(|error| KeyTeleportAlert::Protocol(error.to_string()))?;

        Ok(KeyTeleportReceiveState {
            packet: std::sync::Arc::new(KeyTeleportReceiverPacket::from(request.packet)),
            numeric_code: request.numeric_code.as_str().to_string(),
            grouped_numeric_code: request.numeric_code.grouped(),
            created_at_secs: self.created_at_secs,
            network: self.network,
            wallet_mode: self.wallet_mode,
        })
    }

    pub(crate) fn into_id(self) -> ReceiveSessionId {
        self.id
    }

    fn scope(&self) -> ReceiveScope {
        ReceiveScope { network: self.network, wallet_mode: self.wallet_mode }
    }
}

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

impl ReceiveScope {
    pub(crate) fn current() -> Self {
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

fn now_secs() -> u64 {
    UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}
