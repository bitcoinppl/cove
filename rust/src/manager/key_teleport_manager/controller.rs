use super::{
    Action, KeyTeleportAlert, KeyTeleportInput, RustKeyTeleportManager,
    model::{Phase, StateMachine},
    receive::ReceiveWorkflow,
    send::SendWorkflow,
};

pub(crate) struct ManagerController<'a>(&'a RustKeyTeleportManager);

#[derive(Clone, Copy)]
enum PacketDirection {
    Receiver,
    Sender,
}

impl ManagerController<'_> {
    pub(crate) fn new(manager: &RustKeyTeleportManager) -> ManagerController<'_> {
        ManagerController(manager)
    }

    pub(crate) fn handle(&self, action: Action) -> Result<(), KeyTeleportAlert> {
        let _action_guard = self.0.action_lock.lock();

        match action {
            Action::StartReceive => ReceiveWorkflow::new(self.0).start(),
            Action::RestartReceive => ReceiveWorkflow::new(self.0).restart(),
            Action::EndReceive | Action::FinishReview => ReceiveWorkflow::new(self.0).end(),
            Action::Ingest(input) => self.ingest(input),
            Action::StartSendFromWallet(wallet_id) => {
                SendWorkflow::new(self.0).start_from_wallet(wallet_id)
            }
            Action::SelectSendWallet(wallet_id) => {
                SendWorkflow::new(self.0).select_wallet(wallet_id)
            }
            Action::EnterReceiverCode(code) => SendWorkflow::new(self.0).enter_receiver_code(&code),
            Action::EnterSenderPassword(password) => {
                ReceiveWorkflow::new(self.0).enter_password(&password)
            }
            Action::ImportReceivedWallet => ReceiveWorkflow::new(self.0).import_wallet(),
            Action::RevealXprv => ReceiveWorkflow::new(self.0).set_xprv_revealed(true),
            Action::HideXprv => ReceiveWorkflow::new(self.0).set_xprv_revealed(false),
            Action::Clear => {
                StateMachine::new(self.0).set_phase(Phase::Idle);
                Ok(())
            }
        }
    }

    fn ingest(&self, input: KeyTeleportInput) -> Result<(), KeyTeleportAlert> {
        let parsed = match input {
            KeyTeleportInput::Receiver(packet) => {
                self.ensure_ingest_direction(PacketDirection::Receiver)?;
                return SendWorkflow::new(self.0).start_with_receiver_packet(packet);
            }

            KeyTeleportInput::Sender(packet) => {
                self.ensure_ingest_direction(PacketDirection::Sender)?;
                return ReceiveWorkflow::new(self.0).start_password_entry(packet);
            }

            KeyTeleportInput::MultiFormat(input) => {
                crate::key_teleport::parse_key_teleport_input(input)
            }
        };

        match parsed {
            Ok(crate::key_teleport::ParsedKeyTeleport::Receiver(packet)) => {
                self.ensure_ingest_direction(PacketDirection::Receiver)?;
                SendWorkflow::new(self.0).start_with_receiver_packet(packet)
            }

            Ok(crate::key_teleport::ParsedKeyTeleport::Sender(packet)) => {
                self.ensure_ingest_direction(PacketDirection::Sender)?;
                ReceiveWorkflow::new(self.0).start_password_entry(packet)
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
        let phase = &self.0.model.lock().phase;
        let conflicts = match packet_direction {
            PacketDirection::Receiver => phase.is_receive_direction(),
            PacketDirection::Sender => phase.is_send_direction(),
        };

        if conflicts {
            return Err(KeyTeleportAlert::ConflictingTransferDirection);
        }

        Ok(())
    }
}
