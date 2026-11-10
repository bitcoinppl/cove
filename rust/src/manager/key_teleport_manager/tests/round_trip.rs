//! Transfers between two Cove phones, run through both managers end to end
//!
//! iOS and Android run the same Rust send and receive flows, so one round trip here stands in for
//! either direction between the platforms

use bdk_wallet::{
    KeychainKind,
    bitcoin::{NetworkKind, bip32::Xpriv},
    template::{Bip84, DescriptorTemplate as _},
};
use cove_device::keychain::WalletXprv;

use crate::{
    multi_format::MultiFormat,
    test_support::{WALLET_MNEMONIC_KEY_SUFFIX, shared_mock_keychain},
    wallet::{Wallet, metadata::WalletType},
};

use super::{
    support::{
        ABANDON_ABOUT, ExpectedWallet, WalletTargetGuard, all_wallet_ids, deliver_response,
        every_scope, import_new_wallet, select_target, send_from_cove, start_receiver,
        try_send_from_cove, wallets_in,
    },
    *,
};

/// Sends a mnemonic wallet from one Cove manager to another and checks the imported wallet
fn assert_mnemonic_transfer_imports_the_sender_wallet(mnemonic: Mnemonic) {
    let expected = ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet);
    let word_count = mnemonic.word_count();
    let (receiver, request) = start_receiver();
    let (packet, password) = send_from_cove(WalletSecret::Mnemonic(mnemonic.clone()), &request);

    deliver_response(&receiver, &packet, &password).unwrap();

    let KeyTeleportManagerState::ReceiveMnemonicReview(review) = receiver.state() else {
        panic!("expected mnemonic review, got {:?}", receiver.state())
    };

    assert_eq!(review.word_count as usize, word_count);
    assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());

    let wallet = import_new_wallet(&receiver);
    expected.assert_imported(&wallet);
}

#[test]
fn twelve_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();

    // BIP84's published m/84'/1'/0'/0/0 vector pins the independent derivation itself
    let expected = ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet);
    assert_eq!(expected.first_address, "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl");

    assert_mnemonic_transfer_imports_the_sender_wallet(mnemonic);
}

#[test]
fn eighteen_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    assert_mnemonic_transfer_imports_the_sender_wallet(
        Mnemonic::from_entropy(&[0x5c; 24]).unwrap(),
    );
}

#[test]
fn twenty_four_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    assert_mnemonic_transfer_imports_the_sender_wallet(
        Mnemonic::from_entropy(&[0xa7; 32]).unwrap(),
    );
}

#[test]
fn master_private_key_transfer_imports_an_xprv_wallet_with_the_same_first_address() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    // a Signet wallet holds its key as a tprv, KeyTeleport carries it as an xprv because the
    // `keyteleport` crate accepts only that form, and the receiver stores the tprv again
    let tprv = Xpriv::new_master(NetworkKind::Test, &[0x3d; 32]).unwrap();
    let xprv = Xpriv { network: NetworkKind::Main, ..tprv };
    let expected = ExpectedWallet::from_xprv(tprv, Network::Signet);
    let (receiver, request) = start_receiver();
    let secret = WalletSecret::Xpriv(WalletXprv::try_from(tprv).unwrap());
    let (packet, password) = send_from_cove(secret, &request);

    deliver_response(&receiver, &packet, &password).unwrap();

    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveXprvReview(_)));
    assert_eq!(receiver.reveal_xprv().as_deref(), Some(xprv.to_string().as_str()));

    let wallet = import_new_wallet(&receiver);
    let stored = Keychain::global().get_wallet_secret(&wallet.id).unwrap().unwrap();
    assert_eq!(stored.as_xprv().map(WalletXprv::to_xpriv), Some(tprv));
    expected.assert_imported(&wallet);
}

#[test]
fn teleporting_the_same_wallet_twice_offers_the_existing_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let imported = import_new_wallet(&receiver);

    // 73c5da0a is the master fingerprint of the BIP84 test mnemonic the sender used
    assert_eq!(imported.name, "KeyTeleport 73C5DA0A");
    let wallets_after_first_import = all_wallet_ids();

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    receiver.handle_action(Action::ImportReceivedWallet).unwrap();

    let KeyTeleportManagerState::ReceiveAlreadyImportedWallet(existing) = receiver.state() else {
        panic!("expected the existing wallet, got {:?}", receiver.state())
    };

    assert_eq!(existing.id, imported.id);
    assert_eq!(existing.name, imported.name);
    assert_eq!(all_wallet_ids(), wallets_after_first_import);
    assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
}

#[test]
fn received_words_of_a_watch_only_wallet_upgrade_it_in_place() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    // the receiving phone already watches the account the words belong to, imported from a
    // hardware wallet's export
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();
    let watched = import_hardware_export(&mnemonic, Network::Signet);
    assert_ne!(watched.wallet_type, WalletType::Hot);
    let keychain = Keychain::global();
    assert!(keychain.get_wallet_secret(&watched.id).unwrap().is_none());
    let descriptors = keychain.get_public_descriptor(&watched.id).unwrap();
    let xpub = keychain.get_wallet_xpub(&watched.id).unwrap();
    let wallets_before = all_wallet_ids();

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    receiver.handle_action(Action::ImportReceivedWallet).unwrap();

    let KeyTeleportManagerState::ReceiveImportedWallet(upgraded) = receiver.state() else {
        panic!("expected the upgraded wallet, got {:?}", receiver.state())
    };

    assert_eq!(upgraded.id, watched.id, "the words imported as a second wallet");
    assert_eq!(all_wallet_ids(), wallets_before);
    assert_eq!(upgraded.name, watched.name);
    assert_eq!(upgraded.origin, watched.origin);
    assert!(upgraded.verified, "received words count as verified");
    assert_eq!(upgraded.hardware_metadata, None);
    assert_eq!(Database::global().global_config.selected_wallet(), Some(watched.id.clone()));

    // the stored row, not just the state the review shows, is the signing wallet
    let stored = Database::global()
        .wallets
        .get(&watched.id, Network::Signet, WalletMode::Main)
        .unwrap()
        .unwrap();

    assert_eq!(stored, upgraded);
    assert_eq!(keychain.get_public_descriptor(&watched.id).unwrap(), descriptors);
    assert_eq!(keychain.get_wallet_xpub(&watched.id).unwrap(), xpub);
    let secret = keychain.get_wallet_secret(&watched.id).unwrap().unwrap();
    assert_eq!(secret.as_mnemonic(), Some(&mnemonic));
    ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet).assert_imported(&stored);
    assert!(keychain.get_key_teleport_receive_session().unwrap().is_none());
}

/// Imports the mnemonic's BIP84 account as a hardware wallet's descriptor export, the way a
/// scanned export arrives, and returns the watch-only wallet it creates
fn import_hardware_export(mnemonic: &Mnemonic, network: Network) -> WalletMetadata {
    let bitcoin_network = bdk_wallet::bitcoin::Network::from(network);
    let xprv = Xpriv::new_master(bitcoin_network, &mnemonic.to_seed("")).unwrap();
    let public_descriptor = |keychain| {
        let (descriptor, _, _) = Bip84(xprv, keychain).build(xprv.network).unwrap();
        descriptor.to_string()
    };

    let export = format!(
        "{}\n{}",
        public_descriptor(KeychainKind::External),
        public_descriptor(KeychainKind::Internal)
    );

    let format = pubport::Format::try_new_from_str(&export).unwrap();
    Wallet::try_new_persisted_from_pubport(format).unwrap().metadata
}

#[test]
fn received_wallet_lands_only_in_the_request_scope() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Decoy);

    let (receiver, request) = start_receiver();
    assert_eq!((request.network, request.wallet_mode), (Network::Signet, WalletMode::Decoy));
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let wallet = import_new_wallet(&receiver);

    assert_eq!((wallet.network, wallet.wallet_mode), (Network::Signet, WalletMode::Decoy));
    for (network, mode) in every_scope() {
        let present = wallets_in(network, mode).iter().any(|stored| stored.id == wallet.id);
        let is_request_scope = (network, mode) == (Network::Signet, WalletMode::Decoy);
        assert_eq!(present, is_request_scope, "wallet presence in {network:?} {mode:?}");
    }
}

#[test]
fn receive_import_refuses_a_changed_wallet_mode() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let wallets_before = all_wallet_ids();
    select_target(Network::Signet, WalletMode::Decoy);

    let error = receiver.handle_action(Action::ImportReceivedWallet);

    assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
    assert_eq!(all_wallet_ids(), wallets_before);
}

#[test]
fn response_for_a_replaced_request_is_rejected_and_the_new_request_still_works() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let (receiver, old_request) = start_receiver();
    let (old_packet, old_password) = sender_transfer(&old_request);

    receiver.handle_action(Action::RestartReceive).unwrap();
    let new_request = receive_request(&receiver);
    assert_ne!(new_request.packet, old_request.packet);

    assert!(deliver_response(&receiver, &old_packet, &old_password).is_err());
    assert_eq!(receive_request(&receiver), new_request, "the new request must stay displayed");

    let (packet, password) = sender_transfer(&new_request);
    deliver_response(&receiver, &packet, &password).unwrap();
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
}

#[test]
fn response_for_an_expired_request_is_rejected_and_its_replacement_still_works() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let (receiver, expired_request) = start_receiver();
    let (expired_packet, expired_password) = sender_transfer(&expired_request);

    // the request on screen ages past its 24-hour lifetime before the response arrives
    {
        let mut model = receiver.model.lock();
        let Phase::Receive(ReceivePhase::Ready { session, .. }) = &mut model.phase else {
            panic!("expected a displayed receive request")
        };
        session.created_at_secs -= 24 * 60 * 60 + 1;
    }

    let error = deliver_response(&receiver, &expired_packet, &expired_password);

    assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionExpired));
    let replacement = receive_request(&receiver);
    assert_ne!(replacement.packet, expired_request.packet);
    assert!(deliver_response(&receiver, &expired_packet, &expired_password).is_err());
    assert_eq!(receive_request(&receiver), replacement);

    let (packet, password) = sender_transfer(&replacement);
    deliver_response(&receiver, &packet, &password).unwrap();
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
}

#[test]
fn new_request_changes_the_receiver_code_and_only_the_new_code_completes_a_transfer() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();
    let (receiver, old_request) = start_receiver();

    // both apps dispatch this from their Create New Request button
    receiver.clone().dispatch(Action::RestartReceive);
    let new_request = receive_request(&receiver);

    // two random 8-digit codes match about once in 10^8 runs, which is rare enough to assert
    assert_ne!(new_request.numeric_code, old_request.numeric_code);
    assert_ne!(new_request.packet, old_request.packet);

    // both apps show the grouped code, so it must be the new request's code and not a stale one
    let shown_digits: String =
        new_request.grouped_numeric_code.chars().filter(char::is_ascii_digit).collect();
    assert_eq!(shown_digits, new_request.numeric_code);

    // reopening Receive shows the new request, not the one it replaced
    let (_, reopened_request) = start_receiver();
    assert_eq!(reopened_request, new_request);

    // a sender who copied the old code either has it refused or produces a response the
    // receiver cannot open
    let old_code_transfer = try_send_from_cove(
        WalletSecret::Mnemonic(mnemonic.clone()),
        &new_request.packet,
        &old_request.grouped_numeric_code,
    )
    .and_then(|(packet, password)| deliver_response(&receiver, &packet, &password));

    assert!(old_code_transfer.is_err(), "the replaced code unlocked the new request");
    assert_eq!(receive_request(&receiver), new_request);

    let (packet, password) = send_from_cove(WalletSecret::Mnemonic(mnemonic.clone()), &new_request);
    deliver_response(&receiver, &packet, &password).unwrap();
    assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());
}

#[test]
fn pasted_request_link_or_packet_with_the_shown_code_sends_the_fixed_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();

    // the receiver shares its request as a link and shows it as a BBQr packet, and the sender
    // types the code either as shown or as bare digits
    let link = |request: &KeyTeleportReceiveState| {
        (request.packet.url().unwrap(), request.grouped_numeric_code.clone())
    };
    let packet = |request: &KeyTeleportReceiveState| {
        (request.packet.bbqr_part().unwrap(), request.numeric_code.clone())
    };

    for pasted_form in [link, packet] {
        let (receiver, request) = start_receiver();
        let (pasted, code) = pasted_form(&request);
        let fixture = SendWalletFixture::with_secret(WalletSecret::Mnemonic(mnemonic.clone()));
        let sender = RustKeyTeleportManager::new();
        sender.handle_action(Action::StartSendFromWallet(fixture.wallet.id.clone())).unwrap();

        // both apps parse the pasted text with this call and hand the receiver packet over;
        // clipboard text often carries surrounding whitespace
        let clipboard = StringOrData::String(format!("  {pasted}\n"));
        let MultiFormat::KeyTeleportReceiver(parsed) = MultiFormat::try_from(clipboard).unwrap()
        else {
            panic!("pasted request did not parse as a receiver request")
        };

        sender.handle_action(Action::Ingest(KeyTeleportInput::Receiver(parsed))).unwrap();
        let KeyTeleportManagerState::SendEnterCode(entry) = sender.state() else {
            panic!("expected receiver code entry, got {:?}", sender.state())
        };

        assert_eq!(entry.selected_wallet.id, fixture.wallet.id);

        sender.handle_action(Action::EnterReceiverCode(code)).unwrap();
        let KeyTeleportManagerState::SendReady(ready) = sender.state() else {
            panic!("expected the sender response, got {:?}", sender.state())
        };

        assert_eq!(ready.selected_wallet.id, fixture.wallet.id);
        deliver_response(&receiver, &ready.packet, &ready.password.display_text()).unwrap();
        assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());
        receiver.handle_action(Action::FinishReview).unwrap();
    }
}

#[test]
fn ending_the_session_discards_the_request_so_its_response_no_longer_opens() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    let wallets_before = all_wallet_ids();

    // both apps dispatch this from their End Session button
    receiver.clone().dispatch(Action::EndReceive);

    assert!(matches!(receiver.state(), KeyTeleportManagerState::Idle));
    assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
    assert_eq!(
        deliver_response(&receiver, &packet, &password),
        Err(KeyTeleportAlert::NoActiveReceiveSession)
    );

    // reopening Receive starts a new request instead of resuming the ended one
    let (reopened, new_request) = start_receiver();
    assert_ne!(new_request.packet, request.packet);
    assert!(deliver_response(&reopened, &packet, &password).is_err());
    assert_eq!(receive_request(&reopened), new_request);
    assert_eq!(all_wallet_ids(), wallets_before);
}

#[test]
fn wrong_direction_paste_is_refused_and_both_phones_still_finish_their_transfer() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();

    // the receiving phone is pasted another phone's request
    let (receiver, request) = start_receiver();
    let stray_request = ReceiverSession::new().request().unwrap();
    let stray_request = KeyTeleportReceiverPacket::from(stray_request.packet).url().unwrap();
    let pasted = KeyTeleportInput::MultiFormat(StringOrData::String(stray_request));

    assert_eq!(
        receiver.handle_action(Action::Ingest(pasted)),
        Err(KeyTeleportAlert::ConflictingTransferDirection)
    );
    assert_eq!(receive_request(&receiver), request);

    // the sending phone is pasted a sender response while it waits for a request
    let fixture = SendWalletFixture::with_secret(WalletSecret::Mnemonic(mnemonic.clone()));
    let sender = RustKeyTeleportManager::new();
    sender.handle_action(Action::StartSendFromWallet(fixture.wallet.id.clone())).unwrap();
    let (stray_response, _) = sender_transfer(&request);
    let pasted = StringOrData::String(stray_response.bbqr_part().unwrap());

    assert_eq!(
        sender.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(pasted))),
        Err(KeyTeleportAlert::ConflictingTransferDirection)
    );
    assert!(matches!(sender.state(), KeyTeleportManagerState::SendAwaitReceiver));

    // after the alert both phones carry on with the packets they expect
    let pasted = StringOrData::String(request.packet.url().unwrap());
    sender.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(pasted))).unwrap();
    sender.handle_action(Action::EnterReceiverCode(request.grouped_numeric_code)).unwrap();
    let KeyTeleportManagerState::SendReady(ready) = sender.state() else {
        panic!("expected the sender response, got {:?}", sender.state())
    };

    deliver_response(&receiver, &ready.packet, &ready.password.display_text()).unwrap();
    assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());
}

#[test]
fn fixed_sender_answers_a_request_without_reading_other_wallets() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let (receiver, request) = start_receiver();
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();
    let fixed = SendWalletFixture::with_secret(WalletSecret::Mnemonic(mnemonic.clone()));

    // the sending phone also holds a healthy hot wallet and one whose secret cannot be read
    let other_words = Mnemonic::from_entropy(&[0x42; 16]).unwrap();
    let other = SendWalletFixture::with_secret(WalletSecret::Mnemonic(other_words));
    let mut unreadable = WalletMetadata::preview_new();
    unreadable.network = fixed.wallet.network;
    unreadable.wallet_mode = fixed.wallet.wallet_mode;
    shared_mock_keychain().fail_reads_for(format!("{}{WALLET_MNEMONIC_KEY_SUFFIX}", unreadable.id));
    Database::global()
        .wallets
        .save_all_wallets(
            fixed.wallet.network,
            fixed.wallet.wallet_mode,
            vec![fixed.wallet.clone(), other.wallet.clone(), unreadable.clone()],
        )
        .unwrap();

    shared_mock_keychain().record_reads();
    let sender = RustKeyTeleportManager::new();
    let scanned = StringOrData::String(request.packet.url().unwrap());
    sender.handle_action(Action::StartSendFromWallet(fixed.wallet.id.clone())).unwrap();
    sender.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(scanned))).unwrap();
    let after_scan = sender.state();
    sender.handle_action(Action::EnterReceiverCode(request.grouped_numeric_code)).unwrap();
    let reads = shared_mock_keychain().take_recorded_reads();

    let KeyTeleportManagerState::SendEnterCode(entry) = after_scan else {
        panic!("a fixed sender was asked to choose a wallet: {after_scan:?}")
    };

    assert_eq!(entry.selected_wallet.id, fixed.wallet.id);

    // the send must read the fixed wallet's secret, which shows the recording saw keychain reads
    let fixed_secret = format!("{}{WALLET_MNEMONIC_KEY_SUFFIX}", fixed.wallet.id);
    assert!(reads.contains(&fixed_secret), "no read of the fixed wallet's secret was recorded");

    for wallet_id in [&other.wallet.id, &unreadable.id] {
        let prefix = format!("{wallet_id}::");
        let read = reads.iter().find(|key| key.starts_with(&prefix));
        assert_eq!(read, None, "the fixed send read an unrelated wallet's keychain entry");
    }

    let KeyTeleportManagerState::SendReady(ready) = sender.state() else {
        panic!("expected the sender response, got {:?}", sender.state())
    };

    deliver_response(&receiver, &ready.packet, &ready.password.display_text()).unwrap();
    assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());
}

#[test]
fn main_receive_request_stays_hidden_and_unusable_in_decoy_mode() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let (main_receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    let wallets_before = all_wallet_ids();

    select_target(Network::Signet, WalletMode::Decoy);
    let decoy_receiver = RustKeyTeleportManager::new();

    assert_eq!(
        decoy_receiver.handle_action(Action::StartReceive),
        Err(KeyTeleportAlert::ReceiveSessionScopeChanged)
    );
    assert!(matches!(decoy_receiver.state(), KeyTeleportManagerState::ReceiveError));
    assert_eq!(
        deliver_response(&decoy_receiver, &packet, &password),
        Err(KeyTeleportAlert::NoActiveReceiveSession)
    );

    // the main Receive screen left open behind the mode switch refuses the response too
    assert_eq!(
        deliver_response(&main_receiver, &packet, &password),
        Err(KeyTeleportAlert::ReceiveSessionScopeChanged)
    );
    assert_eq!(all_wallet_ids(), wallets_before);

    // back in main mode the request was kept, so the sender's response still lands there
    select_target(Network::Signet, WalletMode::Main);
    let (resumed, resumed_request) = start_receiver();
    assert_eq!(resumed_request, request);
    deliver_response(&resumed, &packet, &password).unwrap();
    let wallet = import_new_wallet(&resumed);
    assert_eq!((wallet.network, wallet.wallet_mode), (Network::Signet, WalletMode::Main));
}

#[test]
fn expired_request_reports_expiry_when_its_response_is_scanned_after_a_relaunch() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let (_, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);

    // the request outlives its 24-hour lifetime while the app is closed
    let stored = Keychain::global().get_key_teleport_receive_session().unwrap().unwrap();
    let mut persisted: serde_json::Value = serde_json::from_str(&stored).unwrap();
    let created_at_secs = persisted["created_at_secs"].as_u64().unwrap();
    persisted["created_at_secs"] = (created_at_secs - 24 * 60 * 60 - 1).into();
    Keychain::global().save_key_teleport_receive_session(&persisted.to_string()).unwrap();

    // the response is scanned before Receive is open, which hands it straight to a new manager
    let relaunched = RustKeyTeleportManager::new();
    let mut displayed = None;
    relaunched.clone().dispatch(Action::Ingest(KeyTeleportInput::Sender(packet.clone())));
    apply_alert_messages(&relaunched, &mut displayed);

    assert_eq!(displayed, Some(KeyTeleportAlert::ReceiveSessionExpired));
    let replacement = receive_request(&relaunched);
    assert_ne!(replacement.packet, request.packet);
    assert!(deliver_response(&relaunched, &packet, &password).is_err());
    assert_eq!(receive_request(&relaunched), replacement);

    let (packet, password) = sender_transfer(&replacement);
    deliver_response(&relaunched, &packet, &password).unwrap();
    assert!(matches!(relaunched.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
}
