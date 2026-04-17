use bitcoin::secp256k1::{Secp256k1, SecretKey};
use rand::RngExt as _;
use zeroize::Zeroizing;

use crate::crypto::{aes256ctr, pbkdf2_stretch, receiver_pubkey_key, session_key, verify_checksum};
use crate::error::Error;
use crate::packet::{ReceiverPacket, SenderPacket};
use crate::payload::Payload;

/// Maximum value for the 8-digit numeric code (inclusive).
const MAX_NUMERIC_CODE: u32 = 99_999_999;

/// A receiver session: holds the ephemeral EC private key bytes and numeric code.
/// The key bytes are zeroed on drop via `Zeroizing`.
#[derive(Debug)]
pub struct ReceiverSession {
    /// Raw 32-byte secret key — kept as bytes so we can zero them on drop.
    privkey_bytes: Zeroizing<[u8; 32]>,
    /// 8-digit code shown to the receiver, shared out-of-band with the sender.
    numeric_code: u32,
}

impl ReceiverSession {
    /// Generate a fresh receiver session (random keypair + random 8-digit code).
    pub fn generate() -> Self {
        let mut key_bytes = [0u8; 32];
        rand::rng().fill(&mut key_bytes);
        // Retry if we somehow hit an invalid scalar (astronomically unlikely)
        while SecretKey::from_slice(&key_bytes).is_err() {
            rand::rng().fill(&mut key_bytes);
        }

        let numeric_code = rand::random::<u32>() % (MAX_NUMERIC_CODE + 1);
        Self { privkey_bytes: Zeroizing::new(key_bytes), numeric_code }
    }

    fn privkey(&self) -> SecretKey {
        SecretKey::from_slice(&self.privkey_bytes[..]).expect("stored key is always valid")
    }

    /// The 8-digit numeric code (raw value).
    pub fn numeric_code(&self) -> u32 {
        self.numeric_code
    }

    /// The numeric code formatted as a zero-padded 8-digit string for display.
    pub fn numeric_code_display(&self) -> String {
        format!("{:08}", self.numeric_code)
    }

    /// Build the `R` packet to share with the sender (via QR / NFC / link).
    ///
    /// The receiver's compressed pubkey is AES-256-CTR encrypted using a key derived
    /// from the numeric code.
    pub fn to_packet(&self) -> ReceiverPacket {
        let secp = Secp256k1::new();
        let pubkey = self.privkey().public_key(&secp);
        let compressed = pubkey.serialize(); // 33 bytes

        let key = receiver_pubkey_key(self.numeric_code);
        let encrypted = aes256ctr(&key, &compressed);
        let arr: [u8; 33] = encrypted.try_into().expect("33 bytes in, 33 bytes out");
        ReceiverPacket::new(arr)
    }

    /// Decode an incoming sender packet using this session's private key and
    /// the teleport password supplied by the sender.
    ///
    /// Decryption flow (per spec):
    /// 1. ECDH(privkey, sender_pubkey) → session key
    /// 2. AES-CTR(session_key) decrypt → outer plaintext
    /// 3. Verify 2-byte checksum on outer plaintext
    /// 4. PBKDF2(session_key, teleport_pass) → inner key
    /// 5. AES-CTR(inner_key) decrypt → inner plaintext
    /// 6. Verify 2-byte checksum on inner plaintext
    /// 7. Parse payload type byte
    pub fn decode(
        &self,
        sender_pkt: &SenderPacket,
        teleport_password: &str,
    ) -> Result<Payload, Error> {
        let sk = session_key(&self.privkey(), sender_pkt.sender_pubkey());

        // Outer decryption + checksum
        let outer_plain = aes256ctr(&sk, sender_pkt.encrypted_body());
        let intermediate = verify_checksum(&outer_plain).ok_or(Error::ChecksumMismatch)?;

        // Inner decryption + checksum
        let inner_key = pbkdf2_stretch(&sk, teleport_password.as_bytes());
        let inner_plain = aes256ctr(&inner_key, intermediate);
        let payload_bytes = verify_checksum(&inner_plain).ok_or(Error::ChecksumMismatch)?;

        Payload::from_bytes(payload_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_valid_packet() {
        let session = ReceiverSession::generate();
        let pkt = session.to_packet();
        let bbqr = pkt.to_bbqr();
        assert!(bbqr.starts_with("B$2R"));
        let recovered = ReceiverPacket::from_bbqr(&bbqr).unwrap();
        assert_eq!(recovered, pkt);
    }

    #[test]
    fn numeric_code_is_in_range() {
        for _ in 0..20 {
            let s = ReceiverSession::generate();
            assert!(s.numeric_code() <= MAX_NUMERIC_CODE);
            assert_eq!(s.numeric_code_display().len(), 8);
        }
    }
}
