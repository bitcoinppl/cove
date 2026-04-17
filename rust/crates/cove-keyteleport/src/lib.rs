//! `cove-keyteleport` — Rust implementation of the Key Teleport cryptographic protocol.
//!
//! Implements the non-multisig (mnemonic / xprv) participant flow:
//! - Receiver generates an R packet + numeric code and can decode incoming S packets.
//! - Sender parses an R packet + numeric code, picks a teleport password, and encrypts
//!   a payload into an S packet.
//!
//! No UniFFI surface, no UI, no persistence — pure protocol primitives.
//!
//! # Example
//! ```rust
//! use cove_keyteleport::{ReceiverSession, SenderSession, Payload};
//! use bip39::Mnemonic;
//!
//! // Receiver side
//! let receiver = ReceiverSession::generate();
//! let r_packet = receiver.to_packet();
//! let code = receiver.numeric_code();
//!
//! // --- out-of-band: share r_packet.to_bbqr() and code ---
//!
//! // Sender side
//! let entropy = [0xABu8; 32]; // 32 bytes of entropy → 24-word mnemonic
//! let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();
//! let payload = Payload::Mnemonic(mnemonic);
//! let sender = SenderSession::new(&r_packet, code).unwrap();
//! let s_packet = sender.encrypt(&payload);
//! let teleport_pass = sender.teleport_password().to_string();
//!
//! // --- out-of-band: share s_packet.to_bbqr() and teleport_pass ---
//!
//! // Receiver decodes
//! let decoded = receiver.decode(&s_packet, &teleport_pass).unwrap();
//! ```

mod bbqr;
mod crypto;
mod error;
mod packet;
mod payload;
mod receiver;
mod sender;

pub use error::Error;
pub use packet::{ReceiverPacket, SenderPacket};
pub use payload::Payload;
pub use receiver::ReceiverSession;
pub use sender::SenderSession;

#[cfg(test)]
mod tests {
    use bip39::Mnemonic;

    use super::*;

    fn random_mnemonic(words: usize) -> Mnemonic {
        use rand::RngExt as _;
        let entropy_len = match words {
            12 => 16,
            18 => 24,
            24 => 32,
            _ => panic!("unsupported word count"),
        };
        let mut entropy = vec![0u8; entropy_len];
        rand::rng().fill(entropy.as_mut_slice());
        Mnemonic::from_entropy(&entropy).unwrap()
    }

    fn roundtrip(payload: Payload) -> Payload {
        let receiver = ReceiverSession::generate();
        let r_pkt = receiver.to_packet();
        let code = receiver.numeric_code();

        let sender = SenderSession::new(&r_pkt, code).unwrap();
        let s_pkt = sender.encrypt(&payload);
        let pass = sender.teleport_password().to_string();

        receiver.decode(&s_pkt, &pass).unwrap()
    }

    #[test]
    fn roundtrip_mnemonic_12_words() {
        let m = random_mnemonic(12);
        let original = m.to_string();
        let decoded = roundtrip(Payload::Mnemonic(m));
        match decoded {
            Payload::Mnemonic(m2) => assert_eq!(m2.to_string(), original),
            _ => panic!("expected mnemonic"),
        }
    }

    #[test]
    fn roundtrip_mnemonic_24_words() {
        let m = random_mnemonic(24);
        let original = m.to_string();
        let decoded = roundtrip(Payload::Mnemonic(m));
        match decoded {
            Payload::Mnemonic(m2) => assert_eq!(m2.to_string(), original),
            _ => panic!("expected mnemonic"),
        }
    }

    #[test]
    fn wrong_teleport_password_fails() {
        let receiver = ReceiverSession::generate();
        let r_pkt = receiver.to_packet();

        let sender = SenderSession::new(&r_pkt, receiver.numeric_code()).unwrap();
        let m = random_mnemonic(24);
        let s_pkt = sender.encrypt(&Payload::Mnemonic(m));

        let result = receiver.decode(&s_pkt, "WRONGPAS");
        assert_eq!(result.unwrap_err(), Error::ChecksumMismatch);
    }

    #[test]
    fn bbqr_transport_roundtrip() {
        let receiver = ReceiverSession::generate();
        let r_pkt = receiver.to_packet();

        // simulate transmission via BBQr strings
        let r_bbqr = r_pkt.to_bbqr();
        let r_pkt_parsed = ReceiverPacket::from_bbqr(&r_bbqr).unwrap();

        let sender = SenderSession::new(&r_pkt_parsed, receiver.numeric_code()).unwrap();
        let m = random_mnemonic(24);
        let original = m.to_string();
        let s_pkt = sender.encrypt(&Payload::Mnemonic(m));

        let s_bbqr = s_pkt.to_bbqr();
        let s_pkt_parsed = SenderPacket::from_bbqr(&s_bbqr).unwrap();

        let decoded = receiver.decode(&s_pkt_parsed, sender.teleport_password()).unwrap();
        match decoded {
            Payload::Mnemonic(m2) => assert_eq!(m2.to_string(), original),
            _ => panic!("expected mnemonic"),
        }
    }
}
