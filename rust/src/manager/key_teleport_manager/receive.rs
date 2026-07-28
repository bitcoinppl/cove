use std::{str::FromStr as _, sync::Arc};

use bip39::Mnemonic;
use cove_device::keychain::{WalletSecret, WalletXprv};
use cove_keyteleport::{DecodedPayload, TeleportPassword, XprvPayload};
use tracing::error;

use crate::{
    database::Database,
    key_teleport::KeyTeleportSenderPacket,
    manager::import_wallet_manager::{
        ImportWalletError, import_key_teleport_wallet_secret_with_target,
    },
};

use super::{
    KeyTeleportAlert, Message, RustKeyTeleportManager,
    model::{Phase, PhaseGeneration, ReceivePhase, StateMachine},
    receive_session::{ActiveReceiveSession, LoadedReceiveSession, ReceiveSessionId},
};

pub(crate) struct ReceiveWorkflow<'a>(&'a RustKeyTeleportManager);

enum ReceivedSecret {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

impl ReceiveWorkflow<'_> {
    pub(crate) fn new(manager: &RustKeyTeleportManager) -> ReceiveWorkflow<'_> {
        ReceiveWorkflow(manager)
    }

    fn state_machine(&self) -> StateMachine<'_> {
        StateMachine::new(self.0)
    }

    pub(crate) fn start(&self) -> Result<(), KeyTeleportAlert> {
        let (receive_flow_active, active_session) = {
            let model = self.0.model.lock();
            match &model.phase {
                Phase::Send(_) => return Err(KeyTeleportAlert::ConflictingTransferDirection),
                Phase::Receive(phase) => (phase.is_flow_active(), phase.active_session()?),
                Phase::Idle => (false, None),
            }
        };
        if receive_flow_active {
            let Some(session) = active_session else {
                return Ok(());
            };
            if let Err(error) = self.0.receive_sessions.authoritative(&session) {
                self.state_machine().set_phase(Phase::Receive(ReceivePhase::Error));
                return Err(error);
            }

            return Ok(());
        }

        match self.0.receive_sessions.load() {
            Ok(Some(LoadedReceiveSession::Active(session))) => {
                if let Err(error) = session.ensure_current_scope() {
                    self.state_machine().set_phase(Phase::Receive(ReceivePhase::Error));
                    return Err(error);
                }

                return self.activate_session(session);
            }
            Ok(Some(LoadedReceiveSession::Expired(session_id))) => {
                self.replace_session_if_matches(
                    &session_id,
                    KeyTeleportAlert::ReceiveSessionExpired,
                )?;
                return Ok(());
            }
            Ok(None) => {}
            Err(error) => {
                error!("unable to load KeyTeleport receive session: {error}");
                self.replace_session(KeyTeleportAlert::ReceiveSessionReset)?;
                return Ok(());
            }
        }

        let generation = self.state_machine().invalidate_phase();
        self.create_session(generation).inspect_err(|_| {
            self.state_machine()
                .set_phase_if_current(generation, Phase::Receive(ReceivePhase::Error));
        })
    }

    pub(crate) fn restart(&self) -> Result<(), KeyTeleportAlert> {
        if self.0.model.lock().phase.is_send_direction() {
            return Err(KeyTeleportAlert::ConflictingTransferDirection);
        }

        let generation = self.state_machine().invalidate_phase();
        self.0.receive_sessions.delete()?;

        self.create_session(generation).inspect_err(|_| {
            self.state_machine()
                .set_phase_if_current(generation, Phase::Receive(ReceivePhase::Error));
        })
    }

    pub(crate) fn end(&self) -> Result<(), KeyTeleportAlert> {
        let session_id = {
            let model = self.0.model.lock();
            match &model.phase {
                Phase::Send(_) => return Err(KeyTeleportAlert::ConflictingTransferDirection),
                Phase::Receive(phase) => phase.session_id(),
                Phase::Idle => Ok(None),
            }
        }?;
        let generation = self.state_machine().invalidate_phase();

        match session_id {
            Some(session_id) => {
                if let Err(error) = self.0.receive_sessions.delete_if_matches(&session_id)
                    && error != KeyTeleportAlert::NoActiveReceiveSession
                {
                    return Err(error);
                }
            }
            None => self.0.receive_sessions.delete()?,
        }
        self.state_machine().set_phase_if_current(generation, Phase::Idle);

        Ok(())
    }

    pub(crate) fn reveal_mnemonic_words(&self) -> Vec<String> {
        let (session, mnemonic) = {
            let model = self.0.model.lock();
            let Phase::Receive(ReceivePhase::MnemonicReview { session, mnemonic }) = &model.phase
            else {
                return Vec::new();
            };

            let Ok(session) = session.try_clone() else {
                return Vec::new();
            };

            (session, mnemonic.clone())
        };
        if self.0.receive_sessions.authoritative(&session).is_err() {
            return Vec::new();
        }

        mnemonic.words().map(ToString::to_string).collect()
    }

    pub(crate) fn reveal_xprv(&self) -> Option<String> {
        self.set_xprv_revealed(true).ok()?;

        let model = self.0.model.lock();
        let Phase::Receive(ReceivePhase::XprvReview { xprv, .. }) = &model.phase else {
            return None;
        };

        Some(xprv.expose_string().to_string())
    }

    pub(crate) fn start_password_entry(
        &self,
        packet: Arc<KeyTeleportSenderPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let (generation, session) = self.take_ready_session()?;
        let _authoritative = self.0.receive_sessions.authoritative(&session)?;
        session
            .receiver_session()
            .decode_step1(packet.inner())
            .map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;

        let phase = Phase::Receive(ReceivePhase::EnterPassword { session, packet });
        if !self.state_machine().set_phase_if_current(generation, phase) {
            return Err(KeyTeleportAlert::NoActiveReceiveSession);
        }

        Ok(())
    }

    pub(crate) fn enter_password(&self, password: &str) -> Result<(), KeyTeleportAlert> {
        let (generation, session, packet) = self.take_password_phase()?;
        let password = TeleportPassword::from_str(password)
            .map_err(|_| KeyTeleportAlert::WrongTeleportPassword)?;
        let _authoritative = self.0.receive_sessions.authoritative(&session)?;
        let decoded = session
            .receiver_session()
            .decode(packet.inner(), &password)
            .map_err(KeyTeleportAlert::from_receive_decode_error)?;

        let phase = match decoded {
            DecodedPayload::Mnemonic(mnemonic) => {
                ReceivePhase::MnemonicReview { session, mnemonic }
            }
            DecodedPayload::Xprv(xprv) => {
                ReceivePhase::XprvReview { session, xprv, revealed: false }
            }
            DecodedPayload::Notes(notes) => {
                ReceivePhase::MessageReview { session, review: notes.into() }
            }
        };
        if !self.state_machine().set_phase_if_current(generation, Phase::Receive(phase)) {
            return Err(KeyTeleportAlert::NoPendingReceiveSecret);
        }

        Ok(())
    }

    pub(crate) fn import_wallet(&self) -> Result<(), KeyTeleportAlert> {
        let (generation, session, secret) = {
            let model = self.0.model.lock();
            match &model.phase {
                Phase::Receive(ReceivePhase::MnemonicReview { session, mnemonic }) => (
                    model.generation,
                    session.try_clone()?,
                    ReceivedSecret::Mnemonic(mnemonic.clone()).to_wallet_secret()?,
                ),
                Phase::Receive(ReceivePhase::XprvReview { session, xprv, .. }) => (
                    model.generation,
                    session.try_clone()?,
                    ReceivedSecret::Xprv(xprv.clone()).to_wallet_secret()?,
                ),
                _ => return Err(KeyTeleportAlert::NoPendingReceiveSecret),
            }
        };
        let authoritative = self.0.receive_sessions.authoritative(&session)?;
        let result = import_key_teleport_wallet_secret_with_target(
            secret,
            session.network,
            session.wallet_mode,
        );

        let phase = match result {
            Ok(metadata) => {
                authoritative.delete()?;
                ReceivePhase::Imported { session_id: session.id, wallet: metadata }
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
                authoritative.delete()?;
                ReceivePhase::AlreadyImported { session_id: session.id, wallet: metadata }
            }
            Err(error) => return Err(KeyTeleportAlert::ImportFailed(error.to_string())),
        };
        if !self.state_machine().set_phase_if_current(generation, Phase::Receive(phase)) {
            return Err(KeyTeleportAlert::NoPendingReceiveSecret);
        }

        Ok(())
    }

    pub(crate) fn set_xprv_revealed(&self, revealed: bool) -> Result<(), KeyTeleportAlert> {
        let session = {
            let model = self.0.model.lock();
            let Phase::Receive(ReceivePhase::XprvReview { session, .. }) = &model.phase else {
                return Ok(());
            };

            session.try_clone()?
        };
        self.0.receive_sessions.authoritative(&session)?;

        let state = {
            let mut model = self.0.model.lock();
            let Phase::Receive(ReceivePhase::XprvReview { revealed: current, .. }) =
                &mut model.phase
            else {
                return Ok(());
            };
            *current = revealed;
            model.phase.public_state()
        };
        self.0.reconciler.send(Message::UpdateState(state));

        Ok(())
    }

    fn replace_session(&self, alert: KeyTeleportAlert) -> Result<(), KeyTeleportAlert> {
        let generation = self.state_machine().invalidate_phase();
        self.0.receive_sessions.delete()?;
        self.create_session(generation).inspect_err(|_| {
            self.state_machine()
                .set_phase_if_current(generation, Phase::Receive(ReceivePhase::Error));
        })?;
        self.0.reconciler.send(Message::SetAlert(alert));

        Ok(())
    }

    fn replace_session_if_matches(
        &self,
        session_id: &ReceiveSessionId,
        alert: KeyTeleportAlert,
    ) -> Result<(), KeyTeleportAlert> {
        let generation = self.state_machine().invalidate_phase();
        self.0.receive_sessions.delete_if_matches(session_id)?;
        self.create_session(generation).inspect_err(|_| {
            self.state_machine()
                .set_phase_if_current(generation, Phase::Receive(ReceivePhase::Error));
        })?;
        self.0.reconciler.send(Message::SetAlert(alert));

        Ok(())
    }

    fn create_session(&self, generation: PhaseGeneration) -> Result<(), KeyTeleportAlert> {
        let session = self.0.receive_sessions.create()?;
        let session_id = session.id.clone();

        if self.activate_session_if_current(generation, session)? {
            return Ok(());
        }

        if let Err(error) = self.0.receive_sessions.delete_if_matches(&session_id)
            && error != KeyTeleportAlert::NoActiveReceiveSession
        {
            return Err(error);
        }

        Ok(())
    }

    fn activate_session(&self, session: ActiveReceiveSession) -> Result<(), KeyTeleportAlert> {
        let state = session.receive_state()?;
        self.state_machine().set_phase(Phase::Receive(ReceivePhase::Ready { session, state }));

        Ok(())
    }

    fn activate_session_if_current(
        &self,
        generation: PhaseGeneration,
        session: ActiveReceiveSession,
    ) -> Result<bool, KeyTeleportAlert> {
        let state = session.receive_state()?;
        let phase = Phase::Receive(ReceivePhase::Ready { session, state });

        Ok(self.state_machine().set_phase_if_current(generation, phase))
    }

    fn take_ready_session(
        &self,
    ) -> Result<(PhaseGeneration, ActiveReceiveSession), KeyTeleportAlert> {
        let (generation, active_session) = {
            let model = self.0.model.lock();
            let active_session = match &model.phase {
                Phase::Receive(ReceivePhase::Ready { session, .. }) => Some(session.try_clone()?),
                phase if phase.can_restore_receive_session() => None,
                _ => return Err(KeyTeleportAlert::NoActiveReceiveSession),
            };

            (model.generation, active_session)
        };
        let session = match active_session {
            Some(session) => session,
            None => match self.0.receive_sessions.load()? {
                Some(LoadedReceiveSession::Active(session)) => session,
                Some(LoadedReceiveSession::Expired(session_id)) => {
                    self.replace_session_if_matches(
                        &session_id,
                        KeyTeleportAlert::ReceiveSessionExpired,
                    )?;
                    return Err(KeyTeleportAlert::ReceiveSessionExpired);
                }
                None => return Err(KeyTeleportAlert::NoActiveReceiveSession),
            },
        };

        self.ensure_session_fresh(session).map(|session| (generation, session))
    }

    fn take_password_phase(
        &self,
    ) -> Result<
        (PhaseGeneration, ActiveReceiveSession, Arc<KeyTeleportSenderPacket>),
        KeyTeleportAlert,
    > {
        let (generation, session, packet) = {
            let model = self.0.model.lock();
            let Phase::Receive(ReceivePhase::EnterPassword { session, packet }) = &model.phase
            else {
                return Err(KeyTeleportAlert::NoPendingReceiveSecret);
            };

            (model.generation, session.try_clone()?, packet.clone())
        };

        self.ensure_session_fresh(session).map(|session| (generation, session, packet))
    }

    fn ensure_session_fresh(
        &self,
        session: ActiveReceiveSession,
    ) -> Result<ActiveReceiveSession, KeyTeleportAlert> {
        session.ensure_current_scope()?;
        self.0.receive_sessions.authoritative(&session)?;
        if session.is_expired() {
            self.replace_session_if_matches(&session.id, KeyTeleportAlert::ReceiveSessionExpired)?;
            return Err(KeyTeleportAlert::ReceiveSessionExpired);
        }

        Ok(session)
    }
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
