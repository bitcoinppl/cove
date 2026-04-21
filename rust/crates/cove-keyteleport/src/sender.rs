use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use data_encoding::BASE32_NOPAD;
use rand::RngExt as _;
use zeroize::Zeroizing;

use crate::crypto::{aes256ctr, checksum, pbkdf2_stretch, receiver_pubkey_key, session_key};
use crate::error::Error;
use crate::packet::{ReceiverPacket, SenderPacket};
use crate::payload::Payload;

/// A sender session: holds the ephemeral private key and derived session state.
/// Key bytes are zeroed on drop via `Zeroizing`.
#[derive(Debug)]
pub struct SenderSession {
    privkey_bytes: Zeroizing<[u8; 32]>,
    session_key: Zeroizing<[u8; 32]>,
    /// 8-character Base32 teleport password shown to the sender, shared out-of-band.
    teleport_password: Zeroizing<String>,
}

impl SenderSession {
    /// Create a sender session from a `ReceiverPacket` and the numeric code.
    ///
    /// Steps:
    /// 1. Decrypt receiver pubkey from R packet using SHA256(numeric_code)
    /// 2. Generate ephemeral sender keypair
    /// 3. ECDH(sender_privkey, receiver_pubkey) → session key
    /// 4. Generate random 5-byte teleport password → Base32 (8 chars)
    pub fn new(r_packet: &ReceiverPacket, numeric_code: u32) -> Result<Self, Error> {
        // Decrypt the receiver's pubkey
        let key = receiver_pubkey_key(numeric_code);
        let pubkey_bytes = aes256ctr(&key, r_packet.encrypted_pubkey());
        let receiver_pubkey = PublicKey::from_slice(&pubkey_bytes).map_err(|_| {
            Error::InvalidReceiverPacket(
                "decrypted bytes are not a valid pubkey — wrong numeric code?".into(),
            )
        })?;

        // Generate ephemeral keypair from random bytes
        let mut key_bytes = [0u8; 32];
        rand::rng().fill(&mut key_bytes);
        while SecretKey::from_slice(&key_bytes).is_err() {
            rand::rng().fill(&mut key_bytes);
        }
        let privkey = SecretKey::from_slice(&key_bytes).expect("validated above");

        // Derive session key
        let sk = session_key(&privkey, &receiver_pubkey);

        // Generate teleport password: 5 random bytes → 8 Base32 chars
        let mut raw = [0u8; 5];
        rand::rng().fill(&mut raw[..]);
        let teleport_password = Zeroizing::new(BASE32_NOPAD.encode(&raw));

        Ok(Self {
            privkey_bytes: Zeroizing::new(key_bytes),
            session_key: Zeroizing::new(sk),
            teleport_password,
        })
    }

    fn privkey(&self) -> SecretKey {
        SecretKey::from_slice(&self.privkey_bytes[..]).expect("stored key is always valid")
    }

    /// The 8-character Base32 teleport password to share with the receiver out-of-band.
    pub fn teleport_password(&self) -> &str {
        self.teleport_password.as_str()
    }

    /// Encrypt the payload and produce a `SenderPacket`.
    ///
    /// Encryption flow (per spec):
    /// 1. Serialize payload → inner_plain
    /// 2. Append 2-byte checksum → inner_with_cs
    /// 3. inner_key = PBKDF2(session_key, teleport_pass)
    /// 4. layer2 = AES-CTR(inner_key, inner_with_cs)
    /// 5. Append 2-byte checksum to layer2 → outer_with_cs
    /// 6. body = AES-CTR(session_key, outer_with_cs)
    /// 7. S packet = sender_pubkey (33 bytes) || body
    pub fn encrypt(&self, payload: &Payload) -> SenderPacket {
        let secp = Secp256k1::new();
        let sender_pubkey = self.privkey().public_key(&secp);

        let inner_plain = payload.to_bytes();
        let inner_cs = checksum(&inner_plain);
        let mut inner_with_cs = inner_plain;
        inner_with_cs.extend_from_slice(&inner_cs);

        let inner_key = pbkdf2_stretch(&self.session_key, self.teleport_password.as_bytes());
        let layer2 = aes256ctr(&inner_key, &inner_with_cs);

        let outer_cs = checksum(&layer2);
        let mut outer_with_cs = layer2;
        outer_with_cs.extend_from_slice(&outer_cs);

        let body = aes256ctr(&self.session_key, &outer_with_cs);

        SenderPacket::new(sender_pubkey, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::ReceiverSession;

    #[test]
    fn teleport_password_is_8_base32_chars() {
        let receiver = ReceiverSession::generate();
        let r_pkt = receiver.to_packet();
        let sender = SenderSession::new(&r_pkt, receiver.numeric_code()).unwrap();
        let pw = sender.teleport_password();
        assert_eq!(pw.len(), 8);
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn wrong_numeric_code_gives_error() {
        let receiver = ReceiverSession::generate();
        let r_pkt = receiver.to_packet();
        let wrong_code = (receiver.numeric_code() + 1) % 100_000_000;
        // With overwhelming probability, wrong key → invalid pubkey bytes → error
        let result = SenderSession::new(&r_pkt, wrong_code);
        assert!(result.is_err());
    }
}
