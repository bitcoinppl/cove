use std::str::FromStr as _;

use bip39::Mnemonic;
use bitcoin::{
    bip32::{ChildNumber, Xpriv},
    secp256k1::Secp256k1,
};
use cove_keyteleport::{
    DecodedPayload, Error, NumericCode, Packet, Payload, ReceiverSession, SenderSession,
    TeleportPassword, XprvPayload,
};

const RECEIVER_SECRET: [u8; 32] = [1; 32];
const RECEIVER_SECRET_2: [u8; 32] = [2; 32];
const SENDER_SECRET: [u8; 32] = [3; 32];
const PASSWORD_BYTES: [u8; 5] = [0x12, 0x34, 0x56, 0x78, 0x9a];
const XPRV: &str = "xprv9s21ZrQH143K4BwRCYKSEPwcAMYweWkfKLURabnnv2GLNhJN1LSCgDQyGWyNcat72najQKwyshCBXWfHHVbcdxPAZPqByMyWDbWp5SjCfEa";
const KEYTELEPORT_DOC_EXAMPLE: &str =
    "https://keyteleport.com/#B$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO";
const EXPECTED_RECEIVER_PACKET: &str =
    "c6cc594473287ba6a0af8b6a5f5183cf51cb750d1df10c8a6cc5236fe43fc5e5dc";
const EXPECTED_SENDER_PACKET: &str =
    "02531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe3378159627a12c2";

// Fixtures generated from COLDCARD firmware testing/teleport_protocol.py at
// bcc2c382a324690a2fcf972c0bac3b79bf923f7b

#[test]
fn mnemonic_payload_roundtrips_for_supported_word_counts() {
    for entropy_len in [16, 24, 32] {
        let entropy = vec![0_u8; entropy_len];
        let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();
        let decoded = roundtrip(Payload::mnemonic(mnemonic.clone()).unwrap()).unwrap();

        assert_eq!(decoded, DecodedPayload::Mnemonic(mnemonic));
    }
}

#[test]
fn mnemonic_payload_rejects_word_counts_coldcard_cannot_encode() {
    for entropy_len in [20, 28] {
        let mnemonic = Mnemonic::from_entropy(&vec![0_u8; entropy_len]).unwrap();

        assert!(matches!(
            Payload::mnemonic(mnemonic),
            Err(Error::UnsupportedMnemonicWordCount(15 | 21))
        ));
    }
}

#[test]
fn xprv_protocol_payload_roundtrips_and_validates_base58check() {
    let decoded = roundtrip(Payload::xprv(XPRV).unwrap()).unwrap();

    match decoded {
        DecodedPayload::Xprv(xprv) => assert_eq!(xprv.expose_string(), XPRV),
        DecodedPayload::Mnemonic(_) | DecodedPayload::Notes(_) => panic!("expected xprv"),
    }

    assert!(matches!(
        XprvPayload::parse("xprv9s21ZrQH143K4invalid"),
        Err(Error::InvalidXprvPayload)
    ));

    let child = Xpriv::from_str(XPRV)
        .unwrap()
        .derive_priv(&Secp256k1::new(), &[ChildNumber::from_normal_idx(0).unwrap()])
        .unwrap();
    assert!(matches!(Payload::xprv(child.to_string()), Err(Error::NonMasterXprvPayload)));
}

#[test]
fn wrong_password_fails_inner_checksum_after_outer_decrypt_succeeds() {
    let receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
    let request = receiver.request().unwrap();
    let sender = SenderSession::with_private_key_and_password(
        &request.packet,
        &request.numeric_code,
        SENDER_SECRET,
        TeleportPassword::from_bytes(PASSWORD_BYTES),
    )
    .unwrap();
    let response = sender.send(Payload::mnemonic(test_mnemonic_12()).unwrap()).unwrap();
    let pending = receiver.decode_step1(&response.packet).unwrap();
    let wrong_password = TeleportPassword::from_bytes([9, 9, 9, 9, 9]);

    assert!(matches!(pending.complete(&wrong_password), Err(Error::Checksum)));
}

#[test]
fn wrong_receiver_key_fails_outer_checksum_without_consuming_packet() {
    let sender_receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
    let request = sender_receiver.request().unwrap();
    let sender = SenderSession::with_private_key_and_password(
        &request.packet,
        &request.numeric_code,
        SENDER_SECRET,
        TeleportPassword::from_bytes(PASSWORD_BYTES),
    )
    .unwrap();
    let response = sender.send(Payload::mnemonic(test_mnemonic_12()).unwrap()).unwrap();
    let wrong_receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET_2).unwrap();

    assert!(matches!(wrong_receiver.decode_step1(&response.packet), Err(Error::Checksum)));
}

#[test]
fn mistyped_but_curve_valid_receiver_code_fails_at_receiver_checksum() {
    let receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
    let request = receiver.request().unwrap();
    let wrong_code = (0..100_000_000)
        .map(|value| NumericCode::from_str(&format!("{value:08}")).unwrap())
        .find(|code| {
            code != &request.numeric_code && SenderSession::new(&request.packet, code).is_ok()
        })
        .expect("a curve-valid mistyped code should be found");
    let sender = SenderSession::with_private_key_and_password(
        &request.packet,
        &wrong_code,
        SENDER_SECRET,
        TeleportPassword::from_bytes(PASSWORD_BYTES),
    )
    .unwrap();
    let response = sender.send(Payload::mnemonic(test_mnemonic_12()).unwrap()).unwrap();

    assert!(matches!(receiver.decode_step1(&response.packet), Err(Error::Checksum)));
}

#[test]
fn coldcard_protocol_vectors_match() {
    let receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
    let request = receiver.request().unwrap();
    let sender = SenderSession::with_private_key_and_password(
        &request.packet,
        &request.numeric_code,
        SENDER_SECRET,
        TeleportPassword::from_bytes(PASSWORD_BYTES),
    )
    .unwrap();
    let response = sender.send(Payload::mnemonic(test_mnemonic_12()).unwrap()).unwrap();

    assert_eq!(request.numeric_code.as_str(), "88805930");
    assert_eq!(hex_string(request.packet.as_bytes()), EXPECTED_RECEIVER_PACKET);
    assert_eq!(response.password.as_display_text(), "CI2FM6E2");
    assert_eq!(hex_string(response.packet.as_bytes()), EXPECTED_SENDER_PACKET);
    assert!(request.packet.to_bbqr_part().unwrap().starts_with("B$2R0100"));
    assert!(response.packet.to_bbqr_part().unwrap().starts_with("B$2S0100"));
}

#[test]
fn url_parse_build_handles_case_and_rejects_invalid_fragments() {
    let packet = Packet::from_url(KEYTELEPORT_DOC_EXAMPLE).unwrap();

    match &packet {
        Packet::Receiver(receiver) => assert_eq!(receiver.as_bytes().len(), 33),
        _ => panic!("expected receiver packet"),
    }

    let rebuilt = packet.to_url().unwrap();
    assert!(rebuilt.starts_with("https://keyteleport.com/#B$2R0100"));

    let mixed_case_url = KEYTELEPORT_DOC_EXAMPLE.replace("keyteleport.com", "KeyTeleport.com");
    assert!(Packet::from_url(&mixed_case_url).is_ok());
    let raw_fragment = KEYTELEPORT_DOC_EXAMPLE.split_once('#').unwrap().1;
    assert!(Packet::from_url(raw_fragment).is_ok());
    assert!(Packet::from_url("https://keyteleport.com/#not-bbqr").is_err());
    assert!(Packet::from_url("https://example.com/#B$2R0100").is_err());
}

#[test]
fn password_parsing_is_case_insensitive_and_groups_for_display() {
    let password = TeleportPassword::from_bytes(PASSWORD_BYTES);
    let display = password.as_display_text();
    let lowercase = display.to_ascii_lowercase();

    assert_eq!(TeleportPassword::from_str(&lowercase).unwrap(), password);
    assert_eq!(password.grouped(), "CI 2F M6 E2");
}

#[test]
fn receiver_code_groups_for_display() {
    let code = NumericCode::from_str("12345678").unwrap();

    assert_eq!(code.grouped(), "12 34 56 78");
}

fn roundtrip(payload: Payload) -> Result<DecodedPayload, Error> {
    let receiver = ReceiverSession::from_private_key_bytes(RECEIVER_SECRET).unwrap();
    let request = receiver.request().unwrap();
    let password = TeleportPassword::from_bytes(PASSWORD_BYTES);
    let sender = SenderSession::with_private_key_and_password(
        &request.packet,
        &request.numeric_code,
        SENDER_SECRET,
        password.clone(),
    )
    .unwrap();
    let response = sender.send(payload).unwrap();

    receiver.decode(&response.packet, &password)
}

fn test_mnemonic_12() -> Mnemonic {
    Mnemonic::from_entropy(&[0_u8; 16]).unwrap()
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
