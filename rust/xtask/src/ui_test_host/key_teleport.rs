//! The other phone in a KeyTeleport transfer, for UI tests
//!
//! A UI test reads the app's receiver request from the QR it displays and the
//! code it shows, then asks [`send`] to answer it as a sending phone would. The
//! response comes back as BBQr frames for the QR scanner seam and the password
//! a person would type. [`receive_request`] and [`receive_response`] play a
//! receiving phone for tests where the app sends
//!
//! Mnemonics and xprvs go through the `keyteleport` crate's sender, the one the
//! app itself uses. The crate can't send COLDCARD Secure Notes, so notes are
//! encrypted by [`ColdcardSender`], a port of `encode_payload` in COLDCARD
//! firmware `shared/teleport.py` that the Rust COLDCARD tests also use

use std::{collections::HashMap, str::FromStr};

use aes::Aes256;
use bbqr::{
    encode::Encoding,
    file_type::FileType,
    qr::Version,
    split::{Split, SplitOptions},
};
use bip39::Mnemonic;
use bitcoin::secp256k1::{ecdh::shared_secret_point, PublicKey, Secp256k1, SecretKey};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use ctr::cipher::{KeyIvInit as _, StreamCipher as _};
use keyteleport::{
    DecodedPayload, NotesRecord, NumericCode, Packet, Payload, ReceiverPacket, ReceiverSession,
    SenderPacket, SenderSession, TeleportPassword,
};
use parking_lot::Mutex;
use rand::RngExt as _;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256, Sha512};

type Aes256Ctr = ctr::Ctr128BE<Aes256>;

/// Receiving phones the host is playing, by the session id it handed the test
#[derive(Default)]
pub struct Receivers {
    next_id: Mutex<u64>,
    sessions: Mutex<HashMap<u64, ReceiverSession>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    /// The receiver request as the app shows it: a `B$2R` BBQr part or a
    /// keyteleport.com link
    request: String,
    /// The receiver code, with or without the spaces the app groups it with
    code: String,
    payload: SendPayload,
    /// Split the response into at least this many BBQr frames, as a sender
    /// with a large payload shows it
    #[serde(default = "one")]
    min_parts: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum SendPayload {
    Mnemonic(String),
    Xprv(String),
    /// COLDCARD Secure Notes & Passwords records, sent as COLDCARD's JSON
    Notes(Vec<Value>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiveResponseRequest {
    session: u64,
    /// The sender response as the app shows it: a `B$2S` BBQr part or a
    /// keyteleport.com link
    response: String,
    password: String,
}

fn one() -> usize {
    1
}

/// Answers a receiver request with an encrypted payload, as a sending phone does
pub fn send(request: SendRequest) -> Result<Value> {
    let receiver = match parse_packet(&request.request)? {
        Packet::Receiver(packet) => packet,
        other => {
            return Err(eyre!(
                "Expected a KeyTeleport receiver request, got {}",
                packet_kind(&other)
            ))
        }
    };

    let code = NumericCode::from_str(&request.code).wrap_err("Invalid receiver code")?;
    let (packet, password) = match request.payload {
        SendPayload::Mnemonic(words) => {
            let mnemonic = Mnemonic::parse(&words).wrap_err("Invalid mnemonic")?;
            crate_send(&receiver, &code, Payload::mnemonic(mnemonic)?)?
        }
        SendPayload::Xprv(xprv) => crate_send(&receiver, &code, Payload::xprv(xprv)?)?,
        SendPayload::Notes(records) => {
            let mut cleartext = b"n".to_vec();
            cleartext.extend(serde_json::to_vec(&records)?);

            let sender = ColdcardSender::random();
            (sender.sender_packet(&code, &receiver, &cleartext)?, sender.password())
        }
    };

    let bbqr = packet.to_bbqr_part()?;
    let parts = if request.min_parts > 1 {
        split_sender_packet(&packet, request.min_parts)?
    } else {
        vec![bbqr.clone()]
    };

    Ok(json!({
        "bbqr": bbqr,
        "url": packet.to_url()?,
        "parts": parts,
        "password": password.as_display_text(),
    }))
}

impl Receivers {
    /// Starts a receive session and returns its request, as a receiving phone shows it
    pub fn receive_request(&self) -> Result<Value> {
        let session = ReceiverSession::new();
        let request = session.request()?;

        let id = {
            let mut next_id = self.next_id.lock();
            *next_id += 1;
            *next_id
        };

        self.sessions.lock().insert(id, session);

        Ok(json!({
            "session": id,
            "bbqr": request.packet.to_bbqr_part()?,
            "url": request.packet.to_url()?,
            "code": request.numeric_code.as_str(),
        }))
    }

    /// Decrypts the app's response to a session from [`Self::receive_request`]
    /// and ends the session
    pub fn receive_response(&self, request: ReceiveResponseRequest) -> Result<Value> {
        let session = self
            .sessions
            .lock()
            .remove(&request.session)
            .ok_or_else(|| eyre!("No receive session {}", request.session))?;

        let packet = match parse_packet(&request.response)? {
            Packet::Sender(packet) => packet,
            other => {
                return Err(eyre!(
                    "Expected a KeyTeleport sender response, got {}",
                    packet_kind(&other)
                ))
            }
        };

        let password =
            TeleportPassword::from_str(&request.password).wrap_err("Invalid teleport password")?;

        Ok(match session.decode(&packet, &password)? {
            DecodedPayload::Mnemonic(mnemonic) => {
                json!({ "kind": "mnemonic", "mnemonic": mnemonic.to_string() })
            }
            DecodedPayload::Xprv(xprv) => json!({ "kind": "xprv", "xprv": xprv.expose_string() }),
            DecodedPayload::Notes(notes) => json!({
                "kind": "notes",
                "titles": notes.records().iter().map(NotesRecord::title).collect::<Vec<_>>(),
            }),
        })
    }
}

fn crate_send(
    receiver: &ReceiverPacket,
    code: &NumericCode,
    payload: Payload,
) -> Result<(SenderPacket, TeleportPassword)> {
    let response = SenderSession::new(receiver, code)?.send(payload)?;
    Ok((response.packet, response.password))
}

fn parse_packet(text: &str) -> Result<Packet> {
    Packet::from_url(text.trim()).wrap_err("Not a KeyTeleport packet or link")
}

fn packet_kind(packet: &Packet) -> &'static str {
    match packet {
        Packet::Receiver(_) => "a receiver request",
        Packet::Sender(_) => "a sender response",
        Packet::Psbt(_) => "a PSBT",
    }
}

/// Splits a response into a multi-part BBQr, as COLDCARD shows a large one
fn split_sender_packet(packet: &SenderPacket, min_parts: usize) -> Result<Vec<String>> {
    let split = Split::try_from_data(
        packet.as_bytes(),
        FileType::KeyTeleportSender,
        SplitOptions {
            // KeyTeleport BBQr is always Base32, never compressed
            encoding: Encoding::Base32,
            min_split_number: min_parts,
            max_split_number: 100,
            min_version: Version::V01,
            max_version: Version::V40,
        },
    )
    .map_err(|error| eyre!("BBQr encoding failed: {error}"))?;

    Ok(split.parts)
}

/// A KeyTeleport sender that encrypts arbitrary cleartext the way COLDCARD does
struct ColdcardSender {
    private_key: SecretKey,
    password: [u8; 5],
}

impl ColdcardSender {
    /// A sender with a fresh key and password, as COLDCARD picks them for each teleport
    fn random() -> Self {
        let mut rng = rand::rng();
        loop {
            // a random 32 bytes is almost never outside the curve order, but retry if it is
            if let Ok(private_key) = SecretKey::from_slice(&rng.random::<[u8; 32]>()) {
                return Self { private_key, password: rng.random() };
            }
        }
    }

    fn password(&self) -> TeleportPassword {
        TeleportPassword::from_bytes(self.password)
    }

    /// Encrypts `cleartext` for the receiver request, as `kt_do_send` does
    fn sender_packet(
        &self,
        code: &NumericCode,
        receiver: &ReceiverPacket,
        cleartext: &[u8],
    ) -> Result<SenderPacket> {
        let receiver_key = decrypt_receiver_pubkey(code, receiver)?;

        let mut packet = self.private_key.public_key(&Secp256k1::new()).serialize().to_vec();
        packet.extend(self.encrypt(&receiver_key, cleartext));

        Ok(SenderPacket::new(packet)?)
    }

    /// Two AES-256-CTR layers, each followed by the last two bytes of SHA-256 of its plaintext
    fn encrypt(&self, receiver_key: &PublicKey, cleartext: &[u8]) -> Vec<u8> {
        let session_key: [u8; 32] =
            Sha256::digest(shared_secret_point(receiver_key, &self.private_key)).into();

        let mut password_key = [0_u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha512>(&session_key, &self.password, 5000, &mut password_key);

        let inner = encrypt_checked(&password_key, cleartext);
        encrypt_checked(&session_key, &inner)
    }
}

fn encrypt_checked(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut ciphertext = plaintext.to_vec();

    // the protocol fixes an all-zero counter, which is safe because each key belongs to one session
    Aes256Ctr::new(key.into(), &[0_u8; 16].into()).apply_keystream(&mut ciphertext);
    ciphertext.extend_from_slice(&Sha256::digest(plaintext)[30..]);

    ciphertext
}

/// Recovers the receiver's public key from its request, as `decrypt_rx_pubkey` does
fn decrypt_receiver_pubkey(code: &NumericCode, receiver: &ReceiverPacket) -> Result<PublicKey> {
    let key: [u8; 32] = Sha256::digest(code.as_str().as_bytes()).into();
    let mut public_key = receiver.as_bytes().to_vec();
    Aes256Ctr::new(&key.into(), &[0_u8; 16].into()).apply_keystream(&mut public_key);
    public_key[0] = 0x02 | (public_key[0] & 0x01);

    PublicKey::from_slice(&public_key).wrap_err("The receiver code doesn't open this request")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    /// A receiver from the crate, standing in for the app, gets back what [`send`] sent
    fn round_trip(payload: Value, min_parts: usize) -> (Value, DecodedPayload) {
        let receiver = ReceiverSession::new();
        let request = receiver.request().unwrap();

        let response = send(
            serde_json::from_value(json!({
                "request": request.packet.to_url().unwrap(),
                "code": request.numeric_code.grouped(),
                "payload": payload,
                "min_parts": min_parts,
            }))
            .unwrap(),
        )
        .unwrap();

        let Packet::Sender(packet) = Packet::from_url(response["bbqr"].as_str().unwrap()).unwrap()
        else {
            panic!("expected a sender packet")
        };

        let password = TeleportPassword::from_str(response["password"].as_str().unwrap()).unwrap();
        let decoded = receiver.decode(&packet, &password).unwrap();

        (response, decoded)
    }

    #[test]
    fn mnemonic_round_trips_through_a_multi_part_response() {
        let (response, decoded) = round_trip(json!({ "mnemonic": WORDS }), 3);

        let DecodedPayload::Mnemonic(mnemonic) = decoded else { panic!("expected a mnemonic") };
        assert_eq!(mnemonic.to_string(), WORDS);

        let parts: Vec<String> = serde_json::from_value(response["parts"].clone()).unwrap();
        assert!(parts.len() >= 3, "expected at least 3 parts, got {}", parts.len());

        let joined = bbqr::join::Joined::try_from_parts(parts).unwrap();
        assert_eq!(joined.file_type, FileType::KeyTeleportSender);
        assert_eq!(joined.encoding, Encoding::Base32);
    }

    #[test]
    fn notes_round_trip_through_the_coldcard_sender() {
        let notes = json!({ "notes": [{ "title": "Quick Note", "misc": "Attack at dawn." }] });
        let (_, decoded) = round_trip(notes, 1);

        let DecodedPayload::Notes(notes) = decoded else { panic!("expected notes") };
        assert_eq!(
            notes.records().iter().map(NotesRecord::title).collect::<Vec<_>>(),
            ["Quick Note"]
        );
    }

    #[test]
    fn host_receiver_decodes_a_crate_sender_response() {
        let receivers = Receivers::default();
        let request = receivers.receive_request().unwrap();

        let Packet::Receiver(packet) = Packet::from_url(request["url"].as_str().unwrap()).unwrap()
        else {
            panic!("expected a receiver packet")
        };

        let code = NumericCode::from_str(request["code"].as_str().unwrap()).unwrap();
        let payload = Payload::mnemonic(Mnemonic::parse(WORDS).unwrap()).unwrap();
        let (response, password) = crate_send(&packet, &code, payload).unwrap();

        let decoded = receivers
            .receive_response(
                serde_json::from_value(json!({
                    "session": request["session"],
                    "response": response.to_url().unwrap(),
                    "password": password.grouped(),
                }))
                .unwrap(),
            )
            .unwrap();

        assert_eq!(decoded, json!({ "kind": "mnemonic", "mnemonic": WORDS }));
    }
}
