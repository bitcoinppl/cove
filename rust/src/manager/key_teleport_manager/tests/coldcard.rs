//! COLDCARD-format KeyTeleport payloads received by Cove
//!
//! The keyteleport crate only sends mnemonics and xprvs, so these tests encrypt raw COLDCARD
//! cleartext with [`ColdcardSender`], a port of `encode_payload` in COLDCARD firmware
//! `shared/teleport.py` (Coldcard/firmware a772156d). Payload layouts follow the same file:
//! `kt_do_send` prefixes the JSON or raw body with a one-letter type code, and
//! `kt_accept_values` lists the codes

use aes::Aes256;
use bbqr::{
    encode::Encoding,
    file_type::FileType,
    qr::Version,
    split::{Split, SplitOptions},
};
use bdk_wallet::bitcoin::{
    NetworkKind,
    bip32::{ChainCode, ChildNumber, Fingerprint as Bip32Fingerprint, Xpriv},
    secp256k1::{self, PublicKey, Secp256k1, SecretKey, ecdh::shared_secret_point},
};
use cove_device::keychain::WalletXprv;
use ctr::cipher::{KeyIvInit as _, StreamCipher as _};
use keyteleport::{SenderPacket, TeleportPassword};
use sha2::{Digest as _, Sha256, Sha512};

use crate::{
    multi_format::StringOrData,
    qr_scanner::{MultiQrError, QrScannerFFI, ScanResult},
};

use super::{
    support::{
        ABANDON_ABOUT, ExpectedWallet, WalletTargetGuard, all_wallet_ids, deliver_response,
        import_new_wallet, start_receiver,
    },
    *,
};

type Aes256Ctr = ctr::Ctr128BE<Aes256>;

/// A KeyTeleport sender that encrypts arbitrary cleartext the way COLDCARD does
struct ColdcardSender {
    private_key: SecretKey,
    password: [u8; 5],
}

impl ColdcardSender {
    fn new(private_key: [u8; 32], password: [u8; 5]) -> Self {
        Self { private_key: SecretKey::from_slice(&private_key).unwrap(), password }
    }

    /// The Teleport Password as COLDCARD shows it to the sender
    fn password_text(&self) -> String {
        TeleportPassword::from_bytes(self.password).as_display_text()
    }

    /// Encrypts `cleartext` for the receiver request, as `kt_do_send` does
    fn sender_packet(&self, request: &KeyTeleportReceiveState, cleartext: &[u8]) -> Vec<u8> {
        let receiver_key =
            decrypt_receiver_pubkey(&request.numeric_code, request.packet.as_bytes());

        let mut packet = self.private_key.public_key(&Secp256k1::new()).serialize().to_vec();
        packet.extend(self.encrypt(&receiver_key, cleartext));

        packet
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

    fn sender_response(
        &self,
        request: &KeyTeleportReceiveState,
        cleartext: &[u8],
    ) -> KeyTeleportSenderPacket {
        SenderPacket::new(self.sender_packet(request, cleartext)).unwrap().into()
    }
}

fn encrypt_checked(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut ciphertext = plaintext.to_vec();
    Aes256Ctr::new(key.into(), &[0_u8; 16].into()).apply_keystream(&mut ciphertext);
    ciphertext.extend_from_slice(&Sha256::digest(plaintext)[30..]);

    ciphertext
}

/// Recovers the receiver's public key from its request, as `decrypt_rx_pubkey` does
fn decrypt_receiver_pubkey(code: &str, packet: &[u8]) -> PublicKey {
    let key: [u8; 32] = Sha256::digest(code.as_bytes()).into();
    let mut public_key = packet.to_vec();
    Aes256Ctr::new(&key.into(), &[0_u8; 16].into()).apply_keystream(&mut public_key);
    public_key[0] = 0x02 | (public_key[0] & 0x01);

    PublicKey::from_slice(&public_key).unwrap()
}

/// A COLDCARD secret stash with trailing zeros trimmed, as `share_master_secret` sends it
fn stash_cleartext(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut cleartext = vec![b's', marker];
    cleartext.extend_from_slice(body);
    while cleartext.last() == Some(&0) {
        cleartext.pop();
    }

    cleartext
}

/// Sends `cleartext` from a COLDCARD to a new receive request and enters the password
fn receive_from_coldcard(
    cleartext: &[u8],
) -> (Arc<RustKeyTeleportManager>, Result<(), KeyTeleportAlert>) {
    let (receiver, request) = start_receiver();
    let coldcard = ColdcardSender::new([0x4b; 32], [0x0c, 0x01, 0xdc, 0xa2, 0xd5]);
    let response = coldcard.sender_response(&request, cleartext);
    let result = deliver_response(&receiver, &response, &coldcard.password_text());

    (receiver, result)
}

#[test]
fn coldcard_sender_matches_the_firmware_protocol_vector() {
    // vector from keyteleport-0.1.0 src/sender.rs, generated with COLDCARD firmware
    // testing/teleport_protocol.py at bcc2c382
    let receiver = keyteleport::ReceiverSession::from_private_key_bytes([1; 32]).unwrap();
    let request = receiver.request().unwrap();
    let request = KeyTeleportReceiveState {
        packet: Arc::new(request.packet.into()),
        numeric_code: request.numeric_code.as_str().to_string(),
        grouped_numeric_code: request.numeric_code.grouped(),
        created_at_secs: 0,
        network: Network::Signet,
        wallet_mode: WalletMode::Main,
    };

    let coldcard = ColdcardSender::new([3; 32], [0x12, 0x34, 0x56, 0x78, 0x9a]);

    let packet = coldcard.sender_packet(&request, &stash_cleartext(0x80, &[0; 16]));

    assert_eq!(coldcard.password_text(), "CI2FM6E2");
    assert_eq!(
        hex::encode(packet),
        "02531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe3378159627a12c2"
    );
}

#[test]
fn coldcard_mnemonic_stashes_import_with_the_expected_fingerprint_and_first_address() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    // the marker's low bits give the entropy length in 8-byte units minus two; the all-zero
    // 24-word entropy trims down to the marker alone
    for (marker, entropy) in [(0x80, vec![0x1f; 16]), (0x81, vec![0x62; 24]), (0x82, vec![0; 32])] {
        let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();
        let expected = ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet);

        let (receiver, result) = receive_from_coldcard(&stash_cleartext(marker, &entropy));

        result.unwrap();
        assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());
        expected.assert_imported(&import_new_wallet(&receiver));
    }
}

#[test]
fn coldcard_private_key_and_raw_master_secret_stashes_import_as_xprv_wallets() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    // marker 0x01 carries a master xprv as chain code then private key; a final zero byte is
    // trimmed in transit and restored by the 72-byte stash padding
    let chain_code = [0x2e; 32];
    let mut private_key = [0x91; 32];
    private_key[31] = 0;
    let master = Xpriv {
        network: NetworkKind::Main,
        depth: 0,
        parent_fingerprint: Bip32Fingerprint::default(),
        child_number: ChildNumber::Normal { index: 0 },
        private_key: secp256k1::SecretKey::from_slice(&private_key).unwrap(),
        chain_code: ChainCode::from(chain_code),
    };

    let xprv_stash = stash_cleartext(0x01, &[chain_code, private_key].concat());

    // markers 0x10..=0x40 carry a raw BIP32 seed of that many bytes, sent by COLDCARD as
    // "Master Seed Bytes"
    let seed_32 = [0x6a; 32];
    let seed_64 = [0x0d; 64];

    for (cleartext, expected_xprv) in [
        (xprv_stash, master),
        (stash_cleartext(0x20, &seed_32), Xpriv::new_master(NetworkKind::Main, &seed_32).unwrap()),
        (stash_cleartext(0x40, &seed_64), Xpriv::new_master(NetworkKind::Main, &seed_64).unwrap()),
    ] {
        let expected = ExpectedWallet::from_xprv(expected_xprv, Network::Signet);

        let (receiver, result) = receive_from_coldcard(&cleartext);

        result.unwrap();
        assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveXprvReview(_)));
        let wallet = import_new_wallet(&receiver);
        let stored = Keychain::global().get_wallet_secret(&wallet.id).unwrap().unwrap();
        assert_eq!(stored.as_xprv().map(WalletXprv::to_xpriv), Some(expected_xprv));
        expected.assert_imported(&wallet);
    }
}

#[test]
fn coldcard_note_is_shown_as_a_message_without_a_wallet_import() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let wallets_before = all_wallet_ids();

    // the record shape `quick_note` sends, followed by a password record from `picked_note`
    let cleartext = br#"n[{"title": "Quick Note", "misc": "Attack at dawn."}, {"title": "Exchange", "user": "satoshi", "password": "hunter2", "site": "example.com", "misc": ""}]"#;
    let (receiver, result) = receive_from_coldcard(cleartext);

    result.unwrap();
    let KeyTeleportManagerState::ReceiveMessageReview(review) = receiver.state() else {
        panic!("expected a message review, got {:?}", receiver.state())
    };

    assert_eq!(
        review.items,
        vec![
            KeyTeleportMessageItem::Note {
                title: "Quick Note".into(),
                text: "Attack at dawn.".into(),
                group: String::new(),
            },
            KeyTeleportMessageItem::Password {
                title: "Exchange".into(),
                username: "satoshi".into(),
                password: "hunter2".into(),
                site: "example.com".into(),
                notes: String::new(),
                group: String::new(),
            },
        ]
    );

    assert!(receiver.reveal_mnemonic_words().is_empty());
    assert_eq!(receiver.reveal_xprv(), None);

    assert_eq!(
        receiver.handle_action(Action::ImportReceivedWallet),
        Err(KeyTeleportAlert::NoPendingReceiveSecret)
    );

    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMessageReview(_)));
    assert_eq!(all_wallet_ids(), wallets_before);

    receiver.handle_action(Action::FinishReview).unwrap();
    assert_eq!(receiver.state(), KeyTeleportManagerState::Idle);
    assert_eq!(all_wallet_ids(), wallets_before);
}

#[test]
fn coldcard_vault_and_backup_payloads_are_rejected_without_importing_their_secrets() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let wallets_before = all_wallet_ids();

    let vault_entropy = [0x1f_u8; 16];
    let vault_mnemonic = Mnemonic::from_entropy(&vault_entropy).unwrap();
    let vault_master = Xpriv::new_master(NetworkKind::Main, &vault_mnemonic.to_seed("")).unwrap();
    let vault_xfp = u32::from_le_bytes(vault_master.fingerprint(&Secp256k1::new()).to_bytes());

    // `pick_vault_submenu` sends one `VaultEntry(xfp, encoded, label, origin)` from
    // shared/seed.py as a JSON list: the little-endian fingerprint and the hex stash
    let vault =
        format!("v[{vault_xfp}, \"80{}\", \"Spending\", \"Imported\"]", hex::encode(vault_entropy));

    let backup_mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();
    let backup_master = Xpriv::new_master(NetworkKind::Test, &backup_mnemonic.to_seed("")).unwrap();
    let backup_xpub =
        bdk_wallet::bitcoin::bip32::Xpub::from_priv(&Secp256k1::new(), &backup_master);

    // `share_full_backup` sends `render_backup_contents` from shared/backups.py with comment
    // and blank lines removed, so the mnemonic and xprv travel in clear text; `raw_secret` is
    // the hex stash with trailing zeros stripped, which leaves only the marker's first digit
    let backup = format!(
        "bmnemonic = \"{ABANDON_ABOUT}\"\nchain = \"XTN\"\nxprv = \"{backup_master}\"\n\
         xpub = \"{backup_xpub}\"\nraw_secret = \"8\"\nfw_date = \"2025-02-19\"\n\
         fw_version = \"1.3.1Q\"\nhardware = \"Q\""
    );

    for (kind, cleartext) in [("vault", vault), ("backup", backup)] {
        let (receiver, result) = receive_from_coldcard(cleartext.as_bytes());

        assert_eq!(result, Err(KeyTeleportAlert::UnsupportedPayload), "{kind}");
        assert!(
            matches!(receiver.state(), KeyTeleportManagerState::ReceiveEnterPassword),
            "{kind} left the receiver in {:?}",
            receiver.state()
        );

        assert_eq!(
            receiver.handle_action(Action::ImportReceivedWallet),
            Err(KeyTeleportAlert::NoPendingReceiveSecret),
            "{kind}"
        );

        assert_eq!(all_wallet_ids(), wallets_before, "{kind} created a wallet");
    }
}

/// A COLDCARD multisig PSBT teleport: a 4-byte nonce, then the doubly encrypted `p` payload
/// without a sender public key, as `kt_send_psbt` passes `prefix=ri` to `kt_do_send`
fn coldcard_psbt_packet() -> Vec<u8> {
    // a 2-input, 2-output PSBT, the same one the QR scanner tests use
    let psbt = hex::decode("70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000").unwrap();
    let cosigner_key = SecretKey::from_slice(&[0x77; 32]).unwrap().public_key(&Secp256k1::new());
    let coldcard = ColdcardSender::new([0x4b; 32], [0x0c, 0x01, 0xdc, 0xa2, 0xd5]);

    let mut packet = 0x0123_4567_u32.to_be_bytes().to_vec();
    packet.extend(coldcard.encrypt(&cosigner_key, &[b"p".as_slice(), &psbt].concat()));

    packet
}

fn psbt_bbqr_parts(packet: &[u8], part_count: usize) -> Vec<String> {
    let options = SplitOptions {
        encoding: Encoding::Base32,
        min_split_number: part_count,
        max_split_number: part_count,
        min_version: Version::V01,
        max_version: Version::V40,
    };

    Split::try_from_data(packet, FileType::KeyTeleportPsbt, options).unwrap().parts
}

#[test]
fn coldcard_psbt_teleport_is_rejected_by_the_scanner_and_the_receive_flow() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let packet = coldcard_psbt_packet();
    let wallets_before = all_wallet_ids();

    // COLDCARD animates a PSBT this size over several QR frames
    let scanner = QrScannerFFI::new();
    let parts = psbt_bbqr_parts(&packet, 3);
    let results = parts
        .iter()
        .map(|part| scanner.scan(StringOrData::String(part.clone())))
        .collect::<Vec<_>>();

    assert!(results[..2].iter().all(|result| matches!(result, Ok(ScanResult::InProgress { .. }))));
    assert!(
        matches!(&results[2], Err(MultiQrError::ParseError(message)) if message.contains("PSBT")),
        "completed PSBT teleport scanned as {:?}",
        results[2]
    );

    // pasted or opened as one frame into an active receive request
    let (receiver, request) = start_receiver();
    let single_frame = psbt_bbqr_parts(&packet, 1).remove(0);
    let error = receiver.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(
        StringOrData::String(single_frame),
    )));

    assert_eq!(error, Err(KeyTeleportAlert::UnsupportedPsbt));
    assert_eq!(receive_request(&receiver), request, "the receive request must stay usable");
    assert_eq!(all_wallet_ids(), wallets_before);
}
