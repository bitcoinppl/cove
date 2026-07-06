use bitcoin::secp256k1::PublicKey;

use crate::{
    NumericCode, Payload, ReceiverPacket, Result, SenderPacket, TeleportPassword,
    crypto::{self, EphemeralPrivateKey},
    receiver,
};

#[derive(Debug)]
pub struct SenderSession {
    receiver_public_key: PublicKey,
    private_key: Option<EphemeralPrivateKey>,
    password: TeleportPassword,
}

impl SenderSession {
    pub fn new(receiver_packet: &ReceiverPacket, code: &NumericCode) -> Result<Self> {
        Ok(Self {
            receiver_public_key: crypto::decrypt_receiver_pubkey(code, receiver_packet.as_bytes())?,
            private_key: Some(EphemeralPrivateKey::generate()),
            password: TeleportPassword::generate(),
        })
    }

    pub fn with_private_key_and_password(
        receiver_packet: &ReceiverPacket,
        code: &NumericCode,
        private_key: [u8; 32],
        password: TeleportPassword,
    ) -> Result<Self> {
        Ok(Self {
            receiver_public_key: crypto::decrypt_receiver_pubkey(code, receiver_packet.as_bytes())?,
            private_key: Some(EphemeralPrivateKey::from_bytes(private_key)?),
            password,
        })
    }

    pub fn send(mut self, payload: Payload) -> Result<SendResponse> {
        let private_key = self
            .private_key
            .take()
            .expect("sender private key is present until send consumes the session");
        let packet = receiver::encode_for_sender(
            private_key,
            &self.receiver_public_key,
            &self.password,
            payload,
        )?;

        Ok(SendResponse { packet, password: self.password })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendRequest {
    pub receiver_packet: ReceiverPacket,
    pub numeric_code: NumericCode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendResponse {
    pub packet: SenderPacket,
    pub password: TeleportPassword,
}
