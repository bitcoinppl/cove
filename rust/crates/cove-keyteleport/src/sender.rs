use bitcoin::secp256k1::PublicKey;

use crate::{
    NumericCode, Payload, ReceiverPacket, Result, SenderPacket, TeleportPassword,
    crypto::{self, EphemeralPrivateKey},
    receiver,
};

#[derive(Debug)]
pub struct SenderSession {
    receiver_public_key: PublicKey,
    private_key: EphemeralPrivateKey,
    password: TeleportPassword,
}

impl SenderSession {
    pub fn new(receiver_packet: &ReceiverPacket, code: &NumericCode) -> Result<Self> {
        Ok(Self {
            receiver_public_key: crypto::decrypt_receiver_pubkey(code, receiver_packet.as_bytes())?,
            private_key: EphemeralPrivateKey::generate(),
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
            private_key: EphemeralPrivateKey::from_bytes(private_key)?,
            password,
        })
    }

    pub fn send(self, payload: Payload) -> Result<SendResponse> {
        let Self { receiver_public_key, private_key, password } = self;
        let packet =
            receiver::encode_for_sender(private_key, &receiver_public_key, &password, payload)?;

        Ok(SendResponse { packet, password })
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
