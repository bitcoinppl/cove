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

    #[cfg(test)]
    fn with_private_key_and_password(
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

#[cfg(test)]
mod tests {
    use bip39::Mnemonic;

    use super::*;
    use crate::ReceiverSession;

    const RECEIVER_SECRET: [u8; 32] = [1; 32];
    const SENDER_SECRET: [u8; 32] = [3; 32];
    const PASSWORD_BYTES: [u8; 5] = [0x12, 0x34, 0x56, 0x78, 0x9a];
    const EXPECTED_SENDER_PACKET: &str =
        "02531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe3378159627a12c2";

    #[test]
    fn coldcard_sender_protocol_vector_matches() {
        let receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
        let request = receiver.request().unwrap();
        let sender = SenderSession::with_private_key_and_password(
            &request.packet,
            &request.numeric_code,
            SENDER_SECRET,
            TeleportPassword::from_bytes(PASSWORD_BYTES),
        )
        .unwrap();
        let mnemonic = Mnemonic::from_entropy(&[0_u8; 16]).unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();

        assert_eq!(response.password.as_display_text(), "CI2FM6E2");
        assert_eq!(hex_string(response.packet.as_bytes()), EXPECTED_SENDER_PACKET);
    }

    fn hex_string(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
