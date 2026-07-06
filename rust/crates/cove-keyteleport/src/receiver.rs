use bitcoin::secp256k1::PublicKey;
use zeroize::Zeroize as _;

use crate::{
    DecodedPayload, Error, NumericCode, ReceiverPacket, Result, SenderPacket, TeleportPassword,
    crypto::{self, EphemeralPrivateKey, SessionKey},
    payload::Payload,
};

#[derive(Debug)]
pub struct ReceiverSession {
    private_key: EphemeralPrivateKey,
}

impl ReceiverSession {
    pub fn new() -> Self {
        Self { private_key: EphemeralPrivateKey::generate() }
    }

    pub fn from_private_key_bytes(bytes: [u8; 32]) -> Result<Self> {
        Ok(Self { private_key: EphemeralPrivateKey::from_bytes(bytes)? })
    }

    pub fn private_key_bytes(&self) -> [u8; 32] {
        self.private_key.expose_bytes()
    }

    pub fn request(&self) -> Result<ReceiveRequest> {
        let (numeric_code, payload) = crypto::generate_receiver_packet(&self.private_key)?;

        Ok(ReceiveRequest { numeric_code, packet: ReceiverPacket::new(payload.to_vec())? })
    }

    pub fn decode_step1(&self, packet: &SenderPacket) -> Result<PendingPayload> {
        let sender_pubkey = PublicKey::from_slice(packet.sender_pubkey_bytes())?;
        let session_key = self.private_key.session_key(&sender_pubkey)?;
        let inner = session_key.decrypt_outer(packet.encrypted_body())?;

        Ok(PendingPayload { session_key, inner })
    }

    pub fn decode(
        &self,
        packet: &SenderPacket,
        password: &TeleportPassword,
    ) -> Result<DecodedPayload> {
        self.decode_step1(packet)?.complete(password)
    }
}

impl Default for ReceiverSession {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiveRequest {
    pub numeric_code: NumericCode,
    pub packet: ReceiverPacket,
}

#[derive(Debug)]
pub struct PendingPayload {
    session_key: SessionKey,
    inner: Vec<u8>,
}

impl PendingPayload {
    pub fn complete(mut self, password: &TeleportPassword) -> Result<DecodedPayload> {
        let noid_key = password.expose_bytes();
        let paranoid_key = self.session_key.paranoid_key(&noid_key);
        let plaintext = crypto::decrypt_inner(&paranoid_key, &self.inner)?;
        let decoded = DecodedPayload::decode(&plaintext);

        self.inner.zeroize();

        decoded
    }
}

impl Drop for PendingPayload {
    fn drop(&mut self) {
        self.inner.zeroize();
    }
}

pub(crate) fn encode_for_sender(
    sender_private_key: EphemeralPrivateKey,
    receiver_public_key: &PublicKey,
    password: &TeleportPassword,
    payload: Payload,
) -> Result<SenderPacket> {
    let sender_public_key = sender_private_key.public_key()?;
    let session_key = sender_private_key.session_key(receiver_public_key)?;
    let noid_key = password.expose_bytes();
    let paranoid_key = session_key.paranoid_key(&noid_key);
    let plaintext = payload.encode()?;
    let inner = crypto::encrypt_inner(&paranoid_key, &plaintext);
    let outer = session_key.encrypt_outer(&inner);
    let mut packet = Vec::with_capacity(33 + outer.len());
    packet.extend_from_slice(&sender_public_key.serialize());
    packet.extend_from_slice(&outer);

    SenderPacket::new(packet).map_err(|_| Error::InvalidSenderPacket)
}
