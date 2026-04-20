use std::sync::Arc;

use cove_keyteleport::{Payload, ReceiverSession, SenderPacket};

#[derive(Debug, uniffi::Object)]
pub struct KeyTeleportReceiverSession(ReceiverSession);

#[uniffi::export]
impl KeyTeleportReceiverSession {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self(ReceiverSession::generate()))
    }

    pub fn numeric_code_display(&self) -> String {
        self.0.numeric_code_display()
    }

    pub fn receiver_packet_bbqr(&self) -> String {
        self.0.to_packet().to_bbqr()
    }

    pub fn decode(
        &self,
        sender_packet_bbqr: String,
        teleport_password: String,
    ) -> Result<KeyTeleportPayload, KeyTeleportError> {
        let pkt = SenderPacket::from_bbqr(&sender_packet_bbqr)
            .map_err(|e| KeyTeleportError::InvalidSenderPacket(e.to_string()))?;

        let payload = self
            .0
            .decode(&pkt, &teleport_password)
            .map_err(|e| KeyTeleportError::DecodeFailed(e.to_string()))?;

        Ok(payload.into())
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum KeyTeleportPayload {
    /// A BIP-39 mnemonic — the word list as a space-separated string.
    Mnemonic { words: String },
    /// A serialized XPRV (base58).
    Xprv { xprv: String },
}

impl From<Payload> for KeyTeleportPayload {
    fn from(payload: Payload) -> Self {
        match payload {
            Payload::Mnemonic(m) => Self::Mnemonic { words: m.to_string() },
            Payload::Xprv(xprv) => Self::Xprv { xprv },
        }
    }
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum KeyTeleportError {
    #[error("Invalid sender packet: {0}")]
    InvalidSenderPacket(String),

    #[error("Decryption failed — wrong password or code: {0}")]
    DecodeFailed(String),
}
