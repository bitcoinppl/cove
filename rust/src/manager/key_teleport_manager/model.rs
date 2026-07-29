use std::sync::Arc;

use bip39::Mnemonic;
use keyteleport::XprvPayload;

use crate::{
    key_teleport::{KeyTeleportReceiverPacket, KeyTeleportSenderPacket},
    wallet::metadata::WalletMetadata,
};

use super::{
    KeyTeleportManagerState, KeyTeleportMessageReview, KeyTeleportMnemonicReview,
    KeyTeleportReceiveState, KeyTeleportSendChooseWallet, KeyTeleportSendEnterCode,
    KeyTeleportSendReady, KeyTeleportXprvReview, Message, RustKeyTeleportManager,
    receive_session::{ActiveReceiveSession, ReceiveSessionId},
};

#[derive(Debug, Default)]
pub(crate) struct ManagerModel {
    pub(crate) generation: PhaseGeneration,
    pub(crate) phase: Phase,
}

pub(crate) struct StateMachine<'a>(&'a RustKeyTeleportManager);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhaseGeneration(u64);

impl PhaseGeneration {
    pub(crate) fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("KeyTeleport phase generation exhausted"))
    }
}

impl StateMachine<'_> {
    pub(crate) fn new(manager: &RustKeyTeleportManager) -> StateMachine<'_> {
        StateMachine(manager)
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        let state = {
            let mut model = self.0.model.lock();
            model.generation = model.generation.next();
            model.phase = phase;
            model.phase.public_state()
        };

        self.0.reconciler.send(Message::UpdateState(state));
    }

    pub(crate) fn set_phase_if_current(&self, generation: PhaseGeneration, phase: Phase) -> bool {
        let state = {
            let mut model = self.0.model.lock();
            if model.generation != generation {
                return false;
            }

            model.generation = model.generation.next();
            model.phase = phase;
            model.phase.public_state()
        };

        self.0.reconciler.send(Message::UpdateState(state));

        true
    }

    pub(crate) fn invalidate_phase(&self) -> PhaseGeneration {
        let mut model = self.0.model.lock();
        model.generation = model.generation.next();

        model.generation
    }
}

#[derive(Debug, Default)]
pub(crate) enum Phase {
    #[default]
    Idle,
    Receive(ReceivePhase),
    Send(SendPhase),
}

#[derive(Debug)]
pub(crate) enum ReceivePhase {
    Error,
    Ready { session: ActiveReceiveSession, state: KeyTeleportReceiveState },
    EnterPassword { session: ActiveReceiveSession, packet: Arc<KeyTeleportSenderPacket> },
    MnemonicReview { session: ActiveReceiveSession, mnemonic: Mnemonic },
    XprvReview { session: ActiveReceiveSession, xprv: XprvPayload, revealed: bool },
    MessageReview { session: ActiveReceiveSession, review: KeyTeleportMessageReview },
    Imported { session_id: ReceiveSessionId, wallet: WalletMetadata },
    AlreadyImported { session_id: ReceiveSessionId, wallet: WalletMetadata },
}

#[derive(Debug)]
pub(crate) enum SendPhase {
    AwaitReceiver { wallet: WalletMetadata },
    ChooseWallet { packet: Arc<KeyTeleportReceiverPacket>, eligible_wallets: Vec<WalletMetadata> },
    EnterCode { packet: Arc<KeyTeleportReceiverPacket>, wallet: WalletMetadata },
    Ready(KeyTeleportSendReady),
}

impl Phase {
    pub(crate) fn is_receive_direction(&self) -> bool {
        matches!(self, Self::Receive(_))
    }

    pub(crate) fn is_send_direction(&self) -> bool {
        matches!(self, Self::Send(_))
    }

    pub(crate) fn can_restore_receive_session(&self) -> bool {
        matches!(self, Self::Idle | Self::Send(_))
    }

    pub(crate) fn public_state(&self) -> KeyTeleportManagerState {
        match self {
            Self::Idle => KeyTeleportManagerState::Idle,
            Self::Receive(phase) => phase.public_state(),
            Self::Send(phase) => phase.public_state(),
        }
    }
}

impl ReceivePhase {
    pub(crate) fn is_flow_active(&self) -> bool {
        !matches!(self, Self::Error)
    }

    fn public_state(&self) -> KeyTeleportManagerState {
        match self {
            Self::Error => KeyTeleportManagerState::ReceiveError,
            Self::Ready { state, .. } => KeyTeleportManagerState::ReceiveReady(state.clone()),
            Self::EnterPassword { .. } => KeyTeleportManagerState::ReceiveEnterPassword,
            Self::MnemonicReview { mnemonic, .. } => {
                KeyTeleportManagerState::ReceiveMnemonicReview(KeyTeleportMnemonicReview {
                    word_count: mnemonic.word_count() as u32,
                })
            }

            Self::XprvReview { revealed, .. } => {
                KeyTeleportManagerState::ReceiveXprvReview(KeyTeleportXprvReview {
                    revealed: *revealed,
                })
            }

            Self::MessageReview { review, .. } => {
                KeyTeleportManagerState::ReceiveMessageReview(review.clone())
            }

            Self::Imported { wallet, .. } => {
                KeyTeleportManagerState::ReceiveImportedWallet(wallet.clone())
            }

            Self::AlreadyImported { wallet, .. } => {
                KeyTeleportManagerState::ReceiveAlreadyImportedWallet(wallet.clone())
            }
        }
    }

    pub(crate) fn active_session(
        &self,
    ) -> Result<Option<ActiveReceiveSession>, super::KeyTeleportAlert> {
        let session = match self {
            Self::Ready { session, .. }
            | Self::EnterPassword { session, .. }
            | Self::MnemonicReview { session, .. }
            | Self::XprvReview { session, .. }
            | Self::MessageReview { session, .. } => session,
            Self::Error | Self::Imported { .. } | Self::AlreadyImported { .. } => return Ok(None),
        };

        session.try_clone().map(Some)
    }

    pub(crate) fn session_id(&self) -> Result<Option<ReceiveSessionId>, super::KeyTeleportAlert> {
        match self {
            Self::Imported { session_id, .. } | Self::AlreadyImported { session_id, .. } => {
                Ok(Some(session_id.clone()))
            }

            _ => self.active_session().map(|session| session.map(ActiveReceiveSession::into_id)),
        }
    }
}

impl SendPhase {
    fn public_state(&self) -> KeyTeleportManagerState {
        match self {
            Self::AwaitReceiver { .. } => KeyTeleportManagerState::SendAwaitReceiver,
            Self::ChooseWallet { eligible_wallets, .. } => {
                KeyTeleportManagerState::SendChooseWallet(KeyTeleportSendChooseWallet {
                    eligible_wallets: eligible_wallets.clone(),
                })
            }

            Self::EnterCode { wallet, .. } => {
                KeyTeleportManagerState::SendEnterCode(KeyTeleportSendEnterCode {
                    selected_wallet: wallet.clone(),
                })
            }

            Self::Ready(state) => KeyTeleportManagerState::SendReady(state.clone()),
        }
    }
}
